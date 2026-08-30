/* Ported from js/gl/compositor.js — behavior-preserving. */
import type { PMRegistry } from '../registry';

/**
 * Which uniform an effect/transition param binds to. Kernel-generated shaders
 * declare `u_<param>`; legacy raw shaders declare the positional `u_p<i>` (or
 * `u_c<i>` for colors). Named wins when the linked program actually has it.
 */
export function paramUniformName(pd: { k: string; type?: string }, index: number, hasNamed: boolean): string {
  if (hasNamed) return 'u_' + pd.k;
  return (pd.type === 'color' ? 'u_c' : 'u_p') + index;
}

/** Missing placeholders preserve project data but have no shader to render. */
export function hasRenderableEffects(effects: Array<{ on?: boolean; missing?: boolean }>): boolean {
  return effects.some((effect) => effect.on && effect.missing !== true);
}

/** Evaluate a persisted effect channel, falling back when an extension adds a
    parameter after the project was saved. This keeps one bad effect off the
    whole-frame failure path. */
export function effectParamValue(PM: PMRegistry, layer: any, effect: any, param: any, time: number): any {
  const prop = effect?.p?.[param.k];
  if (!prop) return param.def;
  const value = PM.evP(layer, prop, time, param.k);
  return value == null ? param.def : value;
}

export function install(PM: PMRegistry): void {

const GL: any = {
  gl: null, canvas: null, w: 0, h: 0,
  progs: new Map(), texes: new Map(), pool: [], quad: null,
  stats: { draws: 0, passes: 0, ms: 0, progs: 0 },
  errors: new Map(),
};
PM.GL = GL;
const MAX_FBO_BYTES = PM.Memory?.budget?.('framebuffers') || 384 * 1024 * 1024;
const MAX_TEXTURE_BYTES = PM.Memory?.budget?.('textures') || 192 * 1024 * 1024;
let poolBytes = 0;
let textureBytes = 0;
let resourceTick = 0;

/* ── program cache ─────────────────────────────────────── */
function shader(gl: any, type: any, src: any) {
  const s = gl.createShader(type);
  gl.shaderSource(s, src); gl.compileShader(s);
  if (!gl.getShaderParameter(s, gl.COMPILE_STATUS)) {
    const log = gl.getShaderInfoLog(s);
    gl.deleteShader(s);
    throw new Error(log || 'shader compile failed');
  }
  return s;
}
function program(key: any, frag: any, vert?: any) {
  const gl = GL.gl;
  let p = GL.progs.get(key);
  if (p !== undefined) return p;
  try {
    const v = shader(gl, gl.VERTEX_SHADER, vert || PM.VERT);
    const f = shader(gl, gl.FRAGMENT_SHADER, frag);
    const pr = gl.createProgram();
    gl.attachShader(pr, v); gl.attachShader(pr, f); gl.linkProgram(pr);
    if (!gl.getProgramParameter(pr, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(pr));
    gl.deleteShader(v); gl.deleteShader(f);
    p = { pr, u: new Map(), a: gl.getAttribLocation(pr, 'a_pos') };
    GL.errors.delete(key);
  } catch (e: any) {
    GL.errors.set(key, String(e.message || e).trim());
    p = null;
  }
  GL.progs.set(key, p);
  GL.stats.progs = GL.progs.size;
  return p;
}
GL.compileError = (key: any) => GL.errors.get(key) || null;
GL.dropProgram = (key: any) => {
  const p = GL.progs.get(key);
  if (p && p.pr) GL.gl.deleteProgram(p.pr);
  GL.progs.delete(key); GL.errors.delete(key);
};

function uloc(p: any, name: any) {
  let l = p.u.get(name);
  if (l === undefined) { l = GL.gl.getUniformLocation(p.pr, name); p.u.set(name, l); }
  return l;
}
function use(p: any) {
  const gl = GL.gl;
  gl.useProgram(p.pr);
  gl.bindBuffer(gl.ARRAY_BUFFER, GL.quad);
  gl.enableVertexAttribArray(p.a);
  gl.vertexAttribPointer(p.a, 2, gl.FLOAT, false, 0, 0);
  return { u: (n: any, ...v: any[]) => setU(p, n, v), p };
}
function setU(p: any, n: any, v: any) {
  const gl = GL.gl, l = uloc(p, n);
  if (l === null) return;
  if (v.length === 1 && v[0] && v[0].length === 9) gl.uniformMatrix3fv(l, false, v[0]);
  else if (v.length === 1 && Array.isArray(v[0])) {
    const a = v[0];
    a.length === 2 ? gl.uniform2fv(l, a) : a.length === 3 ? gl.uniform3fv(l, a) : gl.uniform4fv(l, a);
  }
  else if (v.length === 1) Number.isInteger(v[0]) && p.ints && p.ints.has(n) ? gl.uniform1i(l, v[0]) : gl.uniform1f(l, v[0]);
  else if (v.length === 2) gl.uniform2f(l, v[0], v[1]);
  else if (v.length === 3) gl.uniform3f(l, v[0], v[1], v[2]);
  else gl.uniform4f(l, v[0], v[1], v[2], v[3]);
}
function setI(p: any, n: any, v: any) { const l = uloc(p, n); if (l !== null) GL.gl.uniform1i(l, v); }

/* Effect/transition params bind by NAME (`u_amount`) — that is what the kernel
   generates for extension shaders. Legacy raw shaders declare the positional
   `u_p<i>`/`u_c<i>` names instead, so fall back to those when the named uniform
   is not present in the linked program. */
function setParam(p: any, pd: any, index: number, value: any) {
  const name = paramUniformName(pd, index, uloc(p, 'u_' + pd.k) !== null);
  if (pd.type === 'color') setU(p, name, PM.hex2rgb(String(value)));
  else if (pd.type === 'toggle') setU(p, name, [value ? 1 : 0]);
  else setU(p, name, [Number(value) || 0]);
}

/* ── FBO pool ──────────────────────────────────────────── */
function grab(w: any, h: any) {
  const gl = GL.gl;
  for (let i = 0; i < GL.pool.length; i++) {
    const f = GL.pool[i];
    if (!f.busy && f.w === w && f.h === h) { f.busy = true; f.used = ++resourceTick; return f; }
  }
  const tex = gl.createTexture();
  bindTex(0, tex);
  gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA16F, w, h, 0, gl.RGBA, gl.HALF_FLOAT, null);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  bindTex(0, null);
  const fb = gl.createFramebuffer();
  gl.bindFramebuffer(gl.FRAMEBUFFER, fb);
  gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, tex, 0);
  const f = { fb, tex, w, h, busy: true, used: ++resourceTick, bytes: Math.max(0, w * h * 8) };
  GL.pool.push(f);
  poolBytes += f.bytes;
  return f;
}
const free = (f: any) => {
  if (!f) return;
  f.busy = false;
  PM.Memory?.maintain?.('framebuffers');
};
function disposeFbo(f: any) {
  if (!f) return;
  GL.gl?.deleteFramebuffer?.(f.fb);
  GL.gl?.deleteTexture?.(f.tex);
  poolBytes = Math.max(0, poolBytes - (f.bytes || 0));
}
function trimPool(targetBytes: number = MAX_FBO_BYTES) {
  const freeEntries = GL.pool.filter((f: any) => !f.busy).sort((a: any, b: any) => a.used - b.used);
  for (const f of freeEntries) {
    if (poolBytes <= targetBytes || GL.pool.length <= 1) break;
    const index = GL.pool.indexOf(f);
    if (index >= 0) GL.pool.splice(index, 1);
    disposeFbo(f);
  }
}
/* Texture units that still reference a pooled texture from an earlier pass
   would form a framebuffer/texture feedback loop once that FBO is bound again
   (Chromium rejects the draw; WebKit silently tolerated it). */
const boundTex: any[] = [];
let boundFbo: any = null;
function bind(f: any) {
  const gl = GL.gl;
  if (f) for (let u = 0; u < boundTex.length; u++) if (boundTex[u] === f.tex) bindTex(u, null);
  gl.bindFramebuffer(gl.FRAMEBUFFER, f ? f.fb : null);
  gl.viewport(0, 0, f ? f.w : GL.canvas.width, f ? f.h : GL.canvas.height);
  boundFbo = f || null;
}
function clear(r = 0, g = 0, b = 0, a = 0) {
  const gl = GL.gl; gl.clearColor(r, g, b, a); gl.clear(gl.COLOR_BUFFER_BIT);
}
const IDENT = new Float32Array([1, 0, 0, 0, 1, 0, 0, 0, 1]);
function m3(m: any) { return new Float32Array([m[0], m[1], 0, m[2], m[3], 0, m[4], m[5], 1]); }
function fullQuad(w: any, h: any) { return new Float32Array([w, 0, 0, 0, h, 0, 0, 0, 1]); }
function outputScale(W: number, H: number): [number, number] {
  const comp = PM.curComp?.() || PM.proj;
  return [W / Math.max(1, Number(comp?.w) || W), H / Math.max(1, Number(comp?.h) || H)];
}
function scaledWorld(layer: any, time: any, W: number, H: number): [number, number, number, number, number, number] {
  const world = PM.worldMatrix(layer, time);
  const [sx, sy] = outputScale(W, H);
  return [world[0] * sx, world[1] * sy, world[2] * sx, world[3] * sy, world[4] * sx, world[5] * sy];
}

/* ── content textures ──────────────────────────────────── */
function texFor(key: any, source: any, opts: any = {}) {
  const gl = GL.gl;
  let t = GL.texes.get(key);
  if (!t) {
    t = { tex: gl.createTexture(), v: -1, bytes: 0, used: 0 };
    bindTex(0, t.tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
    GL.texes.set(key, t);
  }
  t.used = ++resourceTick;
  if (opts.version !== undefined && t.v === opts.version) return t.tex;
  bindTex(0, t.tex);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  try { gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source); } catch (e) { }
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  t.v = opts.version === undefined ? t.v : opts.version;
  const width = Number(source?.videoWidth || source?.naturalWidth || source?.width || 0);
  const height = Number(source?.videoHeight || source?.naturalHeight || source?.height || 0);
  const bytes = Math.max(0, width * height * 4);
  textureBytes += bytes - (t.bytes || 0);
  t.bytes = bytes;
  PM.Memory?.maintain?.('textures');
  return t.tex;
}
function trimTextures(targetBytes: number = MAX_TEXTURE_BYTES) {
  const entries = [...GL.texes.entries()].sort((a: any, b: any) => a[1].used - b[1].used);
  for (const [key, entry] of entries) {
    if (textureBytes <= targetBytes || GL.texes.size <= 1) break;
    if (boundTex.includes(entry.tex)) continue;
    GL.gl?.deleteTexture?.(entry.tex);
    textureBytes = Math.max(0, textureBytes - (entry.bytes || 0));
    GL.texes.delete(key);
  }
}
GL.dropTextures = (prefix = '') => {
  for (const [key, entry] of GL.texes) {
    if (!key.startsWith(prefix)) continue;
    if (GL.gl && entry.tex) GL.gl.deleteTexture(entry.tex);
    textureBytes = Math.max(0, textureBytes - (entry.bytes || 0));
    GL.texes.delete(key);
  }
};
function bindTex(unit: any, tex: any) {
  const gl = GL.gl;
  gl.activeTexture(gl.TEXTURE0 + unit);
  gl.bindTexture(gl.TEXTURE_2D, tex);
  boundTex[unit] = tex;
}
const draw = () => { GL.gl.drawArrays(GL.gl.TRIANGLE_STRIP, 0, 4); GL.stats.draws++; };

/* ── init ──────────────────────────────────────────────── */
GL.init = (canvas: any) => {
  GL.canvas = canvas;
  const gl = canvas.getContext('webgl2', {
    alpha: false, antialias: false, premultipliedAlpha: true,
    preserveDrawingBuffer: true, powerPreference: 'high-performance', desynchronized: true,
  });
  if (!gl) { window.alert('Powermove needs WebGL2.'); return false; }
  GL.gl = gl;
  gl.getExtension('EXT_color_buffer_half_float');
  gl.getExtension('EXT_color_buffer_float');
  gl.getExtension('OES_texture_float_linear');
  GL.quad = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, GL.quad);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
  gl.disable(gl.DEPTH_TEST);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  return true;
};

GL.resize = (w: any, h: any) => {
  if (GL.canvas.width === w && GL.canvas.height === h) return;
  GL.canvas.width = w; GL.canvas.height = h;
  GL.pool.forEach((f: any) => disposeFbo(f));
  GL.pool.length = 0;
};

GL.memoryStats = () => ({
  framebuffers: { entries: GL.pool.length, bytes: poolBytes, maxBytes: MAX_FBO_BYTES },
  textures: { entries: GL.texes.size, bytes: textureBytes, maxBytes: MAX_TEXTURE_BYTES },
});
PM.Memory?.register?.('framebuffers', {
  bytes: () => poolBytes,
  entries: () => GL.pool.length,
  trim: (target: number) => trimPool(target),
});
PM.Memory?.register?.('textures', {
  bytes: () => textureBytes,
  entries: () => GL.texes.size,
  trim: (target: number) => trimTextures(target),
});

/* ── layer content ─────────────────────────────────────── */
function contentQuad(L: any, T: any, W: any, H: any) {
  /* returns {tex, w, h, ax, ay, uv:[ox,oy,sx,sy], fromFbo, solid, tmp} */
  const d = L.d;
  if (L.type === 'solid') {
    return { solid: PM.hex2rgb(d.color), w: d.w || W, h: d.h || H, ax: 0, ay: 0 };
  }
  if (L.type === 'text' || L.type === 'shape') {
    const ss = PM.clamp(PM.quality || 1, .5, 2);
    const r = PM.raster(L, ss);
    const tex = texFor('r:' + r.key, r.cv, { version: 1 });
    const ax = L.type === 'shape' ? r.w / 2 : r.anchorX;
    const ay = L.type === 'shape' ? r.h / 2 : r.anchorY;
    return { tex, w: r.w, h: r.h, ax, ay, uv: [0, 0, 1, 1] };
  }
  if (L.type === 'image' || L.type === 'video') {
    const a = PM.assets.get(d.asset);
    if (!a) return null;
    let el = a.el, sw = a.w || 1, sh = a.h || 1;
    if (L.type === 'video') {
      const vt = PM.clamp((T - L.from) * (d.speed || 1) + (d.trim || 0), 0, Math.max(0, a.dur - .04));
      if (!PM.playing && Math.abs(el.currentTime - vt) > .02) { try { el.currentTime = vt; } catch (e) { } }
      sw = el.videoWidth || sw; sh = el.videoHeight || sh;
    }
    const videoVersion = L.type === 'video'
      ? Number(el.getVideoPlaybackQuality?.().totalVideoFrames ?? el.webkitDecodedFrameCount ?? Math.round(Number(el.currentTime || 0) * 1000))
      : 1;
    const tex = texFor('a:' + a.id, el, { version: videoVersion });
    const bw = d.w || W, bh = d.h || H;
    let uv = [0, 0, 1, 1];
    if (d.fit === 'cover' || d.fit === 'contain') {
      const ar = sw / sh, br = bw / bh;
      if (d.fit === 'cover') {
        if (ar > br) { const s = br / ar; uv = [(1 - s) / 2, 0, s, 1]; }
        else { const s = ar / br; uv = [0, (1 - s) / 2, 1, s]; }
      } else {
        if (ar > br) { const s = ar / br; uv = [0, (1 - s) / 2, 1, s]; }
        else { const s = br / ar; uv = [(1 - s) / 2, 0, s, 1]; }
      }
    }
    return { tex, w: bw, h: bh, ax: bw / 2, ay: bh / 2, uv };
  }
  if (L.type === 'shader') {
    const comp = PM.curComp?.() || PM.proj;
    const w = Math.max(1, Number(d.w || comp.w || W)), hh = Math.max(1, Number(d.h || comp.h || H));
    const [sx, sy] = outputScale(W, H);
    const targetW = Math.min(4096, Math.max(2, Math.round(w * sx)));
    const targetH = Math.min(4096, Math.max(2, Math.round(hh * sy)));
    const key = 'sh:' + L.id;
    const codeKey = key + ':' + hashStr(d.code);
    const p = program(codeKey, PM.SHADER_HEADER + '\n' + d.code, PM.VERT);
    PM.UIState.setShaderMeta(L, { shaderKey: codeKey });
    if (!p) return null;
    /* nested content renders into its own FBO; the caller's target is restored
       before drawContent samples the result (otherwise the draw reads and writes
       the same texture) */
    const target = boundFbo;
    const f = grab(targetW, targetH);
    bind(f); clear(0, 0, 0, 0);
    const g = use(p);
    g.u('u_m', fullQuad(f.w, f.h)); g.u('u_res', f.w, f.h); g.u('u_uv', 0, 0, 1, 1);
    setU(p, 'iResolution', [f.w, f.h]);
    setU(p, 'iTime', [T - L.from]); setU(p, 'iGlobalTime', [T]);
    setU(p, 'iProgress', [PM.clamp((T - L.from) / Math.max(L.dur, 1e-4), 0, 1)]);
    setI(p, 'iFrame', Math.round(T * PM.proj.fps));
    setU(p, 'iMouse', [0, 0]);
    for (const un in d.uniforms) {
      const def = PM.UIState.getShaderMeta(L).udefs.find((u: any) => u.name === un);
      const val = PM.evP(L, d.uniforms[un], T, un);
      if (def && def.control === 'color') setU(p, un, PM.hex2rgb(String(val)));
      else if (def && def.control === 'toggle') setI(p, un, val ? 1 : 0);
      else setU(p, un, [Number(val) || 0]);
    }
    GL.gl.disable(GL.gl.BLEND);
    draw();
    GL.gl.enable(GL.gl.BLEND);
    bind(target);
    return { tex: f.tex, w, h: hh, ax: 0, ay: 0, uv: [0, 0, 1, 1], fromFbo: true, tmp: f };
  }
  if (L.type === 'precomp') {
    const sub = PM.compOf(L);
    if (!sub || pcDepth >= PC_MAX_DEPTH) return null;
    const comp = PM.curComp?.() || PM.proj;
    const w = Math.max(1, Number(d.w || comp.w || W)), hh = Math.max(1, Number(d.h || comp.h || H));
    const [sx, sy] = outputScale(W, H);
    const targetW = Math.max(2, Math.round(w * sx)), targetH = Math.max(2, Math.round(hh * sy));
    const target = boundFbo;
    const f = grab(targetW, targetH);
    bind(f); clear(0, 0, 0, 0);
    pmScopePush(sub);
    try {
      const inner = GL.renderProject(sub, T - L.from, targetW, targetH, { transparent: true });
      /* blit the nested result into our FBO so ownership stays with this level */
      bind(f);
      const p = program('copyA', PM.FRAG_DRAW);
      const g = use(p);
      bindTex(0, inner.tex); setI(p, 'u_tex', 0);
      g.u('u_m', fullQuad(targetW, targetH)); g.u('u_res', targetW, targetH); g.u('u_uv', 0, 0, 1, 1);
      g.u('u_alpha', 1); setI(p, 'u_fromFbo', 1);
      /* Integrator fix (Phase 3b): the legacy source referenced an undeclared `gl`
         here, so every precomp render threw a ReferenceError. Use the context
         the shader branch above uses. */
      GL.gl.disable(GL.gl.BLEND); draw(); GL.gl.enable(GL.gl.BLEND); GL.stats.passes++;
      free(inner);
    } finally {
      pmScopePop();
      bind(target);
    }
    return { tex: f.tex, w, h: hh, ax: 0, ay: 0, uv: [0, 0, 1, 1], fromFbo: true, tmp: f };
  }
  return null;
}
/* Scope management for nested rendering — renderProject runs inside these. */
function pmScopePush(proj: any) { PM.scope.push(proj); pcDepth++; }
function pmScopePop() { PM.scope.pop(); pcDepth--; }
function hashStr(s: any) { let h = 5381; for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0; return h; }

/* Draw one layer's content (with transform) into the bound target. */
function drawContent(L: any, T: any, W: any, H: any, alpha: any) {
  const c: any = contentQuad(L, T, W, H);
  if (!c) return false;
  const world = scaledWorld(L, T, W, H);
  const M = PM.mul(world, [c.w, 0, 0, c.h, -c.ax, -c.ay]);
  if (c.solid) {
    const p = program('solid', PM.FRAG_SOLID);
    if (!p) return false;
    const g = use(p);
    g.u('u_m', m3(M)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
    g.u('u_color', c.solid[0] * alpha, c.solid[1] * alpha, c.solid[2] * alpha, alpha);
    draw();
    return true;
  }
  const p = program('draw', PM.FRAG_DRAW);
  if (!p) return false;
  const g = use(p);
  bindTex(0, c.tex); setI(p, 'u_tex', 0);
  g.u('u_m', m3(M)); g.u('u_res', W, H);
  g.u('u_uv', c.uv[0], c.uv[1], c.uv[2], c.uv[3]);
  g.u('u_alpha', alpha);
  setI(p, 'u_fromFbo', c.fromFbo ? 1 : 0);
  draw();
  free(c.tmp);
  return true;
}

/* ── effects chain ─────────────────────────────────────── */
function runEffects(L: any, T: any, srcF: any, W: any, H: any) {
  let cur = srcF;
  for (const fx of L.fx) {
    /* `missing` marks an effect whose type is not registered (the extension
       providing it is off or gone). Skip it and keep its data intact. */
    if (!fx.on || fx.missing) continue;
    const def = PM.FX[fx.type];
    if (!def) continue;
    const key = 'fx:' + fx.type;
    const p = program(key, def.frag);
    if (!p) continue;
    const orig = def.keepOrig ? cur : null;
    let input = cur;
    for (let pass = 0; pass < def.passes; pass++) {
      const out = grab(W, H);
      bind(out); clear();
      const g = use(p);
      bindTex(0, input.tex); setI(p, 'u_tex', 0);
      if (orig) { bindTex(1, orig.tex); setI(p, 'u_orig', 1); }
      g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
      g.u('u_texel', 1 / W, 1 / H);
      g.u('u_time', T);
      setI(p, 'u_pass', pass);
      def.params.forEach((pd: any, i: any) => setParam(p, pd, i, effectParamValue(PM, L, fx, pd, T)));
      GL.gl.disable(GL.gl.BLEND);
      draw();
      GL.gl.enable(GL.gl.BLEND);
      GL.stats.passes++;
      if (input !== cur) free(input);
      input = out;
    }
    if (cur !== srcF) free(cur);
    cur = input;
  }
  return cur;
}

/* ── masks ─────────────────────────────────────────────── */
/* Rasterize analytic mask coverage (comp px → layer px via the inverse world
   matrix) and multiply it into the layer buffer. Runs before effects, AE-style. */
const MASK_MAX = 8;
function applyMasks(L: any, T: any, srcF: any, W: any, H: any) {
  const gl = GL.gl;
  const masks = (L.masks || []).filter((m: any) => m && m.on !== false && m.p);
  if (!masks.length) return srcF;
  const world = scaledWorld(L, T, W, H);
  const det = world[0] * world[3] - world[1] * world[2];
  if (!Number.isFinite(det) || Math.abs(det) < 1e-9) return srcF;
  /* inverse of the 2×3 world matrix */
  const inv = [
    world[3] / det, -world[1] / det, -world[2] / det, world[0] / det,
    (-world[3] * world[4] + world[2] * world[5]) / det,
    (world[1] * world[4] - world[0] * world[5]) / det,
  ];

  const cov = grab(W, H);
  bind(cov); clear(0, 0, 0, 0);
  const p = program('masks', PM.FRAG_MASK);
  if (!p) { free(cov); return srcF; }
  const gArr = new Array(32).fill(0), qArr = new Array(32).fill(0);
  let cnt = 0, hasAdd = 0;
  for (const m of masks) {
    if (cnt >= MASK_MAX) break;
    const g = cnt * 4, q = cnt * 4;
    gArr[g + 0] = Number(PM.evP(L, m.p.x, T, 'x')) || 0;
    gArr[g + 1] = Number(PM.evP(L, m.p.y, T, 'y')) || 0;
    gArr[g + 2] = Math.abs(Number(PM.evP(L, m.p.w, T, 'w')) || 0);
    gArr[g + 3] = Math.abs(Number(PM.evP(L, m.p.h, T, 'h')) || 0);
    qArr[q + 0] = (Number(PM.evP(L, m.p.rotation, T, 'rotation')) || 0) * Math.PI / 180;
    qArr[q + 1] = Math.max(0, Number(PM.evP(L, m.p.feather, T, 'feather')) || 0);
    qArr[q + 2] = m.shape === 'ellipse' ? 1 : 0;
    qArr[q + 3] = m.mode === 'subtract' ? 1 : 0;
    if (m.mode !== 'subtract') hasAdd = 1;
    cnt++;
  }
  if (cnt === 0) { free(cov); return srcF; }
  {
    const pm = use(p);
    pm.u('u_m', fullQuad(W, H)); pm.u('u_res', W, H); pm.u('u_uv', 0, 0, 1, 1);
    setU(p, 'u_inv', m3(inv));
    setI(p, 'u_cnt', cnt); setI(p, 'u_hasAdd', hasAdd);
    setU(p, 'u_g', [gArr]); setU(p, 'u_q', [qArr]);
    gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
    GL.stats.passes++;
  }
  const out = grab(W, H);
  bind(out); clear();
  const pa = program('maskApply', PM.FRAG_MASK_APPLY);
  const ga = use(pa);
  bindTex(0, srcF.tex); setI(pa, 'u_tex', 0);
  bindTex(1, cov.tex); setI(pa, 'u_cov', 1);
  ga.u('u_m', fullQuad(W, H)); ga.u('u_res', W, H); ga.u('u_uv', 0, 0, 1, 1);
  gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
  GL.stats.passes++;
  free(cov);
  return out;
}

/* ── main render ───────────────────────────────────────── */
const BLEND_ID = { normal: 0, add: 1, screen: 2, multiply: 3, overlay: 4, softlight: 5, difference: 6, lighten: 7, darken: 8 };
const PC_MAX_DEPTH = 6;
let pcDepth = 0;

function copyFbo(srcF: any, W: any, H: any) {
  const out = grab(W, H);
  bind(out); clear();
  const p = program('copy', PM.FRAG_COPY);
  if (p) {
    const g = use(p);
    bindTex(0, srcF.tex); setI(p, 'u_tex', 0);
    g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
    GL.gl.disable(GL.gl.BLEND); draw(); GL.gl.enable(GL.gl.BLEND); GL.stats.passes++;
  }
  return out;
}

function activeTransition(L: any, T: any) {
  const start = Number(L.from) || 0;
  const length = Math.max(0, Number(L.dur) || 0);
  const end = start + length;
  const at = (transition: any, edge: 'in' | 'out') => {
    if (!transition || transition.missing || typeof transition.type !== 'string') return null;
    const def = PM.transitionDef?.(transition.type);
    if (!def) return null;
    const dur = Math.min(length, Math.max(.02, Number(transition.dur) || .5));
    if (dur <= 0) return null;
    if (edge === 'in' && T >= start && T <= start + dur) {
      return { transition, def, edge, prog: PM.clamp((T - start) / dur, 0, 1) };
    }
    if (edge === 'out' && T >= end - dur && T <= end) {
      return { transition, def, edge, prog: PM.clamp((T - (end - dur)) / dur, 0, 1) };
    }
    return null;
  };
  return at(L.transitionIn, 'in') || at(L.transitionOut, 'out');
}

function runTransition(L: any, T: any, activeTr: any, before: any, withLayer: any, W: any, H: any) {
  const key = 'tr:' + activeTr.transition.type;
  const p = program(key, activeTr.def.frag);
  if (!p) return null;

  const out = grab(W, H);
  bind(out); clear();
  const g = use(p);
  const from = activeTr.edge === 'in' ? before : withLayer;
  const to = activeTr.edge === 'in' ? withLayer : before;
  bindTex(0, from.tex); setI(p, 'u_from', 0);
  bindTex(1, to.tex); setI(p, 'u_to', 1);
  g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
  g.u('u_texel', 1 / W, 1 / H); g.u('u_time', T); g.u('u_prog', activeTr.prog);
  (activeTr.def.params || []).forEach((pd: any, index: number) => {
    const prop = activeTr.transition.p && activeTr.transition.p[pd.k];
    setParam(p, pd, index, prop ? PM.evP(L, prop, T, pd.k) : pd.def);
  });
  GL.gl.disable(GL.gl.BLEND); draw(); GL.gl.enable(GL.gl.BLEND);
  GL.stats.passes++;
  return out;
}

/** Render a whole project (main or nested) into a pooled FBO and return it.
    opt.transparent skips the background fill (nested comps composite over). */
GL.renderProject = (proj: any, T: any, W: any, H: any, opt: any = {}) => {
  const gl = GL.gl;
  const layers = proj.layers;

  let acc = grab(W, H);
  bind(acc);
  if (opt.transparent) clear(0, 0, 0, 0);
  else {
    const fill = PM.normalizeFill(proj.backgroundFill, proj.bg);
    if (fill.type === 'none') clear(0, 0, 0, 0);
    else if (fill.type === 'solid') { const bg = PM.hex2rgb(fill.stops[0].color); clear(bg[0], bg[1], bg[2], 1); }
    else {
      clear(0, 0, 0, 1);
      const p = program('background-fill', PM.FRAG_BACKGROUND_FILL);
      if (p) {
        const g = use(p), packed = [];
        fill.stops.forEach((stop: any) => packed.push(...PM.hex2rgb(stop.color), stop.position / 100));
        while (packed.length < 32) packed.push(0, 0, 0, 1);
        g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
        setI(p, 'u_count', fill.stops.length); setI(p, 'u_type', fill.type === 'radial' ? 2 : 1);
        g.u('u_angle', fill.angle); g.u('u_stops', packed); draw();
      }
    }
  }

  for (let i = layers.length - 1; i >= 0; i--) {
    const L = layers[i];
    if (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].visual === false) continue;
    if (!PM.active(L, T)) continue;
    if (L.shy && opt.hideShy) continue;
    const alpha = PM.worldOpacity(L, T);
    if (alpha <= .001) continue;

    const hasFx = hasRenderableEffects(L.fx);
    const hasMasks = (L.masks || []).some((m: any) => m.on !== false);
    const blend = (BLEND_ID as any)[L.blend] || 0;
    const mb = L.mblur && opt.mblur !== false;
    const transition = activeTransition(L, T);

    /* fast path: no masks, no effects, normal blend, no motion blur → straight into acc */
    if (!hasMasks && !hasFx && !blend && !mb && !transition) {
      bind(acc);
      drawContent(L, T, W, H, alpha);
      continue;
    }

    const lf = grab(W, H);
    bind(lf); clear();
    if (mb) {
      const n = opt.mbSamples || 10, shutter = (opt.shutter || .5) / proj.fps;
      for (let s = 0; s < n; s++) {
        const dt = ((s + .5) / n - .5) * shutter;
        drawContent(L, T + dt, W, H, alpha / n);
      }
    } else drawContent(L, T, W, H, alpha);

    let res = lf;
    if (hasMasks) res = applyMasks(L, T, res, W, H);
    if (hasFx) res = runEffects(L, T, res, W, H);

    let withLayer = acc;
    if (!blend) {
      if (transition) withLayer = copyFbo(acc, W, H);
      bind(withLayer);
      const p = program('copyA', PM.FRAG_DRAW);
      const g = use(p);
      bindTex(0, res.tex); setI(p, 'u_tex', 0);
      g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
      g.u('u_alpha', 1); setI(p, 'u_fromFbo', 1);
      draw();
    } else {
      const nxt = grab(W, H);
      bind(nxt); clear();
      const p = program('comp', PM.FRAG_COMPOSITE);
      const g = use(p);
      bindTex(0, res.tex); setI(p, 'u_tex', 0);
      bindTex(1, acc.tex); setI(p, 'u_dst', 1);
      g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
      g.u('u_alpha', 1); setI(p, 'u_blend', blend);
      gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
      withLayer = nxt;
      if (!transition) { free(acc); acc = nxt; }
    }
    if (transition) {
      const transitioned = runTransition(L, T, transition, acc, withLayer, W, H);
      free(acc);
      acc = transitioned || withLayer;
      if (transitioned) free(withLayer);
    }
    if (res !== lf) free(res);
    free(lf);
  }
  return acc;
};

GL.render = (T: any, opt: any = {}) => {
  const gl = GL.gl; if (!gl) return;
  const t0 = window.performance.now();
  GL.stats.draws = 0; GL.stats.passes = 0;
  const W = GL.canvas.width, H = GL.canvas.height;

  PM.beginEval(T);
  PM.scope.push(PM.proj);
  let acc;
  try {
    acc = GL.renderProject(PM.proj, T, W, H, opt);
  } finally {
    PM.scope.pop();
  }

  /* present */
  bind(null);
  gl.clearColor(0, 0, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT);
  const p = program('present', PM.FRAG_COPY);
  const g = use(p);
  bindTex(0, acc.tex); setI(p, 'u_tex', 0);
  g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
  gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
  free(acc);
  GL.pool.forEach((f: any) => f.busy = false);
  trimPool();
  trimTextures();
  GL.stats.ms = window.performance.now() - t0;
};

/** Render one frame and read back raw RGBA pixels (bottom-up, premultiplied).
    Used for transparent PNG export where the canvas itself is opaque. */
GL.renderToPixels = (T: any, W: any, H: any, opt: any = {}) => {
  const gl = GL.gl; if (!gl) return null;
  PM.beginEval(T);
  PM.scope.push(PM.proj);
  let acc;
  try { acc = GL.renderProject(PM.proj, T, W, H, opt); }
  finally { PM.scope.pop(); }
  bind(acc);
  const px = new Uint8Array(W * H * 4);
  gl.readPixels(0, 0, W, H, gl.RGBA, gl.UNSIGNED_BYTE, px);
  free(acc);
  GL.pool.forEach((f: any) => f.busy = false);
  trimPool();
  trimTextures();
  return px;
};

/* Hit test: which layer is under a comp-space point (top-most first). */
GL.pick = (x: any, y: any, T: any) => {
  const layers = PM.proj.layers;
  for (const L of layers) {
    if (!PM.active(L, T) || L.lock || (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].pickable === false)) continue;
    const b = GL.bounds(L, T);
    if (!b) continue;
    const m = PM.worldMatrix(L, T);
    const det = m[0] * m[3] - m[1] * m[2];
    if (Math.abs(det) < 1e-9) continue;
    const dx = x - m[4], dy = y - m[5];
    const lx = (dx * m[3] - dy * m[2]) / det;
    const ly = (dy * m[0] - dx * m[1]) / det;
    if (lx >= b.x0 && lx <= b.x1 && ly >= b.y0 && ly <= b.y1) return L;
  }
  return null;
};
/** Layer-space bounds (before transform), relative to the layer anchor origin. */
GL.bounds = (L: any, T: any) => {
  const d = L.d;
  let w, h, ax, ay;
  if (L.type === 'solid' || L.type === 'shader') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'precomp') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'text') {
    const r = PM.raster(L, 1);
    if (r.selection) return { ...r.selection, ax: r.anchorX, ay: r.anchorY };
    w = r.w; h = r.h; ax = r.anchorX; ay = r.anchorY;
  }
  else if (L.type === 'shape') {
    const r = PM.raster(L, 1);
    w = r.w; h = r.h;
    ax = w / 2; ay = h / 2;
  } else if (L.type === 'image' || L.type === 'video') {
    w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = w / 2; ay = h / 2;
  } else return null;
  return { x0: -ax, y0: -ay, x1: w - ax, y1: h - ay, w, h, ax, ay };
};
}
