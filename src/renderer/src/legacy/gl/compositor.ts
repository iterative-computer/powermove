/* Ported from js/gl/compositor.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { extensionLayerFragment } from '../../kernel/extension-layers';

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

/** Track frames the browser actually presents, rather than decoder totals that
    can advance in bursts before the corresponding frame is available to WebGL. */
export function trackPresentedVideoFrames(el: any, onFrame: () => void): { version: number; supported: boolean } {
  const state = { version: 0, supported: typeof el?.requestVideoFrameCallback === 'function' };
  if (!state.supported) return state;
  const presented = () => {
    state.version++;
    onFrame();
    el.requestVideoFrameCallback(presented);
  };
  el.requestVideoFrameCallback(presented);
  return state;
}

/** Choose the source-texture density from the layer's actual display matrix.
 * Text and shape layers remain editable source data, but the GPU still needs a
 * bitmap at the final sampling boundary. Rebuilding that bitmap as its display
 * scale grows avoids baking a vector layer at 1x and stretching those pixels.
 * Quarter-step buckets keep animated scale from creating a new cache entry on
 * every frame while remaining visually indistinguishable at preview size. */
export function continuousRasterScale(
  matrix: readonly number[], density = 1, maxScale = 8,
): number {
  const m0 = Number(matrix?.[0]) || 0, m1 = Number(matrix?.[1]) || 0;
  const m2 = Number(matrix?.[2]) || 0, m3 = Number(matrix?.[3]) || 0;
  const aa = m0 * m0 + m1 * m1;
  const bb = m0 * m2 + m1 * m3;
  const cc = m2 * m2 + m3 * m3;
  const discriminant = Math.sqrt(Math.max(0, (aa - cc) ** 2 + 4 * bb * bb));
  const largest = Math.sqrt(Math.max(0, (aa + cc + discriminant) / 2));
  const requested = largest * Math.max(.01, Number(density) || 1);
  const bounded = Math.max(.25, Math.min(Math.max(.25, Number(maxScale) || 8), requested));
  return Math.ceil(bounded * 4 - 1e-9) / 4;
}

export function install(PM: PMRegistry): void {

const GL: any = {
  gl: null, canvas: null, w: 0, h: 0,
  progs: new Map(), texes: new Map(), meshes: new Map(), pool: [], quad: null,
  stats: { draws: 0, passes: 0, ms: 0, progs: 0 },
  errors: new Map(),
};
PM.GL = GL;
const MAX_FBO_BYTES = PM.Memory?.budget?.('framebuffers') || 384 * 1024 * 1024;
const MAX_TEXTURE_BYTES = PM.Memory?.budget?.('textures') || 192 * 1024 * 1024;
let poolBytes = 0;
let textureBytes = 0;
let resourceTick = 0;
const presentedVideoFrames = new WeakMap<any, { version: number; supported: boolean }>();

function videoTextureVersion(el: any): number {
  let state = presentedVideoFrames.get(el);
  if (!state) {
    state = trackPresentedVideoFrames(el, () => PM.invalidate('render'));
    presentedVideoFrames.set(el, state);
  }
  if (state.supported) return state.version;
  return Number(el.getVideoPlaybackQuality?.().totalVideoFrames
    ?? el.webkitDecodedFrameCount
    ?? Math.round(Number(el.currentTime || 0) * 1000));
}

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
GL.dropPrograms = (prefix: string) => {
  for (const key of [...GL.progs.keys()]) if (String(key).startsWith(prefix)) GL.dropProgram(key);
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
function grab(w: any, h: any, depth = false) {
  const gl = GL.gl;
  for (let i = 0; i < GL.pool.length; i++) {
    const f = GL.pool[i];
    if (!f.busy && f.w === w && f.h === h && !!f.depth === depth) { f.busy = true; f.used = ++resourceTick; return f; }
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
  let depthBuffer = null;
  if (depth) {
    depthBuffer = gl.createRenderbuffer();
    gl.bindRenderbuffer(gl.RENDERBUFFER, depthBuffer);
    gl.renderbufferStorage(gl.RENDERBUFFER, gl.DEPTH_COMPONENT16, w, h);
    gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.DEPTH_ATTACHMENT, gl.RENDERBUFFER, depthBuffer);
    gl.bindRenderbuffer(gl.RENDERBUFFER, null);
  }
  const f = { fb, tex, depth, depthBuffer, w, h, busy: true, used: ++resourceTick, bytes: Math.max(0, w * h * (depth ? 10 : 8)) };
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
  if (f.depthBuffer) GL.gl?.deleteRenderbuffer?.(f.depthBuffer);
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
  const gl = GL.gl; gl.clearColor(r, g, b, a); gl.clearDepth(1); gl.clear(gl.COLOR_BUFFER_BIT | gl.DEPTH_BUFFER_BIT);
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

function dropMesh(id: string) {
  const entry = GL.meshes.get(id);
  if (!entry) return;
  GL.gl?.deleteBuffer?.(entry.positions);
  GL.gl?.deleteBuffer?.(entry.normals);
  GL.meshes.delete(id);
}
GL.dropMesh = dropMesh;

function meshBuffers(asset: any) {
  const mesh = asset?.mesh;
  if (!mesh?.positions || !mesh?.normals || !(mesh.vertexCount > 0)) return null;
  const cached = GL.meshes.get(asset.id);
  if (cached?.source === mesh) return cached;
  if (cached) dropMesh(asset.id);
  const gl = GL.gl;
  const positions = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, positions);
  gl.bufferData(gl.ARRAY_BUFFER, mesh.positions, gl.STATIC_DRAW);
  const normals = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, normals);
  gl.bufferData(gl.ARRAY_BUFFER, mesh.normals, gl.STATIC_DRAW);
  const entry = { source: mesh, positions, normals, count: mesh.vertexCount };
  GL.meshes.set(asset.id, entry);
  return entry;
}
function bindTex(unit: any, tex: any) {
  const gl = GL.gl;
  gl.activeTexture(gl.TEXTURE0 + unit);
  gl.bindTexture(gl.TEXTURE_2D, tex);
  boundTex[unit] = tex;
}
const draw = () => { GL.gl.drawArrays(GL.gl.TRIANGLE_STRIP, 0, 4); GL.stats.draws++; };

const MESH_VERTEX = `#version 300 es
precision highp float;
in vec3 a_position;
in vec3 a_normal;
uniform vec2 u_resolution;
uniform float u_yaw;
uniform float u_pitch;
uniform float u_distance;
uniform float u_rotationX;
uniform float u_rotationY;
uniform float u_rotationZ;
uniform float u_scale;
out vec3 v_normal;
out vec3 v_world;
mat3 rx(float a){float c=cos(a),s=sin(a);return mat3(1,0,0,0,c,s,0,-s,c);}
mat3 ry(float a){float c=cos(a),s=sin(a);return mat3(c,0,-s,0,1,0,s,0,c);}
mat3 rz(float a){float c=cos(a),s=sin(a);return mat3(c,s,0,-s,c,0,0,0,1);}
void main(){
  mat3 model=rz(radians(u_rotationZ))*rx(radians(u_rotationX))*ry(radians(u_rotationY));
  vec3 world=model*a_position*u_scale;
  vec3 normal=normalize(model*a_normal);
  float yaw=radians(u_yaw), pitch=radians(u_pitch);
  vec3 camera=u_distance*vec3(cos(pitch)*sin(yaw),sin(pitch),cos(pitch)*cos(yaw));
  vec3 forward=normalize(-camera);
  vec3 right=normalize(cross(forward,vec3(0,1,0)));
  vec3 up=cross(right,forward);
  vec3 rel=world-camera;
  float z=dot(rel,forward);
  float near=.05, far=100.0, focal=1.8;
  float aspect=u_resolution.x/max(u_resolution.y,1.0);
  gl_Position=vec4(dot(rel,right)*focal/aspect,dot(rel,up)*focal,((far+near)/(far-near))*z-(2.0*far*near/(far-near)),z);
  v_normal=normal; v_world=world;
}`;

const MESH_FRAGMENT = `#version 300 es
precision highp float;
in vec3 v_normal;
in vec3 v_world;
uniform vec3 u_objectColor;
uniform vec3 u_lightColor;
uniform float u_roughness;
uniform float u_metalness;
uniform float u_yaw;
uniform float u_pitch;
uniform float u_distance;
out vec4 fragColor;
void main(){
  vec3 n=normalize(v_normal);
  float yaw=radians(u_yaw), pitch=radians(u_pitch);
  vec3 camera=u_distance*vec3(cos(pitch)*sin(yaw),sin(pitch),cos(pitch)*cos(yaw));
  vec3 view=normalize(camera-v_world);
  vec3 key=normalize(vec3(-3.5,5.0,4.0)-v_world);
  vec3 rim=normalize(vec3(4.0,2.5,-3.0)-v_world);
  float diffuse=max(dot(n,key),0.0);
  float rimLight=pow(max(dot(n,rim),0.0),1.5);
  vec3 halfVector=normalize(key+view);
  float power=mix(18.0,180.0,1.0-clamp(u_roughness,0.0,1.0));
  float spec=pow(max(dot(n,halfVector),0.0),power)*(0.12+2.4*clamp(u_metalness,0.0,1.0));
  float fresnel=pow(1.0-max(dot(n,view),0.0),4.0);
  vec3 color=u_objectColor*(0.14+diffuse*(1.0-.65*u_metalness));
  color+=u_lightColor*spec+mix(u_objectColor,u_lightColor,.45)*fresnel*(.25+.7*u_metalness);
  color+=vec3(.35,.42,.65)*rimLight*.2;
  color=color/(color+vec3(1));
  color=pow(color,vec3(1.0/2.2));
  fragColor=vec4(color,1);
}`;

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
PM.bus?.on?.('assets', () => {
  for (const id of [...GL.meshes.keys()]) if (!PM.assets?.get?.(id)) dropMesh(id);
});

/* ── layer content ─────────────────────────────────────── */
function extensionParam(layer: any, definition: any, key: string, time: number, fallback: any) {
  const parameter = definition?.params?.find((item: any) => item.k === key);
  const property = layer.d?.params?.[key];
  return property ? PM.evP(layer, property, time, key) : parameter?.def ?? fallback;
}

/** `undefined` means show the preserved-data placeholder; `null` is a shader failure. */
function meshExtensionQuad(L: any, T: any, w: number, h: number, targetW: number, targetH: number, definition: any): any {
  const assetField = definition.renderer.assetField;
  const assetId = String(L.d?.data?.[assetField] || '');
  const asset = assetId ? PM.assets?.get?.(assetId) : null;
  const buffers = meshBuffers(asset);
  if (!buffers) {
    PM.UIState?.setShaderMeta?.(L, { definition, missing: true, missingAsset: assetId || null });
    return undefined;
  }
  const key = `extension:${definition.id}:mesh:${definition.version}:${hashStr(MESH_VERTEX + MESH_FRAGMENT)}`;
  const p = program(key, MESH_FRAGMENT, MESH_VERTEX);
  PM.UIState?.setShaderMeta?.(L, { shaderKey: key, definition, missing: false, missingAsset: null });
  if (!p) return null;
  const target = boundFbo;
  const f = grab(targetW, targetH, true);
  bind(f);
  const background = PM.hex2rgb(String(extensionParam(L, definition, 'background', T, '#0C0D12')));
  const gl = GL.gl;
  gl.enable(gl.DEPTH_TEST); gl.depthFunc(gl.LEQUAL); gl.depthMask(true);
  gl.disable(gl.BLEND);
  clear(background[0], background[1], background[2], 1);
  gl.useProgram(p.pr);
  const position = gl.getAttribLocation(p.pr, 'a_position');
  const normal = gl.getAttribLocation(p.pr, 'a_normal');
  gl.bindBuffer(gl.ARRAY_BUFFER, buffers.positions);
  gl.enableVertexAttribArray(position); gl.vertexAttribPointer(position, 3, gl.FLOAT, false, 0, 0);
  gl.bindBuffer(gl.ARRAY_BUFFER, buffers.normals);
  gl.enableVertexAttribArray(normal); gl.vertexAttribPointer(normal, 3, gl.FLOAT, false, 0, 0);
  setU(p, 'u_resolution', [f.w, f.h]);
  for (const [uniform, parameter, fallback] of [
    ['u_yaw', 'yaw', 28], ['u_pitch', 'pitch', 18], ['u_distance', 'distance', 4.8],
    ['u_rotationX', 'rotationX', 0], ['u_rotationY', 'rotationY', 25], ['u_rotationZ', 'rotationZ', 0],
    ['u_scale', 'size', 1.35], ['u_roughness', 'roughness', .25], ['u_metalness', 'metalness', .7],
  ] as any) {
    let value = Number(extensionParam(L, definition, parameter, T, fallback)) || 0;
    if (parameter === 'rotationY' && extensionParam(L, definition, 'autoRotate', T, false)) {
      value += (T - L.from) * Number(extensionParam(L, definition, 'speed', T, 25));
    }
    setU(p, uniform, [value]);
  }
  setU(p, 'u_objectColor', PM.hex2rgb(String(extensionParam(L, definition, 'objectColor', T, '#C7C4FF'))));
  setU(p, 'u_lightColor', PM.hex2rgb(String(extensionParam(L, definition, 'lightColor', T, '#FFB36B'))));
  gl.drawArrays(gl.TRIANGLES, 0, buffers.count); GL.stats.draws++; GL.stats.passes++;
  gl.disableVertexAttribArray(position); gl.disableVertexAttribArray(normal);
  gl.disable(gl.DEPTH_TEST); gl.enable(gl.BLEND);
  bind(target);
  return { tex: f.tex, w, h, ax: 0, ay: 0, uv: [0, 0, 1, 1], fromFbo: true, tmp: f };
}

function contentQuad(L: any, T: any, W: any, H: any) {
  /* returns {tex, w, h, ax, ay, uv:[ox,oy,sx,sy], fromFbo, solid, tmp} */
  const d = L.d;
  if (L.type === 'solid') {
    return { solid: PM.hex2rgb(d.color), w: d.w || W, h: d.h || H, ax: 0, ay: 0 };
  }
  if (L.type === 'text' || L.type === 'shape') {
    /* Render editable text/shape source at the density it occupies in this
       output. This is continuous rasterization: no fixed-resolution layer
       bitmap is enlarged when the layer is scaled, parented, or previewed at
       a different output resolution. */
    const ss = continuousRasterScale(scaledWorld(L, T, W, H));
    const r = PM.raster(L, ss, T);
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
    const videoVersion = L.type === 'video' ? videoTextureVersion(el) : 1;
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
  if (L.type === 'extension') {
    const comp = PM.curComp?.() || PM.proj;
    const w = Math.max(1, Number(d.w || comp.w || W)), hh = Math.max(1, Number(d.h || comp.h || H));
    const [sx, sy] = outputScale(W, H);
    const targetW = Math.min(4096, Math.max(2, Math.round(w * sx)));
    const targetH = Math.min(4096, Math.max(2, Math.round(hh * sy)));
    const definition: any = PM.layerDefinition?.(d.definition);
    if (definition?.renderer?.kind === 'mesh') {
      const mesh = meshExtensionQuad(L, T, w, hh, targetW, targetH, definition);
      if (mesh !== undefined) return mesh;
    }
    const missing = `void main(){
      vec2 q=floor(uv*24.0); float checker=mod(q.x+q.y,2.0);
      vec3 a=vec3(.055,.05,.07), b=vec3(.16,.09,.15);
      fragColor=vec4(mix(a,b,checker),1.0);
    }`;
    const source = definition?.renderer?.kind === 'fragment'
      ? extensionLayerFragment(definition, PM.SHADER_HEADER)
      : PM.SHADER_HEADER + '\n' + missing;
    const key = `extension:${String(d.definition || 'missing')}:${definition?.version || 0}:${hashStr(source)}`;
    const p = program(key, source, PM.VERT);
    PM.UIState?.setShaderMeta?.(L, {
      shaderKey: key,
      definition,
      missing: !definition || definition.renderer?.kind !== 'fragment',
      missingAsset: definition?.renderer?.kind === 'mesh' ? String(d.data?.[definition.renderer.assetField] || '') : null,
    });
    if (!p) return null;
    const target = boundFbo;
    const f = grab(targetW, targetH);
    bind(f); clear(0, 0, 0, 0);
    const g = use(p);
    g.u('u_m', fullQuad(f.w, f.h)); g.u('u_res', f.w, f.h); g.u('u_uv', 0, 0, 1, 1);
    setU(p, 'iResolution', [f.w, f.h]);
    setU(p, 'iTime', [T - L.from]); setU(p, 'iGlobalTime', [T]);
    setU(p, 'iProgress', [PM.clamp((T - L.from) / Math.max(L.dur, 1e-4), 0, 1)]);
    setI(p, 'iFrame', Math.round(T * PM.proj.fps)); setU(p, 'iMouse', [0, 0]);
    for (const param of definition?.params || []) {
      const property = d.params?.[param.k];
      const value = property ? PM.evP(L, property, T, param.k) : param.def;
      if (param.type === 'color') setU(p, 'u_' + param.k, PM.hex2rgb(String(value)));
      else if (param.type === 'toggle') setI(p, 'u_' + param.k, value ? 1 : 0);
      else setU(p, 'u_' + param.k, [Number(value) || 0]);
    }
    GL.gl.disable(GL.gl.BLEND); draw(); GL.gl.enable(GL.gl.BLEND);
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
    /* setU receives its uniform arguments as a list; keep the matrix wrapped so
       it selects uniformMatrix3fv rather than treating nine scalars as vec4. */
    setU(p, 'u_inv', [m3(inv)]);
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

/** Apply an adjustment layer to the composition accumulated beneath it.
 * The layer contributes no pixels of its own: the normal editable effect chain
 * processes the full lower image, masks constrain the processed result, and
 * opacity/blend determine how strongly it replaces the untouched original. */
function compositeAdjustment(
  L: any, T: any, acc: any, W: any, H: any,
  alpha: number, hasMasks: boolean, blend: number,
) {
  const adjusted = runEffects(L, T, acc, W, H);
  /* A missing or failed effect program must leave the composition unchanged,
     including transparent nested compositions. */
  if (adjusted === acc) return acc;

  /* Adjustment masks gate the difference between adjusted and original. Apply
     them after effects so transparent pixels outside the mask never become
     effect input and so spatial effects can sample the real lower stack. */
  const contribution = hasMasks ? applyMasks(L, T, adjusted, W, H) : adjusted;

  const out = grab(W, H);
  bind(out); clear();
  const p = program('adjustment', PM.FRAG_COMPOSITE);
  if (!p) {
    free(out);
    if (contribution !== adjusted) free(contribution);
    free(adjusted);
    return acc;
  }
  const g = use(p);
  bindTex(0, contribution.tex); setI(p, 'u_tex', 0);
  bindTex(1, acc.tex); setI(p, 'u_dst', 1);
  g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
  g.u('u_alpha', alpha); setI(p, 'u_blend', blend);
  GL.gl.disable(GL.gl.BLEND); draw(); GL.gl.enable(GL.gl.BLEND);
  GL.stats.passes++;

  if (contribution !== adjusted) free(contribution);
  free(adjusted);
  free(acc);
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

    /* Adjustment layers are full-frame processors over the already-rendered
       stack below. An adjustment without an enabled renderable effect is a
       transparent no-op, matching its lack of source pixels. */
    if (L.type === 'adjustment') {
      if (hasFx) acc = compositeAdjustment(L, T, acc, W, H, alpha, hasMasks, blend);
      continue;
    }

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
  if (L.type === 'solid' || L.type === 'shader' || L.type === 'extension') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'precomp') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'text') {
    const r = PM.raster(L, 1, T);
    if (r.selection) return { ...r.selection, ax: r.anchorX, ay: r.anchorY };
    w = r.w; h = r.h; ax = r.anchorX; ay = r.anchorY;
  }
  else if (L.type === 'shape') {
    const r = PM.raster(L, 1);
    if (r.selection) return { ...r.selection, ax: r.w / 2, ay: r.h / 2 };
    w = r.w; h = r.h;
    ax = w / 2; ay = h / 2;
  } else if (L.type === 'image' || L.type === 'video') {
    w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = w / 2; ay = h / 2;
  } else return null;
  return { x0: -ax, y0: -ay, x1: w - ax, y1: h - ay, w, h, ax, ay };
};
}
