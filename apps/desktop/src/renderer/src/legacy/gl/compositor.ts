import { GPUTiming } from './gpu-timing';
import { performanceMonitor } from '../../runtime/performance-monitor';
import { is3DLayer, planeMatrix, planeContains, depthOrderedLayers, inversePlane } from '../core/space-3d';
import { pathValues, rasterPathsToViewport, tracePath } from '../core/vector-paths';
import { createPreviewWarmup } from './preview-warmup';
import { sourceTime } from '../core/retiming';
import { evaluatedValue, isProperty, resolveContent } from '../core/content-properties';
/* Ported from js/gl/compositor.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { extensionLayerFragment } from '../../kernel/extension-layers';
import { uncoveredRasterRegions, previewShapeRaster, rasterIntersectsViewport, shapeRasterGeometry, type RasterWindow } from './shape-raster-window';

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
export function hasRenderableEffects(effects: Array<{ on?: boolean; missing?: boolean }>, PM?: any, layer?: any, time?: number): boolean {
  return effects.some((effect: any) => (PM ? evaluatedValue(PM, layer, effect.on, time!, `${effect.id}.$enabled`) : effect.on) && effect.missing !== true);
}

/** Groups are composited as nested stacks. Only direct members belong to a
 * pass; nested groups recursively own their descendants. */
export function compositingLevel(layers: any[], parentGroup: string | null): any[] {
  return layers.filter((layer: any) => (layer.group || null) === parentGroup);
}

/** A group remains in a solo render when the solo switch lives on any nested
 * member. Without this, the containing pass would be skipped before reaching
 * that member. */
export function groupContainsSolo(group: any, layers: any[], ancestors: (layer: any) => any[]): boolean {
  return group?.type === 'group' && layers.some((layer: any) => layer.solo
    && ancestors(layer).some((candidate: any) => candidate.id === group.id));
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
  matrix: readonly number[], density = 1, maxScale = 32,
): number {
  const m0 = Number(matrix?.[0]) || 0, m1 = Number(matrix?.[1]) || 0;
  const m2 = Number(matrix?.[2]) || 0, m3 = Number(matrix?.[3]) || 0;
  const aa = m0 * m0 + m1 * m1;
  const bb = m0 * m2 + m1 * m3;
  const cc = m2 * m2 + m3 * m3;
  const discriminant = Math.sqrt(Math.max(0, (aa - cc) ** 2 + 4 * bb * bb));
  const largest = Math.sqrt(Math.max(0, (aa + cc + discriminant) / 2));
  const requested = largest * Math.max(.01, Number(density) || 1);
  const bounded = Math.max(.25, Math.min(Math.max(.25, Number(maxScale) || 32), requested));
  return Math.ceil(bounded * 4 - 1e-9) / 4;
}

/** Backing dimensions for a vector asset at its current display density.
 * The texture retains the SVG's source aspect ratio so existing cover/contain
 * UV behavior stays identical to raster images. */
export function svgRasterDimensions(
  sourceWidth: number,
  sourceHeight: number,
  boxWidth: number,
  boxHeight: number,
  matrix: readonly number[],
  maxDimension = 8192,
): { width: number; height: number; scale: number } {
  const sw = Math.max(1, Number(sourceWidth) || 1);
  const sh = Math.max(1, Number(sourceHeight) || 1);
  const bw = Math.max(1, Number(boxWidth) || sw);
  const bh = Math.max(1, Number(boxHeight) || sh);
  const limit = Math.max(1, Number(maxDimension) || 8192);
  const boxDensity = Math.max(bw / sw, bh / sh);
  const maxScale = Math.max(.25, limit / Math.max(sw, sh));
  const requestedScale = continuousRasterScale(matrix, boxDensity, maxScale);
  let width = Math.max(1, Math.ceil(sw * requestedScale));
  let height = Math.max(1, Math.ceil(sh * requestedScale));
  if (Math.max(width, height) > limit) {
    const ratio = limit / Math.max(width, height);
    width = Math.max(1, Math.floor(width * ratio));
    height = Math.max(1, Math.floor(height * ratio));
  }
  return { width, height, scale: width / sw };
}

export function install(PM: PMRegistry): void {

const GL: any = {
  gl: null, canvas: null, w: 0, h: 0,
  previewViewport: null,
  progs: new Map(), texes: new Map(), meshes: new Map(), pool: [], quad: null,
  stats: { draws: 0, passes: 0, ms: 0, progs: 0 },
  errors: new Map(),
};
PM.GL = GL;
const MAX_FBO_BYTES = PM.Memory?.budget?.('framebuffers') || 384 * 1024 * 1024;
const MAX_TEXTURE_BYTES = PM.Memory?.budget?.('textures') || 192 * 1024 * 1024;
const MAX_TEXTURE_ENTRIES = 4096;
let poolBytes = 0;
let textureBytes = 0;
let resourceTick = 0;
let sourceGeneration = 0;
/* Keep the composited FBO alive until the next frame. Resizing a canvas clears
   its drawing buffer synchronously; retaining this texture lets resize()
   present the last complete image at the new size instead of exposing black
   while the editor's next animation frame is still queued. */
let presentedFrame: any = null;
const presentedVideoFrames = new WeakMap<any, { version: number; supported: boolean }>();
let gpuTiming: GPUTiming | null = null;
const framePrograms = new Set<any>();
let parallelShaderCompile: any = null;
let allowParallelCompile = false;
let compileSubmitMs = 0;
const pendingPrograms = new Map<any, { pr: any; v: any; f: any }>();
let compilePollTimer: ReturnType<typeof setTimeout> | undefined;
const watchedCanvases = new WeakSet<HTMLCanvasElement>();
function pollCompiles() {
  if (compilePollTimer !== undefined || !pendingPrograms.size) return;
  compilePollTimer = setTimeout(() => {
    compilePollTimer = undefined;
    for (const [key, pending] of pendingPrograms) {
      if (GL.gl.getProgramParameter(pending.pr, parallelShaderCompile.COMPLETION_STATUS_KHR)) completeProgram(key, pending);
    }
    GL.compiling = pendingPrograms.size;
    PM.invalidate('render'); PM.invalidate('status');
    pollCompiles();
  }, 16);
}
function completeProgram(key: any, pending: { pr: any; v: any; f: any }) {
  const gl = GL.gl;
  let result: any = null;
  if (gl.getProgramParameter(pending.pr, gl.LINK_STATUS)) {
    result = { pr: pending.pr, u: new Map(), a: gl.getAttribLocation(pending.pr, 'a_pos') };
    GL.errors.delete(key);
  } else {
    GL.errors.set(key, gl.getProgramInfoLog(pending.pr) || gl.getShaderInfoLog(pending.f) || 'Shader compilation failed');
    gl.deleteProgram(pending.pr);
  }
  gl.deleteShader(pending.v); gl.deleteShader(pending.f);
  pendingPrograms.delete(key); GL.progs.set(key, result); GL.compiling = pendingPrograms.size;
  return result;
}
const viewportPathRasters = new Map<string, any>();
let viewportPathFrame = 0;
let viewportPathVersion = 0;

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
  framePrograms.add(key);
  const gl = GL.gl;
  const pending = pendingPrograms.get(key);
  if (pending) {
    if (allowParallelCompile && !gl.getProgramParameter(pending.pr, parallelShaderCompile.COMPLETION_STATUS_KHR)) return null;
    return completeProgram(key, pending);
  }
  let p = GL.progs.get(key);
  if (p !== undefined) { GL.progs.delete(key); GL.progs.set(key, p); return p; }
  if (allowParallelCompile && parallelShaderCompile && /^(sh:|extension:|fx:|tr:)/.test(key)) {
    if (compileSubmitMs >= 4 || pendingPrograms.size >= 8) { pollCompiles(); return null; }
    const started = performance.now();
    const v = gl.createShader(gl.VERTEX_SHADER), f = gl.createShader(gl.FRAGMENT_SHADER), pr = gl.createProgram();
    if (!v || !f || !pr) {
      if (v) gl.deleteShader(v); if (f) gl.deleteShader(f); if (pr) gl.deleteProgram(pr);
      GL.errors.set(key, 'GPU resources unavailable'); GL.progs.set(key, null); return null;
    }
    // Do not ask for compile/link status here: those queries wait for the GPU
    // process and can freeze the editor for hundreds of milliseconds.
    gl.shaderSource(v, vert || PM.VERT); gl.compileShader(v);
    gl.shaderSource(f, frag); gl.compileShader(f);
    gl.attachShader(pr, v); gl.attachShader(pr, f); gl.linkProgram(pr);
    pendingPrograms.set(key, { pr, v, f }); GL.compiling = pendingPrograms.size;
    performanceMonitor.record({ id: `compile:${key}`, name: `Shader compilation · ${key}`, kind: 'shader' }, performance.now() - started);
    compileSubmitMs += performance.now() - started;
    pollCompiles(); PM.invalidate('status'); return null;
  }
  let v: any = null, f: any = null, pr: any = null;
  const started = performance.now();
  try {
    v = shader(gl, gl.VERTEX_SHADER, vert || PM.VERT);
    f = shader(gl, gl.FRAGMENT_SHADER, frag);
    pr = gl.createProgram();
    gl.attachShader(pr, v); gl.attachShader(pr, f); gl.linkProgram(pr);
    if (!gl.getProgramParameter(pr, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(pr));
    p = { pr, u: new Map(), a: gl.getAttribLocation(pr, 'a_pos') };
    GL.errors.delete(key);
  } catch (e: any) {
    GL.errors.set(key, String(e.message || e).trim());
    if (pr) gl.deleteProgram(pr);
    p = null;
  } finally {
    if (v) gl.deleteShader(v); if (f) gl.deleteShader(f);
    if (/^(sh:|extension:|fx:)/.test(key)) performanceMonitor.record({ id: `compile:${key}`, name: `Shader compilation · ${key}`, kind: 'shader' }, performance.now() - started);
  }
  GL.progs.set(key, p);
  GL.stats.progs = GL.progs.size;
  return p;
}
GL.compileError = (key: any) => GL.errors.get(key) || null;
GL.dropProgram = (key: any) => {
  const pending = pendingPrograms.get(key);
  if (pending) {
    GL.gl.deleteProgram(pending.pr); GL.gl.deleteShader(pending.v); GL.gl.deleteShader(pending.f);
    pendingPrograms.delete(key); GL.compiling = pendingPrograms.size;
  }
  const p = GL.progs.get(key);
  if (p && p.pr) GL.gl.deleteProgram(p.pr);
  GL.progs.delete(key); GL.errors.delete(key);
};
GL.dropPrograms = (prefix: string) => {
  for (const key of new Set([...GL.progs.keys(), ...pendingPrograms.keys()])) if (String(key).startsWith(prefix)) GL.dropProgram(key);
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
let previewViewportActive = false;
let previewSourceClipping = false;
function activePreviewViewport(W: number, H: number): any {
  return previewViewportActive && PM.curComp?.() === PM.proj
    && W === GL.canvas.width && H === GL.canvas.height ? GL.previewViewport : null;
}
function canClipPreviewSources(W: number, H: number): boolean {
  // A fitted composition has a full-frame viewport even when the viewer does
  // not need a zoom crop. Large parented artwork must still be clipped there.
  return previewSourceClipping && PM.curComp?.() === PM.proj
    && W === GL.canvas.width && H === GL.canvas.height;
}
function outputScale(W: number, H: number): [number, number] {
  const viewport = activePreviewViewport(W, H);
  if (viewport) return [W / Math.max(1e-6, viewport.width), H / Math.max(1e-6, viewport.height)];
  const comp = PM.curComp?.() || PM.proj;
  return [W / Math.max(1, Number(comp?.w) || W), H / Math.max(1, Number(comp?.h) || H)];
}
function scaledWorld(layer: any, time: any, W: number, H: number): [number, number, number, number, number, number] {
  const world = PM.worldMatrix(layer, time);
  const [sx, sy] = outputScale(W, H);
  const viewport = activePreviewViewport(W, H);
  return [world[0] * sx, world[1] * sy, world[2] * sx, world[3] * sy, (world[4] - (viewport?.x || 0)) * sx, (world[5] - (viewport?.y || 0)) * sy];
}

function backgroundView(W: number, H: number): [number, number, number, number] {
  const viewport = activePreviewViewport(W, H);
  return viewport
    ? [viewport.x / viewport.compWidth, viewport.y / viewport.compHeight, viewport.width / viewport.compWidth, viewport.height / viewport.compHeight]
    : [0, 0, 1, 1];
}

/* ── content textures ──────────────────────────────────── */
const visibleTextures = new Set<string>();
let preparingSource = false;
function texFor(key: any, source: any, opts: any = {}) {
  if (!preparingSource) visibleTextures.add(key);
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
  PM.Memory?.maintain?.('textures', GL.texes.size > MAX_TEXTURE_ENTRIES);
  return t.tex;
}
function trimTextures(targetBytes: number = MAX_TEXTURE_BYTES, protectedKeys?: ReadonlySet<string>) {
  if (textureBytes <= targetBytes && GL.texes.size <= MAX_TEXTURE_ENTRIES) return;
  const entries = [...GL.texes.entries()].sort((a: any, b: any) => a[1].used - b[1].used);
  for (const [key, entry] of entries) {
    if (textureBytes <= targetBytes && GL.texes.size <= MAX_TEXTURE_ENTRIES || GL.texes.size <= 1) break;
    if (boundTex.includes(entry.tex) || protectedKeys?.has(key)) continue;
    GL.gl?.deleteTexture?.(entry.tex);
    textureBytes = Math.max(0, textureBytes - (entry.bytes || 0));
    GL.texes.delete(key);
  }
}
GL.dropTextures = (prefix = '') => {
  sourceGeneration++;
  for (const [key, entry] of GL.texes) {
    if (!key.startsWith(prefix)) continue;
    if (GL.gl && entry.tex) GL.gl.deleteTexture(entry.tex);
    textureBytes = Math.max(0, textureBytes - (entry.bytes || 0));
    GL.texes.delete(key);
  }
};

function trimViewportPathRasters() {
  for (const [id, entry] of viewportPathRasters) {
    if (entry.frame >= viewportPathFrame - 1) continue;
    entry.cv.width = 0; entry.cv.height = 0;
    viewportPathRasters.delete(id);
    GL.dropTextures('viewport-path:' + id);
  }
}

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
GL.init = (canvas: any, options: { alpha?: boolean; quiet?: boolean } = {}) => {
  GL.canvas = canvas;
  const gl = canvas.getContext('webgl2', {
    alpha: options.alpha === true, antialias: false, premultipliedAlpha: true,
    preserveDrawingBuffer: true, powerPreference: 'high-performance', desynchronized: true,
  });
  if (!gl) { if (!options.quiet) window.alert('Powermove needs WebGL2.'); return false; }
  gpuTiming?.dispose();
  GL.gl = gl;
  sourceGeneration++;
  gpuTiming = new GPUTiming(gl);
  parallelShaderCompile = gl.getExtension('KHR_parallel_shader_compile');
  if (!watchedCanvases.has(canvas)) {
    watchedCanvases.add(canvas);
    canvas.addEventListener('webglcontextlost', (event: Event) => {
      event.preventDefault();
      clearTimeout(compilePollTimer); compilePollTimer = undefined;
      pendingPrograms.clear(); GL.compiling = 0;
      gpuTiming?.dispose(); gpuTiming = null;
      GL.progs.clear(); GL.texes.clear(); GL.meshes.clear(); GL.pool = [];
      GL.errors.clear(); viewportPathRasters.clear();
      poolBytes = 0; textureBytes = 0; presentedFrame = null; boundFbo = null;
      GL.gl = null; GL.contextLost = true;
      PM.invalidate('status');
      PM.toast?.('The GPU was interrupted. Restoring the preview…');
    });
    canvas.addEventListener('webglcontextrestored', () => {
      if (GL.init(canvas, options)) { GL.contextLost = false; PM.invalidate(); }
    });
  }
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

GL.resize = (w: any, h: any, previewViewport: any = null) => {
  GL.previewViewport = previewViewport;
  if (GL.canvas.width === w && GL.canvas.height === h) return false;
  const previous = presentedFrame;
  /* All pooled targets except the retained presentation are invalid at the
     new output size. Dispose them before allocating another full frame. */
  for (let index = GL.pool.length - 1; index >= 0; index--) {
    const f = GL.pool[index];
    if (f === previous) continue;
    GL.pool.splice(index, 1);
    disposeFbo(f);
  }
  GL.canvas.width = w; GL.canvas.height = h;
  if (previous) {
    /* The retained texture is a fully composited frame, so the same present
       shader can scale it into the freshly-created default drawing buffer. */
    bind(null);
    const p = program('present', PM.FRAG_COPY);
    const g = use(p);
    bindTex(0, previous.tex); setI(p, 'u_tex', 0);
    g.u('u_m', fullQuad(w, h)); g.u('u_res', w, h); g.u('u_uv', 0, 0, 1, 1);
    GL.gl.disable(GL.gl.BLEND); draw(); GL.gl.enable(GL.gl.BLEND);
    bindTex(0, null);
    /* Keep it until a newly rendered frame replaces it. A wheel/trackpad
       gesture can deliver several resize events before the next rAF. */
  }
  return true;
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

function contentQuad(L: any, T: any, W: any, H: any, clip?: RasterWindow) {
  /* returns {tex, w, h, ax, ay, uv:[ox,oy,sx,sy], fromFbo, solid, tmp} */
  const d = resolveContent(PM, L, T);
  if (L.type === 'solid') {
    return { solid: PM.hex2rgb(d.color), w: d.w || W, h: d.h || H, ax: 0, ay: 0 };
  }
  if (L.type === 'shape' && L.d.paths?.length && activePreviewViewport(W, H)
      && (PM.previewResolution === '1' || PM.perf?.auto === false && PM.quality === 1)) {
    const world = scaledWorld(L, T, W, H);
    const previous = viewportPathRasters.get(L.id);
    const raster = rasterPathsToViewport(PM, L, T, W, H, world, previous);
    if (raster !== previous) {
      if (previous?.cv) { previous.cv.width = 0; previous.cv.height = 0; }
      raster.version = ++viewportPathVersion;
      viewportPathRasters.set(L.id, raster);
    }
    raster.frame = viewportPathFrame;
    GL.stats.viewportVectors = (GL.stats.viewportVectors || 0) + 1;
    const tex = texFor('viewport-path:' + L.id, raster.cv, { version: raster.version });
    return { tex, w: W, h: H, ax: 0, ay: 0, uv: [0, 0, 1, 1], screenSpace: true };
  }
  if (L.type === 'text' || L.type === 'shape') {
    /* Render editable text/shape source at the density it occupies in this
       output. This is continuous rasterization: no fixed-resolution layer
       bitmap is enlarged when the layer is scaled, parented, or previewed at
       a different output resolution. */
    const ss = continuousRasterScale(scaledWorld(L, T, W, H));
    let crop: RasterWindow | undefined;
    // Group effects sample the already-composited W×H group target. Pixels
    // outside that target are discarded before the effect, so its ordinary
    // children can still use clipped sources without changing the effect.
    if (canClipPreviewSources(W, H) && !L.d.paths?.length
        && !is3DLayer(PM, L) && !hasRenderableEffects(L.fx, PM, L, T)
        && !L.masks?.length && !L.matteSource && !L.transitionIn && !L.transitionOut) {
      const visibleWorld = scaledWorld(L, T, W, H);
      if (clip) { visibleWorld[4] -= clip.x; visibleWorld[5] -= clip.y; }
      if (L.type === 'shape') {
        const plan = previewShapeRaster(d, ss, visibleWorld, clip?.width ?? W, clip?.height ?? H);
        if (plan.kind === 'outside') return null;
        if (plan.kind === 'solid') return { solid: PM.hex2rgb(d.color), w: W, h: H, ax: 0, ay: 0, screenSpace: true };
        if (plan.kind === 'crop') crop = plan.window;
      } else if (!L.d.animators?.length && !L.d.styles?.length && !L.d.fontAnchorBounds
          && !rasterIntersectsViewport(PM.textRasterGeometry(L, ss, T), visibleWorld, clip?.width ?? W, clip?.height ?? H)) {
        // Keep visible text on the original full-bitmap path: cropping a glyph
        // can change Canvas antialiasing even with an integer pixel offset.
        return null;
      }
    }
    // CPU bitmaps and uploaded textures have independent memory budgets. A
    // bitmap evicted under CPU pressure need not be drawn and uploaded again
    // while its identical GPU source is still available.
    const r = PM.raster(L, ss, T, (key: string) => GL.texes.get('r:' + key)?.raster, crop);
    const tex = texFor('r:' + r.key, r.cv, { version: 1 });
    const uploaded = GL.texes.get('r:' + r.key);
    if (!uploaded.raster && !r.fontOffset) {
      const { cv: _canvas, ...metadata } = r;
      uploaded.raster = metadata;
    }
    const ax = L.type === 'shape' && !L.d.paths?.length ? r.w / 2 : r.anchorX;
    const ay = L.type === 'shape' && !L.d.paths?.length ? r.h / 2 : r.anchorY;
    return { tex, w: r.w, h: r.h, ax, ay, uv: r.uv || [0, 0, 1, 1] };
  }
  if (L.type === 'image' || L.type === 'video') {
    const a = PM.assets.get(d.asset);
    if (!a) return null;
    let el = PM.preparedVideoFrames?.get(L.id+'@'+T) || a.el, sw = a.w || 1, sh = a.h || 1;
    if (L.type === 'video') {
      const vt = PM.clamp(sourceTime(PM,L,T), 0, Math.max(0, a.dur - .04));
      if (!PM.playing && Math.abs(el.currentTime - vt) > .02) { try { el.currentTime = vt; } catch (e) { } }
      sw = el.videoWidth || sw; sh = el.videoHeight || sh;
    }
    const bw = d.w || W, bh = d.h || H;
    let textureSource = el;
    let textureKey = 'a:' + a.id + (el===a.el?'':':'+L.id+'@'+T);
    if (L.type === 'image' && a.format === 'svg' && PM.rasterSvgAsset) {
      const dimensions = svgRasterDimensions(sw, sh, bw, bh, scaledWorld(L, T, W, H));
      const raster = PM.rasterSvgAsset(a, dimensions.width, dimensions.height);
      textureSource = raster.cv;
      textureKey = 'r:' + raster.key;
    }
    const videoVersion = L.type === 'video' ? (el===a.el?videoTextureVersion(el):PM.preparedVideoVersion) : 1;
    const tex = texFor(textureKey, textureSource, { version: videoVersion });
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
    gpuTiming ? gpuTiming.measure({ id: `shader:${L.id}`, name: L.name || 'Custom shader', kind: 'shader', layerId: L.id }, draw) : draw();
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
    GL.gl.disable(GL.gl.BLEND);
    gpuTiming ? gpuTiming.measure({ id: `shader:${L.id}`, name: L.name || definition?.label || 'Extension shader', kind: 'shader', layerId: L.id }, draw) : draw();
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
      const inner = GL.renderProject(sub, sourceTime(PM,L,T), targetW, targetH, { transparent: true });
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
function drawContent(L: any, T: any, W: any, H: any, alpha: any, clip?: RasterWindow) {
  const c: any = contentQuad(L, T, W, H, clip);
  if (!c) return false;
  const world = scaledWorld(L, T, W, H);
  const M = c.screenSpace ? [W, 0, 0, H, 0, 0] : PM.mul(world, [c.w, 0, 0, c.h, -c.ax, -c.ay]);
  let projected = m3(M);
  if (is3DLayer(PM,L)) {
    const h = planeMatrix(PM, L, T), [sx, sy] = outputScale(W, H);
    projected = new Float32Array([
      h[0]*c.w*sx,h[1]*c.w*sy,h[2]*c.w,
      h[3]*c.h*sx,h[4]*c.h*sy,h[5]*c.h,
      (h[6]-h[0]*c.ax-h[3]*c.ay)*sx,(h[7]-h[1]*c.ax-h[4]*c.ay)*sy,h[8]-h[2]*c.ax-h[5]*c.ay
    ]);
  }
  if (c.solid) {
    const p = program('solid', PM.FRAG_SOLID);
    if (!p) return false;
    const g = use(p);
    g.u('u_m', projected); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
    g.u('u_color', c.solid[0] * alpha, c.solid[1] * alpha, c.solid[2] * alpha, alpha);
    draw();
    return true;
  }
  const p = program('draw', PM.FRAG_DRAW);
  if (!p) return false;
  const g = use(p);
  bindTex(0, c.tex); setI(p, 'u_tex', 0);
  g.u('u_m', projected); g.u('u_res', W, H);
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
    if (!evaluatedValue(PM, L, fx.on, T, `${fx.id}.$enabled`) || fx.missing) continue;
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
      gpuTiming ? gpuTiming.measure({ id: `effect:${L.id}:${fx.id}:${pass}`, name: `${L.name} · ${def.label || fx.type}`, kind: 'effect', layerId: L.id }, draw) : draw();
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
function applyPathMasks(L:any,T:number,srcF:any,W:number,H:number,masks:any[]) {
  const cv=document.createElement('canvas');cv.width=W;cv.height=H;const ctx=cv.getContext('2d')!;
  const mode=(m:any)=>evaluatedValue(PM,L,m.mode,T,`m.${m.id}.mode`);
  if(!masks.some(m=>mode(m)!=='subtract')){ctx.fillStyle='white';ctx.fillRect(0,0,W,H);}
  const world=scaledWorld(L,T,W,H);
  for(const m of masks){const item=document.createElement('canvas');item.width=W;item.height=H;const c=item.getContext('2d')!;c.transform(...world);c.fillStyle='white';
    const ev=(key:string)=>Number(PM.evP(L,m.p[key],T,`m.${m.id}.${key}`))||0;
    c.translate(ev('x'),ev('y'));c.rotate(ev('rotation')*Math.PI/180);
    if(m.path){const v=pathValues(PM,L,m.path,T,`mp.${m.id}`);c.translate(v.x,v.y);c.rotate(v.rotation*Math.PI/180);c.scale(v.scaleX/100,v.scaleY/100);tracePath(c,{...v,closed:true,trimStart:0,trimEnd:100});}
    else{c.beginPath();if(evaluatedValue(PM,L,m.shape,T,`m.${m.id}.shape`)==='ellipse')c.ellipse(0,0,Math.abs(ev('w'))/2,Math.abs(ev('h'))/2,0,0,Math.PI*2);else c.rect(-ev('w')/2,-ev('h')/2,ev('w'),ev('h'));}
    c.fill();ctx.globalCompositeOperation=mode(m)==='subtract'?'destination-out':'source-over';ctx.filter=`blur(${Math.max(0,ev('feather'))*Math.hypot(world[0],world[1])/2}px)`;ctx.drawImage(item,0,0);
  }
  const out=grab(W,H);bind(out);clear();const p=program('pathMaskApply',PM.GLSL_PRE+'uniform sampler2D u_cov; uniform mat3 u_covTransform; void main(){vec3 q=u_covTransform*vec3(v_px,1.0);vec2 uv=q.xy/q.z/u_res;float coverage=(uv.x<0.0||uv.y<0.0||uv.x>1.0||uv.y>1.0)?0.0:texture(u_cov,uv).a;o=texture(u_tex,v_st)*coverage;}');
  const g=use(p);
  let coverageTransform = IDENT;
  if (is3DLayer(PM,L)) {
    const h=planeMatrix(PM,L,T),[sx,sy]=outputScale(W,H);
    const inv=inversePlane([h[0]*sx,h[1]*sy,h[2],h[3]*sx,h[4]*sy,h[5],h[6]*sx,h[7]*sy,h[8]]);
    if (inv) coverageTransform=new Float32Array([0,1,2].flatMap(i=>{
      const x=inv[i*3]!,y=inv[i*3+1]!,w=inv[i*3+2]!;
      return [world[0]*x+world[2]*y+world[4]*w,world[1]*x+world[3]*y+world[5]*w,w];
    }));
  }
  g.u('u_covTransform',coverageTransform);
  // Uploading a canvas binds texture unit 0. Complete the upload before
  // binding the source, or u_tex samples the white coverage instead of video.
  const coverage = texFor('path-mask:'+L.id,cv);
  bindTex(0,srcF.tex);setI(p,'u_tex',0);bindTex(1,coverage);setI(p,'u_cov',1);g.u('u_m',fullQuad(W,H));g.u('u_res',W,H);g.u('u_uv',0,0,1,1);GL.gl.disable(GL.gl.BLEND);draw();GL.gl.enable(GL.gl.BLEND);return out;
}
function applyTrackMatte(L:any,T:number,srcF:any,W:number,H:number,proj:any,opt:any) {
  const source=proj.layers.find((l:any)=>l.id===L.matteSource);
  const mode=evaluatedValue(PM,L,L.matteMode,T,'l.matteMode')||'alpha';
  if(!source || (opt.matteDepth||0)>=8)return srcF;
  const matte=GL.renderProject({...proj,layers:[{...source,on:true}]},T,W,H,{...opt,transparent:true,mattePass:true,explicitLayers:true,matteDepth:(opt.matteDepth||0)+1,matteProject:proj});
  const out=grab(W,H);bind(out);clear();const p=program('trackMatte',PM.GLSL_PRE+'uniform sampler2D u_matte; uniform int u_luma; uniform int u_invert; void main(){vec4 m=texture(u_matte,v_st);float a=u_luma==1?dot(m.rgb,vec3(.2126,.7152,.0722)):m.a;if(u_invert==1)a=1.-a;o=texture(u_tex,v_st)*clamp(a,0.,1.);}');
  const g=use(p);bindTex(0,srcF.tex);setI(p,'u_tex',0);bindTex(1,matte.tex);setI(p,'u_matte',1);setI(p,'u_luma',mode.startsWith('luma')?1:0);setI(p,'u_invert',mode.endsWith('inverted')?1:0);g.u('u_m',fullQuad(W,H));g.u('u_res',W,H);g.u('u_uv',0,0,1,1);GL.gl.disable(GL.gl.BLEND);draw();GL.gl.enable(GL.gl.BLEND);free(matte);return out;
}

const MASK_MAX = 8;
function applyMasks(L: any, T: any, srcF: any, W: any, H: any) {
  const gl = GL.gl;
  const masks = (L.masks || []).filter((m: any) => m && evaluatedValue(PM, L, m.on, T, `m.${m.id}.on`) !== false && m.p);
  if (!masks.length) return srcF;
  if(masks.some((m:any)=>m.path))return applyPathMasks(L,T,srcF,W,H,masks);
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
    const shape = isProperty(m.shape) ? PM.evP(L, m.shape, T, `m.${m.id}.shape`) : m.shape;
    const mode = isProperty(m.mode) ? PM.evP(L, m.mode, T, `m.${m.id}.mode`) : m.mode;
    qArr[q + 2] = shape === 'ellipse' ? 1 : 0;
    qArr[q + 3] = mode === 'subtract' ? 1 : 0;
    if (mode !== 'subtract') hasAdd = 1;
    cnt++;
  }
  if (cnt === 0) { free(cov); return srcF; }
  {
    const pm = use(p);
    pm.u('u_m', fullQuad(W, H)); pm.u('u_res', W, H); pm.u('u_uv', 0, 0, 1, 1);
    /* setU receives its uniform arguments as a list; keep the matrix wrapped so
       it selects uniformMatrix3fv rather than treating nine scalars as vec4. */
    let maskInverse = m3(inv);
    if (is3DLayer(PM,L)) {
      const h = planeMatrix(PM,L,T), [sx,sy] = outputScale(W,H);
      const projectedInverse = inversePlane([h[0]*sx,h[1]*sy,h[2],h[3]*sx,h[4]*sy,h[5],h[6]*sx,h[7]*sy,h[8]]);
      if (projectedInverse) maskInverse = new Float32Array(projectedInverse);
    }
    setU(p, 'u_inv', [maskInverse]);
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

function drawFbo(srcF: any, W: any, H: any, alpha = 1) {
  const p = program('copyA', PM.FRAG_DRAW);
  if (!p) return false;
  const g = use(p);
  bindTex(0, srcF.tex); setI(p, 'u_tex', 0);
  g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
  g.u('u_alpha', alpha); setI(p, 'u_fromFbo', 1);
  draw();
  return true;
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
  const groupSpan = L.type === 'group' ? PM.groupSpan?.(L) : null;
  const start = Number(groupSpan?.from ?? L.from) || 0;
  const length = Math.max(0, Number(groupSpan?.dur ?? L.dur) || 0);
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

function groupCreatesCompositingBoundary(L: any, T: any, opt: any): boolean {
  if (L.type !== 'group') return false;
  const opacity = PM.clamp(PM.ev(L, 'opacity', T) / 100, 0, 1);
  const blend = isProperty(L.blend) ? PM.evP(L, L.blend, T, 'l.blend') : L.blend;
  const motionBlur = isProperty(L.mblur) ? PM.evP(L, L.mblur, T, 'l.mblur') : L.mblur;
  return opacity < 1 - 1e-6
    || hasRenderableEffects(L.fx || [], PM, L, T)
    || (L.masks || []).some((mask: any) => evaluatedValue(PM, L, mask.on, T, `m.${mask.id}.on`) !== false)
    || !!L.matteSource
    || !!(blend && blend !== 'normal')
    || !!(motionBlur && opt.mblur !== false)
    || !!activeTransition(L, T);
}

/** Default groups remain a zero-pass transform/organization feature. A group
 * becomes an offscreen boundary only on frames where one of its visual layer
 * properties needs the combined descendant image. */
function compositingPass(layers: any[], parentGroup: string | null, T: any, opt: any): any[] {
  const result: any[] = [];
  for (const layer of compositingLevel(layers, parentGroup)) {
    if (layer.type === 'group' && !groupCreatesCompositingBoundary(layer, T, opt)) {
      result.push(...compositingPass(layers, layer.id, T, opt));
    } else result.push(layer);
  }
  return result;
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
  const orderedLayers = depthOrderedLayers(PM, proj.layers, T);
  const parentGroup = typeof opt.groupParent === 'string' ? opt.groupParent : null;
  const layers = opt.explicitLayers ? orderedLayers : compositingPass(orderedLayers, parentGroup, T, opt);
  const solo = orderedLayers.some((layer: any) => layer.solo);
  // Rebuild per composition/pass so edits and nested mattes cannot leave a
  // stale index. One scan replaces a full stack scan for every rendered layer.
  const matteSources = new Set(orderedLayers.map((layer: any) => layer.matteSource));

  // Only local, ordinary 2D compositing is eligible. Effects, mattes and
  // non-normal blends can depend on pixels hidden by an opaque layer later.
  const covers: Array<RasterWindow | undefined> = [];
  if (!solo && !opt.exporting && opt.occlusionCulling !== false && previewSourceClipping && activePreviewViewport(W, H)
      && layers.every((layer: any) => !layer.threeD && !layer.fx?.length && !layer.masks?.length
        && !layer.matteSource && !layer.transitionIn && !layer.transitionOut && !layer.mblur
        && (!layer.blend || layer.blend === 'normal')
        && ['shape','text','image','video','audio','group','solid'].includes(layer.type))) {
    let largest: RasterWindow | undefined, area = 0;
    for (let i = 0; i < layers.length; i++) {
      covers[i] = largest;
      const layer = layers[i];
      if (layer.type !== 'shape' || layer.d.paths?.length || !PM.active(layer, T)
          || PM.canvasTextEditing === layer.id || layer.shy && opt.hideShy || PM.worldOpacity(layer, T) < 1) continue;
      const d = resolveContent(PM, layer, T), m = scaledWorld(layer, T, W, H);
      if (d.shape !== 'rect' || !/^#[0-9a-f]{6}$/i.test(d.color) || Math.abs(m[1]) > 1e-9 || Math.abs(m[2]) > 1e-9) continue;
      const geometry = shapeRasterGeometry(d, continuousRasterScale(m));
      const inset = Math.max(0, Math.min(Number(d.radius) || 0, d.w / 2, d.h / 2))
        + Math.max(0, Number(d.stroke) || 0) / 2 + 8 / geometry.density;
      const hw = (d.w / 2 - inset) * Math.abs(m[0]), hh = (d.h / 2 - inset) * Math.abs(m[3]);
      const x = Math.max(0, Math.ceil(m[4] - hw)), y = Math.max(0, Math.ceil(m[5] - hh));
      const width = Math.min(W, Math.floor(m[4] + hw)) - x, height = Math.min(H, Math.floor(m[5] + hh)) - y;
      if (width > 0 && height > 0 && width * height > area) { largest = {x,y,width,height}; area = width * height; }
    }
  }

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
        g.u('u_angle', fill.angle); g.u('u_stops', packed); g.u('u_view', ...backgroundView(W, H)); draw();
      }
    }
  }

  for (let i = layers.length - 1; i >= 0; i--) {
    const L = layers[i];
    if(PM.canvasTextEditing===L.id && !opt.exporting)continue;
    if (!opt.mattePass && matteSources.has(L.id)) continue;
    const groupAncestors = PM.groupAncestors?.(L, proj.layers) || [];
    if (solo && !L.solo && !groupAncestors.some((group: any) => group.solo)
        && !groupContainsSolo(L, orderedLayers, (layer: any) => PM.groupAncestors?.(layer, proj.layers) || [])
        && !opt.mattePass) continue;
    if (L.type !== 'group' && PM.TYPE_META[L.type] && PM.TYPE_META[L.type].visual === false) continue;
    if (!PM.active(L, T)) continue;
    if (L.shy && opt.hideShy) continue;
    /* Each group owns an offscreen compositing boundary, so opacity is applied
       once at its own level instead of being multiplied into every descendant. */
    const alpha = PM.clamp(PM.ev(L, 'opacity', T) / 100, 0, 1);
    if (alpha <= .001) continue;

    const hasFx = hasRenderableEffects(L.fx, PM, L, T);
    const hasMasks = (L.masks || []).some((m: any) => evaluatedValue(PM, L, m.on, T, `m.${m.id}.on`) !== false);
    const blend = (BLEND_ID as any)[isProperty(L.blend) ? PM.evP(L, L.blend, T, 'l.blend') : L.blend] || 0;
    const mb = (isProperty(L.mblur) ? PM.evP(L, L.mblur, T, 'l.mblur') : L.mblur) && opt.mblur !== false;
    const transition = activeTransition(L, T);

    /* Adjustment layers are full-frame processors over the already-rendered
       stack below. An adjustment without an enabled renderable effect is a
       transparent no-op, matching its lack of source pixels. */
    if (L.type === 'adjustment') {
      if (hasFx) acc = compositeAdjustment(L, T, acc, W, H, alpha, hasMasks, blend);
      continue;
    }

    /* fast path: no masks, no effects, normal blend, no motion blur → straight into acc */
    if (L.type !== 'group' && !hasMasks && !hasFx && !blend && !mb && !transition && !L.matteSource) {
      bind(acc);
      const cover = covers[i];
      if (cover) {
        gl.enable(gl.SCISSOR_TEST);
        try {
          for (const region of uncoveredRasterRegions(W, H, cover)) {
            gl.scissor(region.x, H - region.y - region.height, region.width, region.height);
            drawContent(L, T, W, H, alpha, region);
          }
        } finally { gl.disable(gl.SCISSOR_TEST); }
      } else drawContent(L, T, W, H, alpha);
      continue;
    }

    let lf: any;
    if (L.type === 'group') {
      if (mb) {
        lf = grab(W, H); bind(lf); clear();
        const n = opt.mbSamples || 10, shutter = (opt.shutter || .5) / proj.fps;
        for (let s = 0; s < n; s++) {
          const dt = ((s + .5) / n - .5) * shutter;
          const sample = GL.renderProject(proj, T + dt, W, H, {
            ...opt, transparent: true, groupParent: L.id, mblur: false,
          });
          bind(lf); drawFbo(sample, W, H, 1 / n); free(sample);
        }
      } else {
        lf = GL.renderProject(proj, T, W, H, { ...opt, transparent: true, groupParent: L.id });
      }
    } else {
      lf = grab(W, H);
      bind(lf); clear();
      if (mb) {
        const n = opt.mbSamples || 10, shutter = (opt.shutter || .5) / proj.fps;
        for (let s = 0; s < n; s++) {
          const dt = ((s + .5) / n - .5) * shutter;
          drawContent(L, T + dt, W, H, alpha / n);
        }
      } else drawContent(L, T, W, H, alpha);
    }

    let res = lf;
    if (hasMasks) res = applyMasks(L, T, res, W, H);
    if (hasFx) { const prior=res;res = runEffects(L, T, res, W, H);if(prior!==lf && res!==prior)free(prior); }
    if (L.matteSource) {const prior=res;res=applyTrackMatte(L,T,res,W,H,opt.matteProject||proj,opt);if(prior!==lf && prior!==res)free(prior);}

    let withLayer = acc;
    const compositeAlpha = L.type === 'group' ? alpha : 1;
    if (!blend) {
      if (transition) withLayer = copyFbo(acc, W, H);
      bind(withLayer);
      drawFbo(res, W, H, compositeAlpha);
    } else {
      const nxt = grab(W, H);
      bind(nxt); clear();
      const p = program('comp', PM.FRAG_COMPOSITE);
      const g = use(p);
      bindTex(0, res.tex); setI(p, 'u_tex', 0);
      bindTex(1, acc.tex); setI(p, 'u_dst', 1);
      g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
      g.u('u_alpha', compositeAlpha); setI(p, 'u_blend', blend);
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

const requestSourceWarmup = createPreviewWarmup(
  () => ({
    project: PM.proj, time: PM.time,
    key: JSON.stringify([sourceGeneration, PM.animVersion?.(), GL.canvas?.width, GL.canvas?.height, GL.previewViewport, PM.quality]),
    blocked: !GL.gl || PM.Export?.busy || PM.Preview?.preparing || PM.Preview?.active || PM.agentFrameCapture,
  }),
  (layer, time) => {
    // Speculation must fit the existing cache and leave room for the next
    // visible frame. A large source keeps the normal demand-driven path.
    if (layer.d.paths?.length || is3DLayer(PM, layer)) return;
    const W = GL.canvas.width, H = GL.canvas.height;
    PM.scope.push(PM.proj); PM.beginEval(time);
    previewViewportActive = !!GL.previewViewport; previewSourceClipping = true; preparingSource = true;
    try {
      if (!PM.active(layer, time)) return;
      const scale = continuousRasterScale(scaledWorld(layer, time, W, H));
      const geometry = layer.type === 'shape' ? shapeRasterGeometry(resolveContent(PM, layer, time), scale)
        : PM.textRasterGeometry(layer, scale, time);
      const bytes = geometry.width * geometry.height * 4;
      if (bytes > 4 * 1024 * 1024) return;
      // Old zoom/animation variants are disposable; the last visible frame's
      // sources are not. Reclaim only unused variants for speculative uploads.
      const target = MAX_TEXTURE_BYTES - bytes - 4 * 1024 * 1024;
      if (textureBytes > target) trimTextures(target, visibleTextures);
      if (textureBytes <= target) contentQuad(layer, time, W, H);
    } finally {
      bindTex(0, null);
      previewViewportActive = false; previewSourceClipping = false; preparingSource = false;
      PM.scope.pop(); PM.beginEval(PM.time);
    }
  },
  run => window.requestIdleCallback?.(run),
);

GL.render = (T: any, opt: any = {}) => {
  visibleTextures.clear();
  framePrograms.clear(); compileSubmitMs = 0;
  const gl = GL.gl; if (!gl) return;
  gpuTiming?.poll();
  const t0 = window.performance.now();
  GL.stats.draws = 0; GL.stats.passes = 0; GL.stats.viewportVectors = 0;
  viewportPathFrame++;
  const W = GL.canvas.width, H = GL.canvas.height;

  PM.beginEval(T);
  PM.scope.push(PM.proj);
  previewViewportActive = !!GL.previewViewport;
  // Full sources remain available as a reference for pixel/performance checks.
  previewSourceClipping = opt.sourceClipping !== false && !opt.exporting;
  let acc;
  const priorParallel = allowParallelCompile; allowParallelCompile = !opt.exporting;
  try {
    acc = GL.renderProject(PM.proj, T, W, H, opt);
  } finally {
    allowParallelCompile = priorParallel;
    PM.scope.pop();
    previewViewportActive = false;
    previewSourceClipping = false;
  }

  /* present */
  bind(null);
  gl.clearColor(0, 0, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT);
  const p = program('present', PM.FRAG_COPY);
  const g = use(p);
  bindTex(0, acc.tex); setI(p, 'u_tex', 0);
  g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
  gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
  if (presentedFrame && presentedFrame !== acc) free(presentedFrame);
  presentedFrame = acc;
  GL.pool.forEach((f: any) => f.busy = f === presentedFrame);
  trimPool();
  trimTextures();
  trimViewportPathRasters();
  // Keep every program used by this frame: an active scene larger than the
  // cache target must not recompile its shaders on every scrub.
  for (const key of GL.progs.keys()) {
    if (GL.progs.size <= 256) break;
    if (!framePrograms.has(key)) GL.dropProgram(key);
  }
  GL.stats.progs = GL.progs.size;
  GL.stats.ms = window.performance.now() - t0;
  if (!opt.exporting && typeof window.requestIdleCallback === 'function') requestSourceWarmup();
  if (GL.previewViewport && !opt.exporting) PM.bus?.emit?.('preview:presented', { viewport: GL.previewViewport, time: T, version: PM.animVersion?.(), project: PM.proj, quality: PM.quality });
};

/** Render one frame and read back raw RGBA pixels (bottom-up, premultiplied).
    Used for transparent PNG export where the canvas itself is opaque. */
GL.renderToPixels = (T: any, W: any, H: any, opt: any = {}) => {
  const gl = GL.gl; if (!gl) return null;
  PM.beginEval(T);
  let acc;
  try {
    PM.scope.push(PM.proj);
    try { acc = GL.renderProject(PM.proj, T, W, H, opt); }
    finally { PM.scope.pop(); }
    bind(acc);
    const px = new Uint8Array(W * H * 4);
    if (opt.opaque) {
      // Match the visible RGBA8 presentation's quantization without touching its
      // canvas, viewport metadata, or retained frame. Read just the capture size.
      const target = gl.createFramebuffer(), color = gl.createRenderbuffer();
      try {
        gl.bindRenderbuffer(gl.RENDERBUFFER, color);
        gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, W, H);
        gl.bindFramebuffer(gl.FRAMEBUFFER, target);
        gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, color);
        gl.viewport(0, 0, W, H);
        const p = program('present', PM.FRAG_COPY), g = use(p);
        bindTex(0, acc.tex); setI(p, 'u_tex', 0);
        g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
        gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
        gl.readPixels(0, 0, W, H, gl.RGBA, gl.UNSIGNED_BYTE, px);
      } finally {
        gl.bindRenderbuffer(gl.RENDERBUFFER, null);
        gl.deleteFramebuffer(target); gl.deleteRenderbuffer(color);
        bind(null);
      }
    } else {
      const floats=new Float32Array(W*H*4);
      gl.readPixels(0,0,W,H,gl.RGBA,gl.FLOAT,floats);
      for(let i=0;i<px.length;i++)px[i]=Math.round(Math.max(0,Math.min(1,floats[i]!))*255);
    }
    return px;
  } finally {
    if (acc) free(acc);
    GL.pool.forEach((f: any) => f.busy = f === presentedFrame);
    trimPool();
    trimTextures();
    viewportPathFrame++;
    trimViewportPathRasters();
    bind(null);
  }
};

/* Hit test: which layer is under a comp-space point (top-most first). */
GL.pick = (x: any, y: any, T: any, options: { includeLocked?: boolean } = {}) => {
  const layers = depthOrderedLayers(PM, PM.proj.layers, T);
  for (const L of layers) {
    if (!PM.active(L, T) || !options.includeLocked && (L.lock || (PM.groupAncestors?.(L) || []).some((group: any) => group.lock)) || (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].pickable === false)) continue;
    const b = GL.bounds(L, T);
    if (!b) continue;
    if (planeContains(PM, L, T, x, y, b)) return L;
  }
  return null;
};
/** Layer-space bounds (before transform), relative to the layer anchor origin. */
GL.bounds = (L: any, T: any) => {
  if (L.type === 'group') return PM.groupBounds(L, T);
  const d = resolveContent(PM, L, T);
  let w, h, ax, ay;
  if (L.type === 'solid' || L.type === 'shader' || L.type === 'extension') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'null') { w = d.w || 100; h = d.h || 100; ax = 0; ay = 0; }
  else if (L.type === 'precomp') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'text') {
    const r = PM.raster(L, 1, T);
    if (r.selection) return { ...r.selection, ax: r.anchorX, ay: r.anchorY };
    w = r.w; h = r.h; ax = r.anchorX; ay = r.anchorY;
  }
  else if (L.type === 'shape') {
    if (!L.d.paths?.length) {
      const geometry = shapeRasterGeometry(d, 1);
      return { ...geometry.selection, ax: geometry.w / 2, ay: geometry.h / 2 };
    }
    const r = PM.raster(L, 1, T);
    if (r.selection) return { ...r.selection, ax: L.d.paths?.length?r.anchorX:r.w / 2, ay: L.d.paths?.length?r.anchorY:r.h / 2 };
    w = r.w; h = r.h;
    ax = w / 2; ay = h / 2;
  } else if (L.type === 'image' || L.type === 'video') {
    w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = w / 2; ay = h / 2;
  } else return null;
  return { x0: -ax, y0: -ay, x1: w - ax, y1: h - ay, w, h, ax, ay };
};
}
