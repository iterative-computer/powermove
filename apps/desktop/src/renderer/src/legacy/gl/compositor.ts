import {brushGeometry,brushBounds,rasterBrush} from './effect-path';
import { createAutomaticPreviewVideos } from '../core/automatic-preview-video';
import { motionBlurTimes } from '../../core/render-samples';
import { layerInputPlan, hasLayerInputs, inputKey, assertInputBudget, temporalTimes, EffectInputError, referenceParam, referencedPath } from '../../core/effect-inputs';
import { sampleLod, mipBytes, gridSize } from './effect-sampling';
import {viewportCamera,viewportRevision,getViewportMode} from '../../core/scene3d/viewport';
import { effectProgramKey } from '../../kernel/effect-versions';
import { availableColorDepth, colorBufferBytes, normalizeColorDepth, supportedColorDepths } from '../../core/color-depth';
import { videoClipsAt } from '../core/video-timeline';
import { sizeAnchorOffset } from '../core/size-anchor';
import { previewSeekFrame, seekPreviewVideo } from '../core/video-seek';
import { layerVideoElement, videoInstanceTextureKey } from '../core/video-instances';
import { videoPlaybackTime } from '../../../../shared/video-timing';
import { GPUTiming } from './gpu-timing';
import { createOpaqueReadback } from './opaque-readback';
import { createAlphaReadback, createFloatReadback, createHalfReadback, type DeferredAlphaReader } from './half-readback';
import { performanceMonitor } from '../../runtime/performance-monitor';
import { is3DLayer, planeMatrix, planeContains, inversePlane, affinePlane, projectPoint } from '../core/space-3d';
import { pathValues, groupMatrix, rasterPathsToViewport, tracePath } from '../core/vector-paths';
import { createPreviewWarmup } from './preview-warmup';
import { PreviewFrames } from './preview-frames';
import { previewPlan } from './preview-plan';
import { sourceTime } from '../core/retiming';
import { layer3DRole } from '../../core/scene3d/layers';
import { sceneRuntime, pruneSceneRuntimes, compositionRuntime, compositionDepthIds,compositionOrderedLayers } from '../../core/scene3d/service';
import { evaluatedValue, isProperty, resolveContent } from '../core/content-properties';
/* Ported from js/gl/compositor.js — behavior-preserving. */
import type { PMRegistry } from '../registry';
import { extensionLayerFragment } from '../../kernel/extension-layers';
import { cornerInset } from './corner-geometry';
import { uncoveredRasterRegions, previewShapeRaster, previewTextRaster, rasterIntersectsViewport, shapeRasterGeometry, type RasterWindow } from './shape-raster-window';

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

/** Index membership once per pass. Rescanning the whole composition for every
 * ordinary group makes flattening a grouped scene quadratic in layer count.
 * Keep this index local: edits and time-dependent group boundaries must be
 * visible immediately, including nested composition and motion-blur passes. */
export function compositingLayers(layers: any[], parentGroup: string | null, boundary: (layer: any) => boolean): any[] {
  let members: Map<string | null, any[]> | undefined;
  const children = (parent: string) => {
    if (!members) {
      members = new Map();
      for (const layer of layers) {
        const owner = layer.group || null;
        const siblings = members.get(owner);
        if (siblings) siblings.push(layer);
        else members.set(owner, [layer]);
      }
    }
    return members.get(parent) || [];
  };
  const result: any[] = [];
  const visit = (level: any[]) => {
    for (const layer of level) {
      if (layer.type === 'group' && !boundary(layer)) visit(children(layer.id));
      else result.push(layer);
    }
  };
  visit(compositingLevel(layers, parentGroup));
  return result;
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
  const fallback = () => PM.effectParamDefault?.(param, layer, PM.curComp?.() || PM.proj, time) ?? param.def;
  if (!prop) return fallback();
  const value = PM.evP(layer, prop, time, param.k);
  return value == null ? fallback() : value;
}

/** Track frames the browser actually presents, rather than decoder totals that
    can advance in bursts before the corresponding frame is available to WebGL. */
export function trackPresentedVideoFrames(el: any, onFrame: () => void): { version: number; supported: boolean } {
  const state = { version: 0, supported: typeof el?.requestVideoFrameCallback === 'function' };
  el.addEventListener?.('seeked', () => { state.version++; onFrame(); });
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
let readbackGeneration = {};
let preparedVideoTextures = new WeakMap<object, { version: unknown; sources: WeakMap<object, { key: string; version: object }> }>();
/* Keep the composited FBO alive until the next frame. Resizing a canvas clears
   its drawing buffer synchronously; retaining this texture lets resize()
   present the last complete image at the new size instead of exposing black
   while the editor's next animation frame is still queued. */
let presentedFrame: any = null;
let recyclePreviewEvictions = false;
const previewFrames = new PreviewFrames<any>(frame => {
  if (presentedFrame === frame) presentedFrame = null;
  // A full cache replaces an old frame on every new playback/scrub frame.
  // Return that target to the working pool instead of allocating and deleting
  // a full-resolution GPU texture each time. Explicit clears and memory
  // pressure still release resources immediately.
  if (recyclePreviewEvictions && frame.w === GL.canvas?.width && frame.h === GL.canvas?.height) {
    frame.previewCached = false;
    frame.busy = false;
    frame.used = ++resourceTick;
    GL.pool.push(frame);
    poolBytes += frame.bytes;
    return;
  }
  for (let unit = 0; unit < boundTex.length; unit++) if (boundTex[unit] === frame.tex) bindTex(unit, null);
  GL.gl?.deleteFramebuffer(frame.fb);
  GL.gl?.deleteTexture(frame.tex);
  if (frame.depthBuffer) GL.gl?.deleteRenderbuffer(frame.depthBuffer);
});
GL.previewFrames = previewFrames;
let previewSignature = '', previewProject: any = null, frameFailed = false, frameVideoExact = true;
let previewLayers: any = null;
let previewHasMotionBlur = false;
let scenePreviewPlan: ReturnType<typeof previewPlan> | null = null;
let rememberingPreview = false;
let idleVideoFrames: Map<string, ImageBitmap> | null = null;
const inputPreviewVideos = createAutomaticPreviewVideos(PM);
let inputPreviewJob: {key:string;project:any;gl:any;time:number;pending:boolean;frames:Map<string,ImageBitmap>|null} | null = null;
let exactInputPreviewRendering = false;
function cancelInputPreview() {
  inputPreviewVideos.cancel(); inputPreviewJob = null;
}
GL.cancelInputPreview = cancelInputPreview;
Object.defineProperty(GL, 'inputPreviewBytes', {get:()=>inputPreviewVideos.bytes});
if (typeof window !== 'undefined') window.addEventListener?.('pagehide',()=>inputPreviewVideos.dispose(),{once:true});

let cancelPreviewPreparation: (() => void) | undefined;
GL.previewPreparationBytes = 0;
GL.cancelPreviewPreparation = () => cancelPreviewPreparation?.();
for (const event of ['project', 'layers', 'assets', 'quality', 'fonts']) PM.bus?.on?.(event, () => {
  GL.cancelPreviewPreparation(); cancelInputPreview();
  previewFrames.clear(); previewSignature = ''; scenePreviewPlan = null;
});
const previewStateKey = () => JSON.stringify([PM.animVersion?.(), sourceGeneration, GL.canvas?.width, GL.canvas?.height,
  GL.previewViewport, PM.quality, PM.previewResolution, PM.previewFps, PM.proj.fps, PM.proj.layers.length,viewportRevision(PM), normalizeColorDepth(PM.proj.colorDepth)]);
const canCachePreview = (opt: any) => !hasLayerInputs(PM.proj, (fx:any)=>PM.effectDef?.(fx) ?? PM.Kernel?.effects?.get?.(fx.type)) && opt.previewReuse !== false && !opt.exporting && !PM.Export?.busy && !PM.agentFrameCapture
  && !PM.Preview?.preparing && !PM.Preview?.active && !PM.preparedVideoFrames && !PM.assets?.loading?.size;
GL.previewPreparationKey = previewStateKey;
GL.reservePreviewPreparation = (bytes: number) => {
  const budget = Math.max(0, (PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024) - (PM.Preview?.bytes || 0));
  if (!Number.isFinite(bytes) || bytes <= 0 || !budget) return;
  // A new work area replaces older LRU pictures, but the current displayed
  // texture remains available for immediate resize/navigation presentation.
  previewFrames.trim(Math.max(0, budget - Math.min(bytes, budget * .9)), new Set([presentedFrame]));
};
for (const event of ['extensions:changed', 'theme:changed']) PM.Kernel?.events?.on?.(event, () => {
  GL.cancelPreviewPreparation(); cancelInputPreview(); sourceGeneration++;
  previewFrames.clear(); previewSignature = ''; scenePreviewPlan = null;
});
function refreshPreviewState() {
  const signature = previewStateKey();
  if (previewProject !== PM.proj || previewLayers !== PM.proj.layers || previewSignature !== signature) {
    previewFrames.clear(); previewProject = PM.proj; previewLayers = PM.proj.layers; previewSignature = signature;
    previewHasMotionBlur = (PM.ProjectIndex?.allLayers?.() || PM.proj.layers).some((layer: any) => layer.mblur || layer.type === 'comp');
    scenePreviewPlan = previewPlan(PM.proj, (effect: any) => {
      const definition = PM.effectDef?.(effect) ?? (!PM.effectDef ? PM.FX[effect.type] : undefined);
      const linked = GL.progs.get(effectProgramKey(effect.type, effect.version, PM.Kernel?.effects?.get?.(effect.type)));
      return !!definition && !!linked && !definition.backdrop
        && uloc(linked, 'u_time') === null && uloc(linked, 'u_prog') === null;
    });
  }
}
function previewFrameKey(T: number, opt: any, key?: string | number): string {
  const blur = previewHasMotionBlur && opt.mblur !== false;
  const normalized = { ...opt, mblur: blur, mbSamples: blur ? opt.mbSamples || 10 : 0, shutter: blur ? opt.shutter || .5 : 0 };
  const radius = blur ? (opt.shutter || .5) / Math.max(1, PM.proj.fps) : 0;
  const timeKey = key ?? scenePreviewPlan?.timeKey(T, radius) ?? T;
  return JSON.stringify([typeof timeKey === 'number' ? Number(timeKey.toFixed(9)) : timeKey, Object.entries(normalized).sort(([a], [b]) => a.localeCompare(b))]);
}
GL.hasPreviewFrame = (T: number, opt: any) => canCachePreview(opt) && previewProject === PM.proj
  && previewLayers === PM.proj.layers && previewSignature === previewStateKey() && previewFrames.has(previewFrameKey(T, opt));
function rememberPreview(key: string, frame: any): boolean {
  if (frameFailed || !frameVideoExact || pendingPrograms.size || GL.errors.size
      || typeof document === 'undefined' || document.fonts?.status === 'loading') return false;
  const budget = Math.max(0, (PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024) - (PM.Preview?.bytes || 0));
  let remembered = false;
  recyclePreviewEvictions = true;
  try { remembered = previewFrames.put(key, frame, budget); }
  finally { recyclePreviewEvictions = false; }
  if (remembered) {
    const index = GL.pool.indexOf(frame);
    if (index >= 0) { GL.pool.splice(index, 1); poolBytes -= frame.bytes; }
    frame.previewCached = true;
  }
  return remembered;
}
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

let useVideoPreviews = false;
let videoPath = '';
let previewVideoTimes = new Map<string, number>();
let isolatedVideoTexturePrefix: string | null = null;
let isolatedVideoTextureGeneration = 0;
let isolatedCaptureNoMotionBlur = false;
class MissingCaptureVideoFrame extends Error {}

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
  // Float buffers alone are insufficient: the default sampler precision can
  // round values while reading the preceding pass, including raw extensions.
  frag = frag.replace(/^(#version[^\n]*\n)/, '$1precision highp sampler2D;\n');
  if (allowParallelCompile && parallelShaderCompile && /^(sh:|extension:|fx:|tr:)/.test(key)) {
    if (compileSubmitMs >= 4 || pendingPrograms.size >= 8) { frameFailed = true; pollCompiles(); return null; }
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
  GL.cancelPreviewPreparation(); cancelInputPreview(); sourceGeneration++;
  previewSignature = ''; scenePreviewPlan = null;
  previewFrames.clear();
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
  if (referenceParam(pd)) return;
  if (pd.type === 'color') setU(p, name, PM.hex2rgb(String(value)));
  else if (pd.type === 'toggle') setU(p, name, [value ? 1 : 0]);
  else setU(p, name, [Number(value) || 0]);
}

/* ── FBO pool ──────────────────────────────────────────── */
const warnedDepths = new Set<number>();
function workingColorDepth() {
  const requested = normalizeColorDepth(PM.proj.colorDepth);
  const actual = availableColorDepth(requested, GL.supportedColorDepths || [8, 16]);
  if (actual !== requested && !warnedDepths.has(requested)) {
    warnedDepths.add(requested);
    PM.toast?.(`${requested}-bit color is unavailable on this GPU. Using ${actual}-bit rendering.`);
  }
  GL.colorDepth = actual;
  return actual;
}
function grab(w: any, h: any, depth = false, colorDepth = workingColorDepth()) {
  const gl = GL.gl;
  if (layerInputRendering) {
    const bytes = colorBufferBytes(w, h, colorDepth, depth);
    if (GL.pool.reduce((sum: number, f: any) => sum + (f.busy ? f.bytes : 0), 0) + bytes > (PM.Memory?.budget?.('framebuffers') ?? MAX_FBO_BYTES))
      throw new EffectInputError('Layer inputs exceed the framebuffer memory budget');
  }
  for (let i = 0; i < GL.pool.length; i++) {
    const f = GL.pool[i];
    if (!f.busy && f.w === w && f.h === h && !!f.depth === depth && f.colorDepth === colorDepth) { f.busy = true; f.used = ++resourceTick; return f; }
  }
  const tex = gl.createTexture();
  bindTex(0, tex);
  const format = colorDepth === 32 ? gl.RGBA32F : colorDepth === 16 ? gl.RGBA16F : gl.RGBA8;
  const type = colorDepth === 32 ? gl.FLOAT : colorDepth === 16 ? gl.HALF_FLOAT : gl.UNSIGNED_BYTE;
  gl.texImage2D(gl.TEXTURE_2D, 0, format, w, h, 0, gl.RGBA, type, null);
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
  if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
    gl.deleteFramebuffer(fb); gl.deleteTexture(tex);
    if (depthBuffer) gl.deleteRenderbuffer(depthBuffer);
    throw new Error(`Could not allocate a ${colorDepth}-bit color buffer (${w}×${h})`);
  }
  const f = { fb, tex, depth, depthBuffer, colorDepth, w, h, busy: true, used: ++resourceTick, bytes: colorBufferBytes(w, h, colorDepth, depth) };
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
    if (poolBytes <= targetBytes && f.colorDepth === GL.colorDepth) continue;
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
type SourceRaster = {layer:any;project:any;bounds:{x0:number;y0:number;w:number;h:number};extent?:{x0:number;y0:number;w:number;h:number};domain?:{x0:number;y0:number;w:number;h:number}};
let sourceRaster: SourceRaster | null = null;
const activeSourceRaster = () => sourceRaster && (PM.curComp?.() || PM.proj) === sourceRaster.project ? sourceRaster : null;
function outputScale(W: number, H: number): [number, number] {
  const raster=activeSourceRaster();if(raster)return[W/(raster.extent??raster.bounds).w,H/(raster.extent??raster.bounds).h];
  const viewport = activePreviewViewport(W, H);
  if (viewport) return [W / Math.max(1e-6, viewport.width), H / Math.max(1e-6, viewport.height)];
  const comp = PM.curComp?.() || PM.proj;
  return [W / Math.max(1, Number(comp?.w) || W), H / Math.max(1, Number(comp?.h) || H)];
}

function effectViewportCoordinates(W: number, H: number): { uv: [number, number, number, number]; origin: [number, number]; resolution: [number, number] } {
  if(activeSourceRaster())return{uv:[0,0,1,1],origin:[0,0],resolution:[W,H]};
  const viewport = activePreviewViewport(W, H);
  const [sx, sy] = outputScale(W, H);
  const comp = PM.curComp?.() || PM.proj;
  const resolution: [number, number] = [
    Math.max(1, Number(comp?.w) || W) * sx,
    Math.max(1, Number(comp?.h) || H) * sy,
  ];
  const origin: [number, number] = viewport ? [viewport.x * sx, viewport.y * sy] : [0, 0];
  return {
    origin,
    resolution,
    uv: [origin[0] / resolution[0], origin[1] / resolution[1], W / resolution[0], H / resolution[1]],
  };
}

/** Map composition pixels back to the layer's uncropped, normalized bounds.
 * Effects still sample the full-frame buffer; only their gradient geometry
 * follows the layer, including parent transforms and perspective. */
function inputGroupBounds(group: any, time: number) {
  const world = PM.worldMatrix(group, time), det = world[0]*world[3]-world[1]*world[2];
  if (Math.abs(det) < 1e-10) return null;
  const inverse = [world[3]/det,-world[1]/det,-world[2]/det,world[0]/det,
    (world[2]*world[5]-world[3]*world[4])/det,(world[1]*world[4]-world[0]*world[5])/det];
  let x0=Infinity,y0=Infinity,x1=-Infinity,y1=-Infinity;
  const project=PM.curComp?.() || PM.proj;
  for (const child of project.layers) {
    if (['group','audio','null','adjustment'].includes(child.type)
      || !(PM.groupAncestors?.(child,project.layers)||[]).some((g:any)=>g.id===group.id)
      || !inputLayerActive(child,time,project,{inputVisibilityRoot:group.id})) continue;
    const bounds=GL.bounds(child,time); if(!bounds)continue;
    const local=PM.mul(inverse,PM.worldMatrix(child,time));
    for(const x of [bounds.x0,bounds.x1]) for(const y of [bounds.y0,bounds.y1]) {
      const px=local[0]*x+local[2]*y+local[4],py=local[1]*x+local[3]*y+local[5];
      x0=Math.min(x0,px);y0=Math.min(y0,py);x1=Math.max(x1,px);y1=Math.max(y1,py);
    }
  }
  return Number.isFinite(x0) ? {x0,y0,x1,y1,w:x1-x0,h:y1-y0} : null;
}
function effectLayerCoordinates(L: any, T: number, W: number, H: number, input = false):{uv:Float32Array;resolution:number[];valid?:boolean} {
  const raster=activeSourceRaster();if(raster&&raster.layer===L){const e=raster.extent??raster.bounds,b=raster.bounds;return{uv:new Float32Array([e.w/(W*b.w),0,0,0,e.h/(H*b.h),0,(e.x0-b.x0)/b.w,(e.y0-b.y0)/b.h,1]),resolution:[b.w,b.h],valid:true};}
  const bounds = input && L.type === 'group' ? inputGroupBounds(L,T) : GL.bounds(L, T);
  const [sx, sy] = outputScale(W, H);
  const world = PM.worldMatrix(L, T);
  const plane = is3DLayer(PM, L) ? planeMatrix(PM, L, T)
    : [world[0], world[1], 0, world[2], world[3], 0, world[4], world[5], 1];
  const inverse = inversePlane([plane[0] * sx, plane[1] * sy, plane[2],
    plane[3] * sx, plane[4] * sy, plane[5], plane[6] * sx, plane[7] * sy, plane[8]]);
  if (bounds && bounds.w > 0 && bounds.h > 0 && inverse) {
    return {
      uv: new Float32Array([0, 1, 2].flatMap(i => {
        const x = inverse[i * 3]!, y = inverse[i * 3 + 1]!, w = inverse[i * 3 + 2]!;
        return [(x - bounds.x0 * w) / bounds.w, (y - bounds.y0 * w) / bounds.h, w];
      })),
      resolution: [bounds.w, bounds.h],valid:true,
    };
  }
  // Adjustment layers have no source bounds and operate on the composition.
  const { resolution } = effectViewportCoordinates(W, H);
  return { uv: new Float32Array([1 / resolution[0], 0, 0, 0, 1 / resolution[1], 0, 0, 0, 1]), resolution,valid:L.type==='adjustment' };
}

function scaledWorld(layer: any, time: any, W: number, H: number): [number, number, number, number, number, number] {
  const [sx, sy] = outputScale(W, H);
  const raster=activeSourceRaster();if(raster&&raster.layer===layer){const e=raster.extent??raster.bounds;return[sx,0,0,sy,-e.x0*sx,-e.y0*sy];}
  const world = PM.worldMatrix(layer, time);
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
  t.warmUntil = preparingSource ? performance.now() + 2500 : 0;
  if (opts.version !== undefined && t.v === opts.version) return t.tex;
  bindTex(0, t.tex);
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
  gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
  let stored = true;
  try { if(opts.float){gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MIN_FILTER,gl.NEAREST);gl.texParameteri(gl.TEXTURE_2D,gl.TEXTURE_MAG_FILTER,gl.NEAREST);gl.texImage2D(gl.TEXTURE_2D,0,gl.RGBA32F,source.width,source.height,0,gl.RGBA,gl.FLOAT,source.data);}else gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source); } catch (e) { stored = false; }
  gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, false);
  const width = Number(source?.videoWidth || source?.naturalWidth || source?.displayWidth || source?.width || 0);
  const height = Number(source?.videoHeight || source?.naturalHeight || source?.displayHeight || source?.height || 0);
  if (!stored || width <= 0 || height <= 0) frameFailed = true;
  // An empty or failed upload must not be stamped as current, or the key
  // would draw nothing until its texture is dropped. Leave it unversioned so
  // the next frame uploads again from a real source.
  if (stored && width > 0 && height > 0) t.v = opts.version === undefined ? t.v : opts.version;
  const bytes = Math.max(0, width * height * (opts.float?16:4));
  textureBytes += bytes - (t.bytes || 0);
  t.bytes = bytes;
  // Private uploads are removed immediately after their layer draw. Scheduling
  // forced idle trimming for them would later evict retained viewer textures.
  if (!isolatedVideoTexturePrefix || !key.startsWith(isolatedVideoTexturePrefix)) PM.Memory?.maintain?.('textures', GL.texes.size > MAX_TEXTURE_ENTRIES);
  return t.tex;
}
function trimTextures(targetBytes: number = MAX_TEXTURE_BYTES, protectedKeys?: ReadonlySet<string>) {
  if (textureBytes <= targetBytes && GL.texes.size <= MAX_TEXTURE_ENTRIES) return;
  const entries = [...GL.texes.entries()].sort((a: any, b: any) => a[1].used - b[1].used);
  // Prefer disposable old variants over sources prepared for the next scene.
  // A second pass still enforces the hard budget under foreground pressure.
  for (const protectWarm of [true, false]) for (const [key, entry] of entries) {
    if (textureBytes <= targetBytes && GL.texes.size <= MAX_TEXTURE_ENTRIES || GL.texes.size <= 1) break;
    if (!GL.texes.has(key) || boundTex.includes(entry.tex) || protectedKeys?.has(key)
        || protectWarm && entry.warmUntil > performance.now()) continue;
    GL.gl?.deleteTexture?.(entry.tex);
    textureBytes = Math.max(0, textureBytes - (entry.bytes || 0));
    GL.texes.delete(key);
  }
}
function dropTextures(prefix: string) {
  for (const [key, entry] of GL.texes) {
    if (!key.startsWith(prefix)) continue;
    if (GL.gl && entry.tex) GL.gl.deleteTexture(entry.tex);
    textureBytes = Math.max(0, textureBytes - (entry.bytes || 0));
    GL.texes.delete(key);
  }
}
function dropIsolatedVideoTexture(key: string) {
  const entry = GL.texes.get(key);
  if (!entry) return;
  for (let unit = 0; unit < boundTex.length; unit++) if (boundTex[unit] === entry.tex) bindTex(unit, null);
  GL.gl?.deleteTexture?.(entry.tex);
  textureBytes = Math.max(0, textureBytes - (entry.bytes || 0));
  GL.texes.delete(key);
}
GL.dropTextures = (prefix = '') => {
  GL.cancelPreviewPreparation(); cancelInputPreview();
  previewFrames.clear();
  sourceGeneration++;
  dropTextures(prefix);
};

function trimViewportPathRasters() {
  for (const [id, entry] of viewportPathRasters) {
    if (entry.frame >= viewportPathFrame - 1 || entry.warmUntil > performance.now()) continue;
    entry.cv.width = 0; entry.cv.height = 0;
    viewportPathRasters.delete(id);
    // Cache eviction changes no artwork. Do not invalidate every prepared
    // scene or remembered frame when one obsolete path bitmap is released.
    dropTextures('viewport-path:' + id);
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
  readbackGeneration = {};
  preparedVideoTextures = new WeakMap();
  GL.cancelPreviewPreparation(); cancelInputPreview();
  previewFrames.clear();
  GL.canvas = canvas;
  GL.maxTextureSize = undefined;
  const gl = canvas.getContext('webgl2', {
    alpha: options.alpha === true, antialias: false, premultipliedAlpha: true,
    preserveDrawingBuffer: true, powerPreference: 'high-performance', desynchronized: true,
  });
  if (!gl) { if (!options.quiet) window.alert('Powermove needs WebGL2.'); return false; }
  cancelInputPreview();
  gpuTiming?.dispose();
  GL.gl = gl;
  sourceGeneration++;
  gpuTiming = new GPUTiming(gl);
  parallelShaderCompile = gl.getExtension('KHR_parallel_shader_compile');
  if (!watchedCanvases.has(canvas)) {
    watchedCanvases.add(canvas);
    canvas.addEventListener('webglcontextlost', (event: Event) => {
      event.preventDefault();
      GL.cancelPreviewPreparation(); cancelInputPreview();
      previewFrames.clear();
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
  GL.supportedColorDepths = supportedColorDepths(gl);
  warnedDepths.clear();
  workingColorDepth();
  GL.quad = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, GL.quad);
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([0, 0, 1, 0, 0, 1, 1, 1]), gl.STATIC_DRAW);
  gl.disable(gl.DEPTH_TEST);
  gl.enable(gl.BLEND);
  gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA);
  return true;
};

GL.resize = (w: any, h: any, previewViewport: any = null) => {
  GL.cancelPreviewPreparation();
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
function sharedPreparedVideoTexture(source: object | undefined, key: string) {
  const frames = PM.preparedVideoFrames;
  if (!source || !frames || !PM.Export?.busy || useVideoPreviews || isolatedVideoTexturePrefix || idleVideoFrames) return null;
  const version = PM.preparedVideoVersion;
  let generation = preparedVideoTextures.get(frames);
  if (!generation || generation.version !== version) {
    generation = { version, sources: new WeakMap() };
    preparedVideoTextures.set(frames, generation);
  }
  let entry = generation.sources.get(source);
  if (!entry) {
    // Keep the first asset-prefixed key so normal eviction/asset cleanup still
    // owns this texture. The token also distinguishes replaced maps/canvases
    // when a scoped caller restores the same numeric preparation version.
    entry = { key, version: {} };
    generation.sources.set(source, entry);
  }
  return entry;
}

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

function contentQuad(L: any, T: any, W: any, H: any, clip?: RasterWindow, maxSourceBytes = Infinity) {
  const source = nativeContentQuad(L, T, W, H, clip, maxSourceBytes);
  if (!source || source.screenSpace) return source;
  const offset = sizeAnchorOffset(PM, L, T);
  if (!offset.x && !offset.y) return source;
  return { ...source, ax: source.ax - offset.x, ay: source.ay - offset.y };
}

const shaderUniformDefinitions = new WeakMap<object, { code: string; definitions: any[] }>();

function viewportPathQuad(L: any, T: any, W: number, H: number, maxSourceBytes: number) {
  const world = scaledWorld(L, T, W, H);
  const previous = viewportPathRasters.get(L.id);
  if (W * H * 4 > maxSourceBytes) return null;
  if (preparingSource && !previous && [...viewportPathRasters.values()].reduce((bytes, entry) => bytes + entry.cv.width * entry.cv.height * 4, W * H * 4) > MAX_TEXTURE_BYTES) return null;
  const raster = rasterPathsToViewport(PM, L, T, W, H, world, previous);
  if (raster !== previous) {
    if (previous?.cv) { previous.cv.width = 0; previous.cv.height = 0; }
    raster.version = ++viewportPathVersion;
    viewportPathRasters.set(L.id, raster);
  }
  raster.frame = viewportPathFrame;
  raster.warmUntil = preparingSource ? performance.now() + 2500 : 0;
  GL.stats.viewportVectors = (GL.stats.viewportVectors || 0) + 1;
  const tex = texFor('viewport-path:' + L.id, raster.cv, { version: raster.version });
  return { tex, w: W, h: H, ax: 0, ay: 0, uv: [0, 0, 1, 1], screenSpace: true };
}

function nativeContentQuad(L: any, T: any, W: any, H: any, clip?: RasterWindow, maxSourceBytes = Infinity) {
  /* returns {tex, w, h, ax, ay, uv:[ox,oy,sx,sy], fromFbo, solid, tmp} */
  const d = resolveContent(PM, L, T);
  if (L.type === 'solid') {
    return { solid: PM.hex2rgb(d.color), solidAlpha: PM.hexAlpha(d.color), w: d.w || W, h: d.h || H, ax: 0, ay: 0 };
  }
  if (L.type === 'shape' && L.d.paths?.length && activePreviewViewport(W, H)
      && (PM.previewResolution === '1' || PM.perf?.auto === false && PM.quality === 1)) {
    return viewportPathQuad(L, T, W, H, maxSourceBytes);
  }
  if (L.type === 'captions') {
    /* The cue showing now, at the density it occupies in this output. Between
       cues there is nothing to draw; an export without burned-in captions
       draws none. */
    if (PM.captionsHidden) return null;
    const r = PM.raster(L, continuousRasterScale(scaledWorld(L, T, W, H)), T, (key: string) => GL.texes.get('r:' + key)?.raster);
    if (!r) return null;
    if (r.blank) frameFailed = true;
    const tex = texFor('r:' + r.key, r.cv, { version: r.blank ? 0 : 1 });
    const uploaded = GL.texes.get('r:' + r.key);
    if (!uploaded.raster && !r.blank) { const { cv: _canvas, ...metadata } = r; uploaded.raster = metadata; }
    return { tex, w: r.w, h: r.h, ax: r.anchorX, ay: r.anchorY, uv: [0, 0, 1, 1] };
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
        && !hasRenderableEffects(L.fx, PM, L, T)
        && !L.masks?.length && !L.matteSource && !L.transitionIn && !L.transitionOut) {
      const plane = is3DLayer(PM, L) ? affinePlane(planeMatrix(PM, L, T)) : null;
      const [sx, sy] = outputScale(W, H);
      const visibleWorld: [number, number, number, number, number, number] | null = plane
        ? [plane[0] * sx, plane[1] * sy, plane[2] * sx, plane[3] * sy, plane[4] * sx, plane[5] * sy]
        : !is3DLayer(PM, L) ? scaledWorld(L, T, W, H) : null;
      if (visibleWorld) {
        const offset = sizeAnchorOffset(PM, L, T);
        visibleWorld[4] += visibleWorld[0] * offset.x + visibleWorld[2] * offset.y;
        visibleWorld[5] += visibleWorld[1] * offset.x + visibleWorld[3] * offset.y;
        if (clip) { visibleWorld[4] -= clip.x; visibleWorld[5] -= clip.y; }
        if (L.type === 'shape') {
          const plan = previewShapeRaster(d, ss, visibleWorld, clip?.width ?? W, clip?.height ?? H);
          if (plan.kind === 'outside') return null;
          if (plan.kind === 'solid') return { solid: PM.hex2rgb(d.color), solidAlpha: PM.hexAlpha(d.color), w: W, h: H, ax: 0, ay: 0, screenSpace: true };
          if (plan.kind === 'crop') crop = plan.window;
        } else if (!L.d.animators?.length && !L.d.styles?.length && !L.d.fontAnchorBounds) {
          const geometry = PM.textRasterGeometry(L, ss, T);
          if (!rasterIntersectsViewport(geometry, visibleWorld, clip?.width ?? W, clip?.height ?? H)) return null;
          crop = previewTextRaster(geometry, visibleWorld, clip?.width ?? W, clip?.height ?? H);
        }
      }
    }
    // CPU bitmaps and uploaded textures have independent memory budgets. A
    // bitmap evicted under CPU pressure need not be drawn and uploaded again
    // while its identical GPU source is still available.
    // Keep small path bitmaps reusable across animated transforms. Oversized
    // plain backgrounds only need visible pixels; a full 8192² bitmap can
    // otherwise evict all the sources prepared for the next scene.
    const clipLargePaths = canClipPreviewSources(W, H) && L.d.paths?.length && !is3DLayer(PM, L)
      && !hasRenderableEffects(L.fx, PM, L, T) && !L.masks?.length && !L.matteSource && !L.transitionIn && !L.transitionOut;
    const r = PM.raster(L, ss, T, (key: string) => GL.texes.get('r:' + key)?.raster, crop,
      clipLargePaths ? Math.min(maxSourceBytes, 4 * 1024 * 1024) : maxSourceBytes);
    if (!r) return clipLargePaths ? viewportPathQuad(L, T, W, H, maxSourceBytes) : null;
    // A recoverable blank is not a completed frame. Its retry has the same
    // source key, so keep a distinct texture version until real pixels arrive.
    if (r.blank) frameFailed = true;
    const tex = texFor('r:' + r.key, r.cv, { version: r.blank ? 0 : 1 });
    const uploaded = GL.texes.get('r:' + r.key);
    // A blank raster is retried by PM.raster; never let it stand in for the bitmap.
    if (!uploaded.raster && !r.fontOffset && !r.blank) {
      const { cv: _canvas, ...metadata } = r;
      uploaded.raster = metadata;
    }
    const ax = L.type === 'shape' && !L.d.paths?.length ? r.w / 2 : r.anchorX;
    const ay = L.type === 'shape' && !L.d.paths?.length ? r.h / 2 : r.anchorY;
    return { tex, w: r.w, h: r.h, ax, ay, uv: r.uv || [0, 0, 1, 1], sourceWindow: r.sourceWindow };
  }
  if (L.type === 'image' || L.type === 'video') {
    const a = PM.assets.get(d.asset);
    if (!a) return null;
    const preparedVideo = idleVideoFrames?.get(videoPath + L.id + '@' + T)
      || PM.preparedVideoFrames?.get(videoPath + L.id+'@'+T)
      || PM.preparedVideoFrames?.get(L.id+'@'+T);
    const liveVideo = L.type === 'video' && useVideoPreviews && !preparedVideo ? layerVideoElement(PM, a, videoPath + L.id) : a.el;
    if (L.type === 'video' && !preparedVideo && (isolatedVideoTexturePrefix || exactInputPreviewRendering || !useVideoPreviews && PM.preparedVideoExactInputs?.project === PM.proj && PM.preparedVideoExactInputs.frames === PM.preparedVideoFrames)) throw new MissingCaptureVideoFrame(`Capture is missing a prepared video frame: ${videoPath + L.id}@${T}`);
    const playbackFrame = PM.playing && useVideoPreviews && !preparedVideo
      ? PM.playbackVideoFrame?.frames.get(videoPath + L.id) : undefined;
    let captured: ReturnType<typeof previewSeekFrame>;
    let decoded: VideoFrame | undefined;
    let el = preparedVideo || playbackFrame?.source || liveVideo, sw = a.w || 1, sh = a.h || 1;
    if (L.type === 'video') {
      const videoTime = useVideoPreviews ? (previewVideoTimes.get(videoPath + L.id) ?? T) : T;
      const vt = videoPlaybackTime(a, sourceTime(PM,L,videoTime));
      if (!preparedVideo && !playbackFrame && useVideoPreviews) captured = previewSeekFrame(liveVideo, vt);
      if (captured) el = captured.canvas;
      if (rememberingPreview && PM.playing && !preparedVideo && !playbackFrame && !captured) {
        // Read the timestamp and upload those same pixels. Never remember a
        // late streaming frame under a newer timeline position.
        try {
          decoded = new VideoFrame(liveVideo);
          const at = decoded.timestamp / 1e6, duration = (decoded.duration || 0) / 1e6;
          if (!(Math.abs(at - vt) < .0005 || at <= vt + .0005 && duration > 0 && vt < at + duration - .0005)) frameVideoExact = false;
        } catch { frameVideoExact = false; }
      }
      if (!PM.playing && el === liveVideo) seekPreviewVideo(el, vt, .0005);
      // A seek canvas and an idle bitmap have the decoded source's dimensions,
      // including proxy rounding. Original metadata must not change their crop.
      sw = el.videoWidth || el.displayWidth || (preparedVideo || captured ? el.width : 0) || sw;
      sh = el.videoHeight || el.displayHeight || (preparedVideo || captured ? el.height : 0) || sh;
    }
    const bw = d.w || W, bh = d.h || H;
    let textureSource = decoded || el;
    let textureKey = 'a:' + a.id + (el===liveVideo?(el===a.el?'':':preview'):':'+L.id+'@'+T);
    if (L.type === 'video' && !preparedVideo) textureKey = videoInstanceTextureKey(liveVideo);
    if (L.type === 'video' && isolatedVideoTexturePrefix) textureKey = isolatedVideoTexturePrefix + videoPath + L.id + '@' + T;
    const shared = L.type === 'video' ? sharedPreparedVideoTexture(preparedVideo, textureKey) : null;
    if (shared) textureKey = shared.key;
    if (L.type === 'image' && a.format === 'svg' && PM.rasterSvgAsset) {
      const dimensions = svgRasterDimensions(sw, sh, bw, bh, scaledWorld(L, T, W, H));
      const raster = PM.rasterSvgAsset(a, dimensions.width, dimensions.height);
      textureSource = raster.cv;
      textureKey = 'r:' + raster.key;
    }
    const videoVersion = shared ? shared.version : L.type === 'video' ? (decoded ? `decoded:${decoded.timestamp}` : playbackFrame ? `play:${playbackFrame.version}` : captured ? `seek:${captured.version}` : el===liveVideo?videoTextureVersion(el):PM.preparedVideoVersion) : 1;
    // A seek can be pending while the previous decoded frame is still usable.
    // Gate on available pixels, not seeking, or seek-driven playback freezes.
    const waiting = L.type === 'video' && el === liveVideo
      && (el.readyState < 2 || !el.videoWidth || !el.videoHeight);
    if (waiting) frameVideoExact = false;
    if (waiting && !GL.texes.get(textureKey)?.bytes) { decoded?.close(); return null; }
    let tex;
    try { tex = texFor(textureKey, textureSource, { version: waiting ? GL.texes.get(textureKey).v : videoVersion }); }
    finally { decoded?.close(); }
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
    // Uniform types belong to the shader source. Inspector metadata starts
    // empty after opening a project and must not decide how colors are bound.
    let uniforms = shaderUniformDefinitions.get(L);
    if (!uniforms || uniforms.code !== d.code) {
      uniforms = { code: d.code, definitions: PM.parseUniforms(d.code || '') };
      shaderUniformDefinitions.set(L, uniforms);
    }
    PM.UIState.setShaderMeta(L, { shaderKey: codeKey, udefs: uniforms.definitions });
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
      const def = uniforms.definitions.find((u: any) => u.name === un);
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
    if(definition?.renderer?.kind==='layer3d'){
      if(definition.renderer.role!=='object')return null;
      try {
        const runtime=compositionRuntime(PM,T),depthIds=compositionDepthIds(PM,L,T);
        const bottom=(PM._renderComposition3DPass || compositionOrderedLayers(PM,(PM._renderComposition3D || PM.proj).layers.filter((l:any)=>l.type!=='group'),T)).filter((l:any)=>depthIds.has(l.id)).at(-1)?.id;
        const canvas=runtime.render(W,H,L.id,depthIds,L.id===bottom,PM._renderCamera3D||runtime.camera);
        const tex=texFor(`layer3d:${L.id}`,canvas,{version:`${runtime.revision}:${W}:${H}:${PM._renderCamera3D?.matrixWorld.elements}:${PM._renderCamera3D?.projectionMatrix.elements}`});
        PM.UIState?.setShaderMeta?.(L,{definition,missing:false,sceneWarnings:runtime.warnings});
        return {tex,w:W,h:H,ax:0,ay:0,uv:[0,0,1,1],fromFbo:false,screenSpace:true};
      }catch(error){PM.UIState?.setShaderMeta?.(L,{definition,missing:true,sceneError:String(error)});frameFailed=true;return null;}
    }
    if(definition?.renderer?.kind==='mesh'){
      const mesh=meshExtensionQuad(L,T,w,hh,targetW,targetH,definition);
      if(mesh!==undefined)return mesh;
    }
    if (definition?.renderer?.kind === 'scene3d') {
      try {
        const runtime = sceneRuntime(PM,L,T);
        if (runtime) {
          const canvas=runtime.render(targetW,targetH);
          const tex=texFor(`scene3d:${L.id}`,canvas,{version:runtime.revision});
          PM.UIState?.setShaderMeta?.(L,{definition,missing:false,sceneWarnings:runtime.warnings});
          return {tex,w,h:hh,ax:0,ay:0,uv:[0,0,1,1],fromFbo:false};
        }
      } catch(error) {
        PM.UIState?.setShaderMeta?.(L,{definition,missing:true,sceneError:String(error)});
        frameFailed=true;
      }
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
    const density = L.continuousRaster ? precompRasterDensity(L, T, w * sx, hh * sy) : 1;
    const targetW = Math.max(2, Math.round(w * sx * density)), targetH = Math.max(2, Math.round(hh * sy * density));
    const target = boundFbo;
    const f = grab(targetW, targetH);
    bind(f); clear(0, 0, 0, 0);
    const previousVideoPath = videoPath;
    videoPath += L.id + '/';
    pmScopePush(sub);
    try {
      const inner = GL.renderProject(sub, sourceTime(PM,L,T), targetW, targetH,
        inputFrameOptions ? { transparent: true, ...inputFrameOptions } : isolatedCaptureNoMotionBlur ? { transparent: true, mblur: false } : { transparent: true });
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
      videoPath = previousVideoPath;
      bind(target);
    }
    return { tex: f.tex, w, h: hh, ax: 0, ay: 0, uv: [0, 0, 1, 1], fromFbo: true, tmp: f };
  }
  return null;
}
/** Continuously rasterize a pre-comp: render the nested composition at the
    scale the layer is displayed at instead of enlarging a 1× frame. Never
    below 1×, and bounded by the GPU's texture size and a pixel budget so a
    huge scale cannot exhaust video memory. */
const PRECOMP_RASTER_PIXELS = 24 * 1024 * 1024;
function precompRasterDensity(L: any, T: number, baseW: number, baseH: number): number {
  const world = is3DLayer(PM, L) ? affinePlane(planeMatrix(PM, L, T)) : PM.worldMatrix(L, T);
  const wanted = Math.max(1, continuousRasterScale(world));
  const side = Math.min(8192, GL.maxTextureSize ||= GL.gl.getParameter(GL.gl.MAX_TEXTURE_SIZE) || 4096);
  const bySide = side / Math.max(1, baseW, baseH);
  const byPixels = Math.sqrt(PRECOMP_RASTER_PIXELS / Math.max(1, baseW * baseH));
  return Math.max(1, Math.min(wanted, bySide, byPixels));
}
/* Scope management for nested rendering — renderProject runs inside these. */
function pmScopePush(proj: any) { PM.scope.push(proj); pcDepth++; }
function pmScopePop() { PM.scope.pop(); pcDepth--; }
function hashStr(s: any) { let h = 5381; for (let i = 0; i < s.length; i++) h = ((h << 5) + h + s.charCodeAt(i)) | 0; return h; }

/* Draw one layer's content (with transform) into the bound target. */
function drawSample(additive: boolean) {
  if (!additive) { draw(); return; }
  const gl = GL.gl;
  gl.blendFunc(gl.ONE, gl.ONE);
  try { draw(); } finally { gl.blendFunc(gl.ONE, gl.ONE_MINUS_SRC_ALPHA); }
}
function drawContent(L: any, T: any, W: any, H: any, alpha: any, clip?: RasterWindow, additive = false, prepared?:any) {
  const c: any = prepared || contentQuad(L, T, W, H, clip);
  if (!c) return false;
  const world = scaledWorld(L, T, W, H);
  const M = c.screenSpace ? [W, 0, 0, H, 0, 0] : PM.mul(world, [c.w, 0, 0, c.h, -c.ax, -c.ay]);
  let projected = m3(M);
  if (!c.screenSpace && is3DLayer(PM,L)) {
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
    // Premultiplied: the colour's own alpha (#RRGGBBAA) scales with the layer's opacity.
    const a = alpha * (c.solidAlpha ?? 1);
    g.u('u_color', c.solid[0] * a, c.solid[1] * a, c.solid[2] * a, a);
    drawSample(additive);
    return true;
  }
  const p = c.sourceWindow ? program('draw-window', PM.FRAG_DRAW_WINDOW) : program('draw', PM.FRAG_DRAW);
  if (!p) return false;
  const g = use(p);
  bindTex(0, c.tex); setI(p, 'u_tex', 0);
  g.u('u_m', projected); g.u('u_res', W, H);
  g.u('u_uv', c.uv[0], c.uv[1], c.uv[2], c.uv[3]);
  if (c.sourceWindow) g.u('u_sourceWindow', ...c.sourceWindow);
  g.u('u_alpha', alpha);
  setI(p, 'u_fromFbo', c.fromFbo ? 1 : 0);
  try { drawSample(additive); }
  finally {
    // Layers draw immediately into the composition buffer. Releasing each
    // private video upload here keeps even a many-video thumbnail from
    // retaining a full RGBA texture for every prepared native frame.
    if (L.type === 'video' && isolatedVideoTexturePrefix) dropIsolatedVideoTexture(isolatedVideoTexturePrefix + videoPath + L.id + '@' + T);
    free(c.tmp);
  }
  return true;
}

/** Mip allocation belongs to the FBO pool's budget and dies with its texture.
 * Restore base-level filtering before a pooled target is reused elsewhere. */
function prepareEffectMips(input: any, levels: number): (() => void) | undefined {
  if (levels <= 0) return;
  const gl = GL.gl;
  bindTex(0, input.tex);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, levels);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR_MIPMAP_NEAREST);
  try {
    gl.generateMipmap(gl.TEXTURE_2D);
    if (levels > (input.mipLevels || 0)) {
      const bytes = mipBytes(input.w, input.h, input.colorDepth, levels);
      const added = bytes - (input.mipBytes || 0);
      input.mipLevels = levels; input.mipBytes = bytes;
      input.bytes += added; poolBytes += added;
    }
  } catch (error) {
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, 0);
    throw error;
  }
  return () => {
    bindTex(0, input.tex);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR);
    gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAX_LEVEL, 0);
  };
}

type InputContext = { project: any; time: number; width: number; height: number; frames: Map<string, any> };
let layerInputs: InputContext | null = null;
let layerInputRendering = 0;
let inputFrameOptions: any = null;
function inputLayerActive(layer: any, time: number, project: any, opt: any): boolean {
  if (opt.inputLayer && (time<0 || time>project.dur+1e-6))return false;
  if (opt.inputLayer) return layer.type === 'group' || time >= layer.from - 1e-6
    && (time < layer.from + layer.dur - 1e-6 || Math.abs(time-project.dur) <= 1e-6 && Math.abs(time-layer.from-layer.dur) <= 1e-6);
  if (!opt.inputVisibilityRoot) return PM.active(layer, time);
  if (!PM.active({...layer, group: null}, time)) return false;
  const ancestors = PM.groupAncestors?.(layer, project.layers) || [];
  const at = ancestors.findIndex((g: any) => g.id === opt.inputVisibilityRoot);
  return !ancestors.slice(0, Math.max(0, at)).some((g: any) => !evaluatedValue(PM,g,g.on,time,'l.on'));
}

/** Typed geometry is evaluated on its provider, then rasterized in owner-local
 * coordinates. Image assets use their complete decoded image, without a layer. */
function referenceImage(owner:any,fx:any,pd:any,ref:any,T:number,W:number,H:number):{tex:any;resolution:number[];lut?:any}|null{
 if(pd.type==='asset'){
  if(!ref?.assetId)return null;const asset=PM.assets.get(ref.assetId);
  if(pd.assetKind==='lut'){const lut=asset?.kind==='lut'&&asset.lut;if(!lut)throw new EffectInputError(`${owner.name}: missing or invalid LUT asset`);if(lut.width>GL.gl.getParameter(GL.gl.MAX_TEXTURE_SIZE)||lut.height>GL.gl.getParameter(GL.gl.MAX_TEXTURE_SIZE)||lut.data.byteLength>(PM.Memory?.budget?.('textures')??128*1024*1024))throw new EffectInputError('LUT exceeds GPU or texture memory limits');return{tex:texFor('effect-lut:'+asset.id,{width:lut.width,height:lut.height,data:lut.data},{version:lut,float:true}),resolution:[lut.width,lut.height],lut};}
  if(!asset||asset.kind!=='image'||!asset.el)throw new EffectInputError(`${owner.name}: missing or unsupported image asset`);
  const width=asset.el.naturalWidth||asset.el.width||asset.w,height=asset.el.naturalHeight||asset.el.height||asset.h;
  if(width>GL.gl.getParameter(GL.gl.MAX_TEXTURE_SIZE)||height>GL.gl.getParameter(GL.gl.MAX_TEXTURE_SIZE)||!Number.isSafeInteger(width*height*4)||width*height*4>(PM.Memory?.budget?.('textures')??128*1024*1024))throw new EffectInputError('Image reference exceeds GPU or texture memory limits');
  return{tex:texFor('effect-asset:'+asset.id,asset.el,{version:asset.el}),resolution:[asset.w,asset.h]};
 }
 if(!ref?.layerId||!ref?.pathId)return null;
 const project=PM.curComp?.()||PM.proj,provider=project.layers.find((l:any)=>l.id===ref.layerId),item=referencedPath(provider,ref.pathId);
 if(!provider||!item)throw new EffectInputError(`${owner.name}: missing path reference`);
 const bytes=W*H*4;if(bytes>Math.min(PM.Memory?.budget?.('raster')??96*1024*1024,PM.Memory?.budget?.('textures')??128*1024*1024))throw new EffectInputError('Path reference exceeds the raster memory budget');
 const canvas=document.createElement('canvas');canvas.width=W;canvas.height=H;const c=canvas.getContext('2d')!;
 try{
  c.transform(...scaledWorld(owner,T,W,H));c.strokeStyle='#FFFFFF';c.lineCap='round';c.lineJoin='round';
  const width=pd.pathWidth?Number(effectParamValue(PM,owner,fx,PM.effectDef(fx).params.find((p:any)=>p.k===pd.pathWidth),T)):1;
  if(pd.pathBrush){const b=pd.pathBrush,get=(key:string)=>effectParamValue(PM,owner,fx,PM.effectDef(fx).params.find((p:any)=>p.k===key),T),curves=brushGeometry(PM,provider,ref,T,!!get(b.allMasks));
   c.setTransform(1,0,0,1,0,0);rasterBrush(canvas,curves,scaledWorld(owner,T,W,H),width,Number(get(b.hardness)),Number(get(b.spacing)),Number(get(b.start)),Number(get(b.end)),!!get(b.sequential));
   return{tex:texFor(`effect-path:${owner.id}:${fx.id}:${pd.k}`,canvas,{version:{}}),resolution:[W,H]};}
  c.lineWidth=Math.max(.001,width);
  const mask=item.mask;
  if(mask){const value=(key:string)=>Number(PM.evP(provider,mask.p[key],T,`m.${mask.id}.${key}`))||0;c.translate(value('x'),value('y'));c.rotate(value('rotation')*Math.PI/180);
   if(!item.path){c.beginPath();if(evaluatedValue(PM,provider,mask.shape,T,`m.${mask.id}.shape`)==='ellipse')c.ellipse(0,0,Math.abs(value('w'))/2,Math.abs(value('h'))/2,0,0,Math.PI*2);else c.rect(-value('w')/2,-value('h')/2,value('w'),value('h'));}
  }
  if(item.path){const v=pathValues(PM,provider,item.path,T,`reference.${item.path.id}`);
   if(mask){c.translate(v.x,v.y);c.rotate(v.rotation*Math.PI/180);c.scale(v.scaleX/100,v.scaleY/100);}else c.transform(...groupMatrix(PM,provider,item.path,T) as [number,number,number,number,number,number]);
   for(let copy=0;copy<Math.min(256,Math.max(1,Math.round(v.copies??1)));copy++){c.save();c.translate(copy*(v.repeatX??0),copy*(v.repeatY??0));c.rotate(copy*(v.repeatRotation??0)*Math.PI/180);tracePath(c,v);if(width>0)c.stroke();c.restore();}
  }else if(width>0)c.stroke();
  return{tex:texFor(`effect-path:${owner.id}:${fx.id}:${pd.k}`,canvas,{version:{}}),resolution:[W,H]};
 }finally{canvas.width=0;canvas.height=0;}
}

/** A temporal effect consumes immutable owner-prefix frames. Composition
 * samples align provider-time layer coordinates to the current owner: moving
 * the outer layer alone does not create an Echo trail. Source owners use actual
 * extent metadata so animated dimensions/padding do not stretch old frames. */
function runTemporalEffect(L:any,fx:any,def:any,index:number,T:number,src:any,W:number,H:number){
 if(is3DLayer(PM,L))throw new EffectInputError(`${L.name}: temporal sampling requires a 2D content owner`);
 const space=activeSourceRaster()?.layer===L?'source':'composition';
 const times=[T,...temporalTimes(def,key=>effectParamValue(PM,L,fx,def.params.find((p:any)=>p.k===key),T),T)];
 const p=program(effectProgramKey(fx.type,fx.version,PM.Kernel?.effects?.get?.(fx.type)),def.frag);
 if(!p)return src;
 const owner=effectLayerCoordinates(L,T,W,H),raster=activeSourceRaster();
 if(raster?.domain)raster.domain=sourceEffectDomain(L,fx,def,T,raster.domain);
 let acc:any=null;
 try{
  for(let sample=0;sample<times.length;sample++){
   if(inputFrameOptions?.signal?.aborted||GL.contextLost||GL.gl.isContextLost?.())throw new EffectInputError('Temporal rendering cancelled or context lost');
   const at=times[sample]!,frame=sample===0?src:layerInputs?.frames.get(inputKey(L.id,'effects',space,at,index));
   if(!frame)throw new EffectInputError(`${L.name}: temporal owner prefix is unavailable at ${at}`);
   let from:Float32Array|Parameters<typeof inversePlane>[0]=IDENT,valid=owner.valid!==false;
   if(space==='composition'&&sample>0){
    const historical=effectLayerCoordinates(L,at,frame.w,frame.h);from=inversePlane(Array.from(historical.uv) as unknown as Parameters<typeof inversePlane>[0])||IDENT;valid=valid&&historical.valid!==false;
   }
   const out=grab(W,H);bind(out);clear();const g=use(p);
   bindTex(0,acc?.tex||src.tex);setI(p,'u_tex',0);bindTex(1,frame.tex);setI(p,'u_temporal',1);
   g.u('u_m',fullQuad(W,H));g.u('u_res',W,H);g.u('u_uv',0,0,1,1);g.u('u_time',T);
   g.u('u_temporalIndex',sample);g.u('u_temporalCount',times.length);g.u('u_temporalValid',valid?1:0);
   g.u('u_temporalSource',space==='source'?1:0);g.u('u_temporalDirect',sample===0?1:0);
   const currentBounds=GL.bounds(L,T),pastBounds=GL.bounds(L,at);
   g.u('u_temporalBounds',...(currentBounds?[currentBounds.x0,currentBounds.y0,currentBounds.w,currentBounds.h]:[0,0,W,H]));
   g.u('u_temporalPriorBounds',...(pastBounds?[pastBounds.x0,pastBounds.y0,pastBounds.w,pastBounds.h]:[0,0,frame.w,frame.h]));
   g.u('u_temporalOwnerUV',owner.uv);g.u('u_temporalFromUV',from);g.u('u_temporalRes',frame.w,frame.h);
   const extent=raster?.extent??raster?.bounds,prior=frame.sourceExtent;
   g.u('u_temporalExtent',...(extent?[extent.x0,extent.y0,extent.w,extent.h]:[0,0,W,H]));
   g.u('u_temporalPriorExtent',...(prior?[prior.x0,prior.y0,prior.w,prior.h]:[0,0,frame.w,frame.h]));
   def.params.forEach((pd:any,i:number)=>setParam(p,pd,i,effectParamValue(PM,L,fx,pd,T)));
   GL.gl.disable(GL.gl.BLEND);try{draw();}finally{GL.gl.enable(GL.gl.BLEND);}
   GL.stats.passes++;if(acc)free(acc);acc=out;
   if(inputFrameOptions?.signal?.aborted||GL.contextLost||GL.gl.isContextLost?.())throw new EffectInputError('Temporal rendering cancelled or context lost');
  }
  return acc;
 }catch(error){free(acc);throw error;}
}

/* ── effects chain ─────────────────────────────────────── */
function runEffects(L: any, T: any, srcF: any, W: any, H: any, backdropF: any = null, prefix?:number) {
  let cur = srcF;
  const coordinates = effectViewportCoordinates(W, H);
  let layerCoordinates: ReturnType<typeof effectLayerCoordinates> | undefined;
  for (const [effectIndex,fx] of L.fx.entries()) {
    if(prefix!==undefined&&effectIndex>=prefix)break;
    /* `missing` marks an effect whose type is not registered (the extension
       providing it is off or gone). Skip it and keep its data intact. */
    if (!evaluatedValue(PM, L, fx.on, T, `${fx.id}.$enabled`) || fx.missing) continue;
    const def = PM.effectDef?.(fx) ?? (!PM.effectDef ? PM.FX[fx.type] : undefined);
    if (!def) continue;
    if(def.temporal){const next=runTemporalEffect(L,fx,def,effectIndex,T,cur,W,H);if(cur!==srcF)free(cur);cur=next;continue;}
    const raster=activeSourceRaster();
    if(raster?.domain)raster.domain=sourceEffectDomain(L,fx,def,T,raster.domain);
    const key = effectProgramKey(fx.type, fx.version, PM.Kernel?.effects?.get?.(fx.type));
    const p = program(key, def.frag);
    if (!p) continue;
    const usesLayerCoordinates = uloc(p, 'u_layerUV') !== null;
    if (usesLayerCoordinates) layerCoordinates ??= effectLayerCoordinates(L, T, W, H);
    const inputParams = def.params.filter((pd:any)=>referenceParam(pd));
    if (inputParams.length + 3 > GL.gl.getParameter(GL.gl.MAX_TEXTURE_IMAGE_UNITS)) throw new EffectInputError('Too many effect input textures for this GPU');
    const assets=new Set<string>();const bytes=inputParams.reduce((sum:number,pd:any)=>{const ref=fx.inputs?.[pd.k];if(pd.type==='path'&&ref?.pathId)return sum+W*H*4;if(pd.type==='asset'&&ref?.assetId&&!assets.has(ref.assetId)){assets.add(ref.assetId);const a=PM.assets.get(ref.assetId);return sum+(a?.lut?.data.byteLength??(a?.w||0)*(a?.h||0)*4);}return sum;},0);if(bytes>(PM.Memory?.budget?.('textures')??128*1024*1024))throw new EffectInputError('Effect references exceed the texture memory budget');
    const orig = def.keepOrig ? cur : null;
    const scale = outputScale(W, H);
    const world = def.sampling?.space === 'layer' ? activeSourceRaster()?.layer===L ? [1,0,0,1,0,0] : PM.worldMatrix(L, T) : undefined;
    const kernelScale = world ? [Math.hypot(world[0] * scale[0], world[1] * scale[1]), Math.hypot(world[2] * scale[0], world[3] * scale[1])] : scale;
    const parameter = (name: string) => {
      const pd = def.params.find((param: any) => param.k === name);
      return pd ? effectParamValue(PM, L, fx, pd, T) : 0;
    };
    const grid = def.grid ? gridSize(Number(parameter(def.grid.columns)), Number(parameter(def.grid.rows)),
      Math.min(2048, GL.maxTextureSize ||= GL.gl.getParameter(GL.gl.MAX_TEXTURE_SIZE) || 2048)) : undefined;
    let input = cur;
    for (let pass = 0; pass < def.passes; pass++) {
      const targetW = grid && pass === 0 ? grid[0] : W, targetH = grid && pass === 0 ? grid[1] : H;
      const out = grab(targetW, targetH);
      bind(out); clear();
      const g = use(p);
      bindTex(0, input.tex); setI(p, 'u_tex', 0);
      let lod = def.sampling && pass >= (def.sampling.firstPass ?? 0) ? sampleLod(Number(parameter(def.sampling.radius)), def.sampling.support, def.sampling.maxRadius, kernelScale, input.w, input.h) : 0;
      if (grid && pass === 0 && !(def.grid.pointSample && parameter(def.grid.pointSample))) lod = Math.floor(Math.log2(Math.max(input.w, input.h)));
      const resetSampling = prepareEffectMips(input, lod);
      try {
        if (orig) { bindTex(1, orig.tex); setI(p, 'u_orig', 1); }
        if (def.backdrop) { bindTex(2, backdropF?.tex || null); setI(p, 'u_backdrop', 2); }
        g.u('u_m', fullQuad(targetW, targetH)); g.u('u_res', targetW, targetH); g.u('u_uv', ...coordinates.uv);
        g.u('u_coordRes', ...coordinates.resolution); g.u('u_pxOrigin', ...coordinates.origin);
        g.u('u_pixelScale', ...scale); g.u('u_sourceRes', W, H); g.u('u_sampleLod', lod);
        if (grid) g.u('u_gridRes', ...grid);
        if (inputParams.length) {
          const owner = effectLayerCoordinates(L, T, W, H);
          g.u('u_ownerFromUV', inversePlane(Array.from(owner.uv) as unknown as Parameters<typeof inversePlane>[0]) || IDENT);
          inputParams.forEach((pd: any, index: number) => {
            const ref=fx.inputs?.[pd.k];
            if(pd.type==='asset'||pd.type==='path'){
              const image=referenceImage(L,fx,pd,ref,T,W,H);
              bindTex(index+3,image?.tex||input.tex);setI(p,'u_'+pd.k,index+3);g.u('u_has_'+pd.k,image?1:0);
              g.u('u_'+pd.k+'Res',...(image?.resolution??owner.resolution));
              if(pd.assetKind==='lut'){g.u('u_'+pd.k+'Size',image?.lut?.size??2);g.u('u_'+pd.k+'DomainMin',...(image?.lut?.domainMin??[0,0,0]));g.u('u_'+pd.k+'DomainMax',...(image?.lut?.domainMax??[1,1,1]));}
              const forward=new Float32Array([W,0,0,0,-H,0,0,H,1]);g.u('u_'+pd.k+'FromUV',forward);g.u('u_'+pd.k+'UV',new Float32Array(inversePlane(Array.from(forward) as unknown as Parameters<typeof inversePlane>[0])!));g.u('u_'+pd.k+'Midpoint',128/255);return;
            }
            const frame = ref?.layerId ? layerInputs?.frames.get(inputKey(ref.layerId, ref.stage, ref.space,T)) : null;
            if (ref?.layerId && !frame) throw new EffectInputError(`${L.name}: input ${pd.label} is unavailable`);
            bindTex(index + 3, frame?.tex || input.tex); setI(p, 'u_' + pd.k, index + 3);
            g.u('u_has_' + pd.k, frame ? 1 : 0);
            g.u('u_'+pd.k+'Midpoint',(frame?.colorDepth??workingColorDepth())===8?128/255:.5);
            const target = ref?.layerId && layerInputs?.project.layers.find((l: any) => l.id === ref.layerId);
            let mapping=owner;
            if(target&&frame){
              if(ref.space==='source')mapping={uv:new Float32Array([1/W,0,0,0,1/H,0,0,0,1]),resolution:frame.inputResolution};
              else {
                const previous=sourceRaster;sourceRaster=null;
                try{mapping=effectLayerCoordinates(target,T,frame.w,frame.h,true);}finally{sourceRaster=previous;}
                // The shared sampler addresses normalized texture coordinates
                // through the consumer's u_res, even for a source-sized owner.
                const forward=inversePlane(Array.from(mapping.uv) as unknown as Parameters<typeof inversePlane>[0]);
                if(forward&&(frame.w!==W||frame.h!==H)){const scaled=Array.from(forward).map((v,i)=>v*(i%3===0?W/frame.w:i%3===1?H/frame.h:1));mapping={...mapping,uv:new Float32Array(inversePlane(scaled as unknown as Parameters<typeof inversePlane>[0])!)};}
              }
            }
            g.u('u_' + pd.k + 'UV', mapping.uv);
            g.u('u_' + pd.k + 'FromUV', inversePlane(Array.from(mapping.uv) as unknown as Parameters<typeof inversePlane>[0]) || IDENT);
            g.u('u_' + pd.k + 'Res', ...mapping.resolution);
          });
        }
        // Uploading path/asset references can bind texture unit zero. Restore
        // the actual effect source before sampling or the first frame differs
        // from subsequent cached uploads (and from save/load).
        if(inputParams.length)bindTex(0,input.tex);
        if (usesLayerCoordinates) {
          g.u('u_layerUV', layerCoordinates!.uv);
          g.u('u_layerRes', ...layerCoordinates!.resolution);
          g.u('u_ownerValid',layerCoordinates!.valid===false?0:1);
        }
        g.u('u_texel', 1 / input.w, 1 / input.h);
        g.u('u_time', T);
        setI(p, 'u_pass', pass);
        def.params.forEach((pd: any, i: any) => setParam(p, pd, i, effectParamValue(PM, L, fx, pd, T)));
        const domain=activeSourceRaster()?.domain,extent=activeSourceRaster()?.extent;
        if(domain&&extent){const sx=targetW/extent.w,sy=targetH/extent.h;GL.gl.enable(GL.gl.SCISSOR_TEST);GL.gl.scissor(Math.floor((domain.x0-extent.x0)*sx),Math.floor((extent.y0+extent.h-domain.y0-domain.h)*sy),Math.ceil(domain.w*sx),Math.ceil(domain.h*sy));}
        GL.gl.disable(GL.gl.BLEND);
        try { gpuTiming ? gpuTiming.measure({ id: `effect:${L.id}:${fx.id}:${pass}`, name: `${L.name} · ${def.label || fx.type}`, kind: 'effect', layerId: L.id }, draw) : draw(); }
        finally { GL.gl.enable(GL.gl.BLEND);if(domain)GL.gl.disable(GL.gl.SCISSOR_TEST); }
        GL.stats.passes++;
      } finally { resetSampling?.(); }
      if (input !== cur) free(input);
      input = out;
    }
    if (cur !== srcF) free(cur);
    cur = input;
  }
  return cur;
}

function sourceEffectPadding(L:any,fx:any,def:any,T:number){
 const contract=def.sourceProcessing;if(!contract||def.backdrop)throw new EffectInputError(`${L.name}: ${def.label} does not declare source processing support`);
 const value=(key:string)=>effectParamValue(PM,L,fx,def.params.find((p:any)=>p.k===key),T);
 if(contract.when&&!value(contract.when))return 0;
 return Math.ceil((contract.pixels??0)+(contract.padding??[]).reduce((sum:number,key:string)=>sum+Math.abs(Number(value(key))||0),0)*(contract.factor??1));
}
function sourceEffectDomain(L:any,fx:any,def:any,T:number,bounds:any){
 if(def.temporal){
  let result={...bounds};const index=L.fx.indexOf(fx);
  for(const at of temporalTimes(def,key=>effectParamValue(PM,L,fx,def.params.find((p:any)=>p.k===key),T),T)){
   const extent=layerInputs?.frames.get(inputKey(L.id,'effects','source',at,index))?.sourceExtent;if(!extent)continue;
   const x=Math.min(result.x0,extent.x0),y=Math.min(result.y0,extent.y0);result={x0:x,y0:y,w:Math.max(result.x0+result.w,extent.x0+extent.w)-x,h:Math.max(result.y0+result.h,extent.y0+extent.h)-y};
  }return result;
 }
 const pad=sourceEffectPadding(L,fx,def,T);let result={x0:bounds.x0-pad,y0:bounds.y0-pad,w:bounds.w+2*pad,h:bounds.h+2*pad};
 for(const key of def.sourceProcessing.geometry??[]){const pd=def.params.find((p:any)=>p.k===key),ref=fx.inputs?.[key];if(!ref?.layerId||!ref.pathId)continue;const provider=(PM.curComp?.()??PM.proj).layers.find((l:any)=>l.id===ref.layerId);if(!provider)throw new EffectInputError('Missing stroke path provider');const value=(k:string)=>effectParamValue(PM,L,fx,def.params.find((p:any)=>p.k===k),T),b=brushBounds(brushGeometry(PM,provider,ref,T,!!value(pd.pathBrush.allMasks)),Number(value(pd.pathWidth)));if(b){const x=Math.min(result.x0,b.x0),y=Math.min(result.y0,b.y0);result={x0:x,y0:y,w:Math.max(result.x0+result.w,b.x0+b.w)-x,h:Math.max(result.y0+result.h,b.y0+b.h)-y};}}
 return result;
}
function sourceOwnerEffects(L:any,T:number,prefix?:number){return(L.fx??[]).slice(0,prefix).filter((fx:any)=>!fx.missing&&evaluatedValue(PM,L,fx.on,T,`${fx.id}.$enabled`)).map((fx:any)=>({fx,def:PM.effectDef(fx)})).filter((item:any)=>item.def);}
/** Source-space references retain native content bounds before layer transforms,
 * opacity and composition clipping. Every lease belongs to the enclosing DAG. */
function renderInputSource(project:any,L:any,T:number,stage:string,opt:any) {
  const previous=sourceRaster,previousClip=previewSourceClipping,previousViewport=previewViewportActive;
  const bounds=GL.bounds(L,T)||{x0:0,y0:0,w:1,h:1};
  const checkedSize=(w:number,h:number)=>{
    const limit=GL.maxTextureSize||GL.gl.getParameter(GL.gl.MAX_TEXTURE_SIZE);
    if(!Number.isFinite(w*h)||w<=0||h<=0||Math.ceil(w)>limit||Math.ceil(h)>limit)throw new EffectInputError(`${L.name}: source raster exceeds GPU dimensions`);
  };
  checkedSize(bounds.w,bounds.h);
  sourceRaster={layer:L,project,bounds};previewSourceClipping=false;previewViewportActive=false;
  const check=()=>{if(opt.signal?.aborted||GL.contextLost||!GL.gl||GL.gl.isContextLost?.())throw new EffectInputError('Layer input rendering cancelled or context lost');};
  try{
    check();
    // Inactive video inputs need no decoded frame and remain transparent.
    if(!inputLayerActive(L,T,project,{inputLayer:L.id})) {const frame=grab(1,1);bind(frame);clear();frame.inputResolution=[bounds.w,bounds.h];frame.sourceExtent=bounds;return frame;}
    const content=contentQuad(L,T,Math.ceil(bounds.w),Math.ceil(bounds.h));
    check();
    if(content?.screenSpace)throw new EffectInputError(`${L.name}: screen-space content cannot supply an unclipped source raster`);
    const full=content?{x0:-content.ax,y0:-content.ay,w:content.w,h:content.h}:bounds;
    try{checkedSize(full.w,full.h);}catch(error){free(content?.tmp);throw error;}
    const grow=stage==='effects'&&(opt.ownerGrow||L.effectSpace==='source');
    const extent=grow?sourceOwnerEffects(L,T,opt.inputPrefix).reduce((bounds:any,item:any)=>sourceEffectDomain(L,item.fx,item.def,T,bounds),full):full;checkedSize(extent.w,extent.h);
    sourceRaster={layer:L,project,bounds:full,...(grow?{extent,domain:{...full}}:{})};
    const W=Math.ceil(extent.w),H=Math.ceil(extent.h),src=grab(W,H);bind(src);clear();
    if(content)drawContent(L,T,W,H,1,undefined,false,content);
    check();
    let result=src;
    if(stage!=='source'){result=applyMasks(L,T,result,W,H);if(result!==src)free(src);}
    check();
    if(stage==='effects'){const prior=result;result=runEffects(L,T,prior,W,H,null,opt.inputPrefix);if(result!==prior)free(prior);}
    check();
    result.inputResolution=[extent.w,extent.h];result.sourceExtent=extent;return result;
  }finally{sourceRaster=previous;previewSourceClipping=previousClip;previewViewportActive=previousViewport;}
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
  const retained = layerInputs?.project === proj ? layerInputs!.frames.get(inputKey(source.id, 'effects','composition',T)) : null;
  const matte=retained || GL.renderProject({...proj,layers:[{...source,on:true}]},T,W,H,{...opt,transparent:true,mattePass:true,explicitLayers:true,matteDepth:(opt.matteDepth||0)+1,matteProject:proj});
  const out=grab(W,H);bind(out);clear();const p=program('trackMatte',PM.GLSL_PRE+'uniform sampler2D u_matte; uniform int u_luma; uniform int u_invert; void main(){vec4 m=texture(u_matte,v_st);float a=u_luma==1?dot(m.rgb,vec3(.2126,.7152,.0722)):m.a;if(u_invert==1)a=1.-a;o=texture(u_tex,v_st)*clamp(a,0.,1.);}');
  const g=use(p);bindTex(0,srcF.tex);setI(p,'u_tex',0);bindTex(1,matte.tex);setI(p,'u_matte',1);setI(p,'u_luma',mode.startsWith('luma')?1:0);setI(p,'u_invert',mode.endsWith('inverted')?1:0);g.u('u_m',fullQuad(W,H));g.u('u_res',W,H);g.u('u_uv',0,0,1,1);GL.gl.disable(GL.gl.BLEND);draw();GL.gl.enable(GL.gl.BLEND);if(!retained)free(matte);return out;
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

function drawFbo(srcF: any, W: any, H: any, alpha = 1, additive = false) {
  const p = program('copyA', PM.FRAG_DRAW);
  if (!p) return false;
  const g = use(p);
  bindTex(0, srcF.tex); setI(p, 'u_tex', 0);
  g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
  g.u('u_alpha', alpha); setI(p, 'u_fromFbo', 1);
  drawSample(additive);
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
  const adjusted = runEffects(L, T, acc, W, H, acc);
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
  if (!L.transitionIn && !L.transitionOut) return null;
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
  return compositingLayers(layers, parentGroup, layer => groupCreatesCompositingBoundary(layer, T, opt));
}

/** Plan thumbnail source frames with the same passes, eligibility and scope
 * seen by renderProject. This visits no GPU resources or decoder state. */
GL.frameVideoRequests = (comp: any, time: number, options: any = {}) => {
  const requests = new Map<string, { key: string; layer: any; time: number; asset: any; at: number }>();
  let visits=0;
  const visit = (proj: any, T: number, opt: any, depth: number, path: string) => {
    if(options.signal?.aborted)throw new EffectInputError('Temporal video preparation cancelled');
    if(++visits>8192||requests.size>1024)throw new EffectInputError('Temporal video requests exceed the frame request limit');
    if (depth > PC_MAX_DEPTH) throw new EffectInputError('Composition nesting is too deep');
    if (!opt.preparedInputs && hasLayerInputs(proj, (fx:any)=>PM.effectDef?.(fx) ?? PM.Kernel?.effects?.get?.(fx.type))) {
      const plan = layerInputPlan(proj, {assets:PM.assets,definition:(fx:any)=>PM.effectDef?.(fx),time:T,value:(l:any,fx:any,key:string,at:number)=>effectParamValue(PM,l,fx,PM.effectDef(fx).params.find((p:any)=>p.k===key),at),
        enabled:(l:any,fx:any,at=T)=>!!evaluatedValue(PM,l,fx.on,at,`${fx.id}.$enabled`),
        active:(l:any,at=T)=>!!PM.active(l,at),contentActive:(l:any,at=T)=>!!PM.active({...l,group:null},at)});
      for (const node of plan.requests) visit(proj,node.time!,{...opt,inputLayer:node.layer.id,inputStage:node.stage,inputPrefix:node.prefix,
        preparedInputs:true,mblur:false,inputSpace:node.space},depth,path);
      opt={...opt,preparedInputs:true};
    }
    const orderedLayers=proj.layers||[];
    const layers=opt.inputLayer ? orderedLayers.filter((l:any)=>l.id===opt.inputLayer)
      : opt.explicitLayers ? orderedLayers : compositingPass(orderedLayers,opt.groupParent||null,T,opt);
    const solo=orderedLayers.some((l:any)=>l.solo), matteSources=new Set(orderedLayers.map((l:any)=>l.matteSource));
    const content = (L:any,at:number) => {
      if (L.type==='video') {
        const asset=PM.assets.get(resolveContent(PM,L,at).asset),key=path+L.id+'@'+at;
        if(asset?.el){requests.set(key,{key,layer:L,time:at,asset,at:videoPlaybackTime(asset,sourceTime(PM,L,at),1/PM.proj.fps)});if(requests.size>1024)throw new EffectInputError('Temporal video requests exceed the frame request limit');}
      } else if(L.type==='precomp') {
        const sub=PM.compOf(L);if(!sub)return;
        PM.scope.push(sub);
        try{visit(sub,sourceTime(PM,L,at),{transparent:true,mblur:opt.mblur!==false,mbSamples:opt.mbSamples,shutter:opt.shutter},depth+1,path+L.id+'/');}
        finally{PM.scope.pop();}
      }
    };
    for(let index=layers.length-1;index>=0;index--){
      const L=layers[index],ancestors=PM.groupAncestors?.(L,proj.layers)||[];
      if(!opt.inputLayer&&!opt.mattePass&&matteSources.has(L.id))continue;
      if(!opt.inputLayer&&!opt.inputVisibilityRoot&&solo&&!L.solo&&!ancestors.some((g:any)=>g.solo)
        &&!groupContainsSolo(L,orderedLayers,(l:any)=>PM.groupAncestors?.(l,proj.layers)||[])&&!opt.mattePass)continue;
      if(L.type!=='group'&&PM.TYPE_META?.[L.type]?.visual===false)continue;
      if(!inputLayerActive(L,T,proj,opt)||opt.inputSpace!=='source'&&PM.clamp(PM.ev(L,'opacity',T)/100,0,1)<=.001)continue;
      const mb=evaluatedValue(PM,L,L.mblur,T,'l.mblur')&&opt.mblur!==false;
      if(L.type==='group'){
        const times=mb?motionBlurTimes(T,proj.fps,opt):[T];
        for(const at of times)visit(proj,at,{...opt,inputLayer:undefined,inputStage:undefined,inputPrefix:undefined,
          inputVisibilityRoot:opt.inputLayer||opt.inputVisibilityRoot,groupParent:L.id,mblur:mb?false:opt.mblur,
          preparedInputs:at===T&&opt.preparedInputs},depth,path);
      }else if(L.effectSpace==='source'&&mb){for(const at of motionBlurTimes(T,proj.fps,opt))visit(proj,at,{...opt,inputLayer:L.id,inputStage:'effects',preparedInputs:false,mblur:false},depth,path);}
      else for(const at of mb?motionBlurTimes(T,proj.fps,opt):[T])content(L,at);
      if(L.matteSource&&opt.inputPrefix===undefined&&(!opt.inputLayer||opt.inputStage==='effects')&&(opt.matteDepth||0)<8){
        const matteProject=opt.matteProject||proj,source=matteProject.layers.find((l:any)=>l.id===L.matteSource);
        if(source)visit(matteProject,T,{...opt,inputLayer:source.id,inputStage:'effects',mblur:false,
          mattePass:true,matteDepth:(opt.matteDepth||0)+1,preparedInputs:true},depth,path);
      }
    }
  };
  PM.beginEval?.(time);PM.scope.push(comp);
  try{visit(comp,time,options,0,'');}finally{PM.scope.pop();PM.beginEval?.(PM.time);}
  return [...requests.values()];
};
GL.thumbnailVideoRequests = (comp:any,time:number) => GL.frameVideoRequests(comp,time,{mblur:false});

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
/* One layer that throws must never black out the frame: every other layer
   still composites, and the failure is reported once per layer with enough
   detail to reproduce. Rendering resumes for that layer as soon as its
   source stops throwing. */
const reportedLayerFailures = new Map<string, string>();
function reportLayerFailure(layer: any, error: unknown): void {
  frameFailed = true;
  const message = error instanceof Error ? (error.stack || error.message) : String(error);
  if (reportedLayerFailures.get(layer.id) === message) return;
  reportedLayerFailures.set(layer.id, message);
  console.error(`[compositor] layer failed to render: ${layer.name ?? layer.id} (${layer.type})`, { id: layer.id, content: layer.d }, error);
  PM.toast?.(`${layer.name ?? 'A layer'} could not be rendered. See the console for details.`);
}

GL.renderProject = (proj: any, T: any, W: any, H: any, opt: any = {}) => {
  if(proj===PM.proj && !opt.groupParent && !opt.mattePass)pruneSceneRuntimes(PM);
  const previous3DComposition=PM._renderComposition3D,previous3DPass=PM._renderComposition3DPass,previous3DCamera=PM._renderCamera3D;PM._renderComposition3D=proj;
  const previousInputs = layerInputs, previousInputFrameOptions = inputFrameOptions;
  if (inputFrameOptions || hasLayerInputs(proj, (fx:any)=>PM.effectDef?.(fx) ?? PM.Kernel?.effects?.get?.(fx.type))) inputFrameOptions = { signal:opt.signal, mblur: opt.mblur, mbSamples: opt.mbSamples, shutter: opt.shutter };
  const ownsInputs = !opt.inputContext && hasLayerInputs(proj, (fx:any)=>PM.effectDef?.(fx) ?? PM.Kernel?.effects?.get?.(fx.type));
  const baseline = ownsInputs ? new Set(GL.pool.filter((f: any) => f.busy)) : null;
  let completed = false;
  let ownedInputs: InputContext | null = null;
  try {
  if (opt.signal?.aborted) throw new EffectInputError('Layer input rendering cancelled');
  if (ownsInputs) {
    const plan = layerInputPlan(proj, { assets:PM.assets, definition: (fx: any) => PM.effectDef?.(fx),time:T,value:(l:any,fx:any,key:string,at:number)=>effectParamValue(PM,l,fx,PM.effectDef(fx).params.find((p:any)=>p.k===key),at),
      enabled: (l: any, fx: any, at=T) => !!evaluatedValue(PM, l, fx.on, at, `${fx.id}.$enabled`),
      active: (l: any,at=T) => !!PM.active(l, at), contentActive: (l: any,at=T) => !!PM.active({...l, group: null}, at) });
    const depth=workingColorDepth(),budget=PM.Memory?.budget?.('framebuffers')??MAX_FBO_BYTES;
    const sourceBytes=plan.requests.filter(n=>n.space==='source').reduce((sum,n)=>{const b=GL.bounds(n.layer,n.time);return sum+colorBufferBytes(Math.max(1,Math.ceil(b?.w||1)),Math.max(1,Math.ceil(b?.h||1)),depth);},0);
    assertInputBudget(plan.requests.filter(n=>n.space!=='source').length,colorBufferBytes(W,H,depth),budget-sourceBytes);
    ownedInputs = layerInputs = { project: proj, time: T, width: W, height: H, frames: new Map() };
    layerInputRendering++;
    for (const node of plan.requests) {
      if (opt.signal?.aborted || GL.contextLost || GL.gl.isContextLost?.()) throw new EffectInputError('Layer input rendering cancelled or context lost');
      const frame = node.space==='source' ? renderInputSource(proj,node.layer,node.time!,node.stage,{...opt,inputPrefix:node.prefix}) : GL.renderProject(proj, node.time, W, H, { ...opt, rawOwner:undefined, inputContext: layerInputs, inputLayer: node.layer.id, inputStage: node.stage, inputPrefix:node.prefix, transparent: true, previewReuse: false, mblur: false });
      frame.inputTime=node.time;frame.inputPrefix=node.prefix;layerInputs.frames.set(node.key, frame);
    }
  } else if (opt.inputContext) layerInputs = opt.inputContext;
  if (opt.signal?.aborted || ownsInputs && GL.contextLost) throw new EffectInputError('Layer input rendering cancelled or context lost');
  if(opt.rawOwner){const layer=proj.layers.find((l:any)=>l.id===opt.rawOwner);if(!layer||layer.threeD||['group','adjustment','audio','null'].includes(layer.type))throw new EffectInputError('Source owner processing requires a 2D content layer');const frame=renderInputSource(proj,layer,T,'effects',{...opt,ownerGrow:true});completed=true;return frame;}
  PM._renderCamera3D=opt.editorViewport&&(proj===PM.proj||opt.matteProject===PM.proj)&&getViewportMode(PM)==='editor'?viewportCamera(PM,compositionRuntime(PM,T,PM.proj).camera):null;
  const gl = GL.gl;
  const orderedLayers = proj.layers;
  const parentGroup = typeof opt.groupParent === 'string' ? opt.groupParent : null;
  const layers = compositionOrderedLayers(PM,opt.inputLayer ? orderedLayers.filter((l: any) => l.id === opt.inputLayer) : opt.explicitLayers ? orderedLayers : compositingPass(orderedLayers, parentGroup, T, opt),T,PM._renderCamera3D);
  PM._renderComposition3DPass=layers;
  const solo = orderedLayers.some((layer: any) => layer.solo);
  // Rebuild per composition/pass so edits and nested mattes cannot leave a
  // stale index. One scan replaces a full stack scan for every rendered layer.
  const matteSources = new Set(orderedLayers.map((layer: any) => layer.matteSource));
  // Keep the invariant lower stack resident. An animated foreground then
  // costs one GPU copy plus its own draws, regardless of background density.
  let prefixStart = layers.length, prefixKey: string | null = null;
  // Only the live preview has refreshed the temporal plan and cache signature.
  // Captures can request another size without resizing the viewer; reusing a
  // preview prefix there would rescale stale or low-resolution artwork.
  if (rememberingPreview && proj === PM.proj && !opt.groupParent && !opt.mattePass && canCachePreview(opt)
      && !solo && scenePreviewPlan && !orderedLayers.some((layer: any) => layer.threeD || layer.matteSource)) {
    const radius = previewHasMotionBlur && opt.mblur !== false ? (opt.shutter || .5) / Math.max(1, proj.fps) : 0;
    prefixStart = scenePreviewPlan.prefixStart(layers, T, radius);
    const interval = scenePreviewPlan.interval(T, radius);
    if (layers.length - prefixStart >= 4 && interval !== null)
      prefixKey = 'prefix:' + previewFrameKey(T, opt, `${prefixStart}:${interval}`);
  }
  const prefix = prefixKey ? previewFrames.get(prefixKey) : undefined;
  // Retained pixels still depend on their shaders. Count those programs as
  // active so a dense prefix does not evict itself at the end of this frame.
  if (prefix) for (const key of prefix.programKeys || []) framePrograms.add(key);

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
          || PM.worldOpacity(layer, T) < 1) continue;
      const d = resolveContent(PM, layer, T), m = scaledWorld(layer, T, W, H);
      if (d.shape !== 'rect' || !/^#[0-9a-f]{6}$/i.test(d.color) || Math.abs(m[1]) > 1e-9 || Math.abs(m[2]) > 1e-9) continue;
      const geometry = shapeRasterGeometry(d, continuousRasterScale(m));
      const inset = cornerInset(d)
        + Math.max(0, Number(d.stroke) || 0) / 2 + 8 / geometry.density;
      const hw = (d.w / 2 - inset) * Math.abs(m[0]), hh = (d.h / 2 - inset) * Math.abs(m[3]);
      const x = Math.max(0, Math.ceil(m[4] - hw)), y = Math.max(0, Math.ceil(m[5] - hh));
      const width = Math.min(W, Math.floor(m[4] + hw)) - x, height = Math.min(H, Math.floor(m[5] + hh)) - y;
      if (width > 0 && height > 0 && width * height > area) { largest = {x,y,width,height}; area = width * height; }
    }
  }

  let acc = prefix ? copyFbo(prefix, W, H) : grab(W, H);
  bind(acc);
  if (prefix) { /* The retained prefix includes the unchanged background. */ }
  else if (opt.transparent) clear(0, 0, 0, 0);
  else {
    const fill = PM.normalizeFill(proj.backgroundFill, proj.bg);
    if (fill.type === 'none') clear(0, 0, 0, 0);
    else if (fill.type === 'solid') {
      const bg = PM.hex2rgb(fill.stops[0].color), a = PM.hexAlpha(fill.stops[0].color);
      clear(bg[0] * a, bg[1] * a, bg[2] * a, a);
    }
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

  for (let i = prefix ? prefixStart - 1 : layers.length - 1; i >= 0; i--) {
    if (!prefix && prefixKey && i === prefixStart - 1) {
      const saved = copyFbo(acc, W, H);
      saved.programKeys = [...framePrograms];
      if (!rememberPreview(prefixKey, saved)) free(saved);
    }
    const L = layers[i];
    if (!opt.inputLayer && !opt.mattePass && matteSources.has(L.id)) continue;
    const groupAncestors = PM.groupAncestors?.(L, proj.layers) || [];
    if (!opt.inputLayer && !opt.inputVisibilityRoot && solo && !L.solo && !groupAncestors.some((group: any) => group.solo)
        && !groupContainsSolo(L, orderedLayers, (layer: any) => PM.groupAncestors?.(layer, proj.layers) || [])
        && !opt.mattePass) continue;
    if (L.type !== 'group' && PM.TYPE_META[L.type] && PM.TYPE_META[L.type].visual === false) continue;
    if (!inputLayerActive(L, T, proj, opt)) continue;
    /* Each group owns an offscreen compositing boundary, so opacity is applied
       once at its own level instead of being multiplied into every descendant. */
    const alpha = PM.clamp(PM.ev(L, 'opacity', T) / 100, 0, 1);
    if (alpha <= .001) continue;

    try {
      const hasFx = (!opt.inputLayer || opt.inputStage === 'effects') && hasRenderableEffects(opt.inputPrefix===undefined?L.fx:L.fx.slice(0,opt.inputPrefix), PM, L, T);
      const hasMasks = (!opt.inputLayer || opt.inputStage !== 'source') && (L.masks || []).some((m: any) => evaluatedValue(PM, L, m.on, T, `m.${m.id}.on`) !== false);
      const blend = opt.inputLayer ? 0 : (BLEND_ID as any)[isProperty(L.blend) ? PM.evP(L, L.blend, T, 'l.blend') : L.blend] || 0;
      const mb = (isProperty(L.mblur) ? PM.evP(L, L.mblur, T, 'l.mblur') : L.mblur) && opt.mblur !== false;
      const transition = opt.inputLayer ? null : activeTransition(L, T);

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
        // Cached background pixels must not depend on a moving foreground's
        // opaque coverage on the frame when the prefix happened to be built.
        const cover = prefixKey && i >= prefixStart ? undefined : covers[i];
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
      const sourceOwner=L.effectSpace==='source'&&hasFx&&(!opt.inputLayer||opt.inputStage==='effects');
      if(sourceOwner){
        lf=grab(W,H);bind(lf);clear();
        const times=mb?motionBlurTimes(T,proj.fps,opt):[T];
        for(const at of times){
          const frame=GL.renderProject(proj,at,W,H,{...opt,inputLayer:undefined,inputStage:undefined,rawOwner:L.id,ownerGrow:true,mblur:false,inputContext:layerInputs?.time===at?layerInputs:undefined});
          const e=frame.sourceExtent??{x0:0,y0:0,w:frame.inputResolution[0],h:frame.inputResolution[1]};
          bind(lf);drawContent(L,at,W,H,PM.clamp(PM.ev(L,'opacity',at)/100,0,1)/times.length,undefined,mb,{tex:frame.tex,w:e.w,h:e.h,ax:-e.x0,ay:-e.y0,uv:[0,0,1,1],fromFbo:true,tmp:frame});
        }
      }else if (L.type === 'group') {
        if (mb) {
          lf = grab(W, H); bind(lf); clear();
          const times = motionBlurTimes(T, proj.fps, opt), n = times.length;
          for (const at of times) {
            const sample = GL.renderProject(proj, at, W, H, {
              ...opt, inputLayer: undefined, inputStage: undefined, inputPrefix:undefined, inputContext: layerInputs?.project === proj && layerInputs?.time === at ? layerInputs : undefined, inputVisibilityRoot: opt.inputLayer || opt.inputVisibilityRoot, transparent: true, groupParent: L.id, mblur: false,
            });
            bind(lf); drawFbo(sample, W, H, 1 / n, true); free(sample);
          }
        } else {
          lf = GL.renderProject(proj, T, W, H, { ...opt, inputLayer: undefined, inputStage: undefined, inputPrefix:undefined, inputContext: layerInputs?.project === proj ? layerInputs : undefined, inputVisibilityRoot: opt.inputLayer || opt.inputVisibilityRoot, transparent: true, groupParent: L.id });
        }
      } else {
        lf = grab(W, H);
        bind(lf); clear();
        if (mb) {
          const times = motionBlurTimes(T, proj.fps, opt), n = times.length;
          for (const at of times) {
            drawContent(L, at, W, H, alpha / n, undefined, true);
          }
        } else drawContent(L, T, W, H, alpha);
      }

      let res = lf;
      if (hasMasks&&!sourceOwner) res = applyMasks(L, T, res, W, H);
      if (hasFx&&!sourceOwner) { const prior=res;res = runEffects(L, T, res, W, H, acc, opt.inputPrefix);if(prior!==lf && res!==prior)free(prior); }
      if (L.matteSource && opt.inputPrefix===undefined && (!opt.inputLayer || opt.inputStage === 'effects')) {const prior=res;res=applyTrackMatte(L,T,res,W,H,opt.matteProject||proj,opt);if(prior!==lf && prior!==res)free(prior);}

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
    } catch (error) {
      if (error instanceof MissingCaptureVideoFrame || error instanceof EffectInputError || opt.inputLayer) throw error;
      reportLayerFailure(L, error);
    }
  }
  completed = true;
  return acc;
  }finally{
    if (ownsInputs) {
      for (const frame of ownedInputs?.frames.values() || []) free(frame);
      if (!completed) for (const frame of GL.pool) if (frame.busy && !baseline!.has(frame)) free(frame);
      if (ownedInputs) layerInputRendering = Math.max(0, layerInputRendering - 1);
    }
    layerInputs = previousInputs; inputFrameOptions = previousInputFrameOptions;
    PM._renderComposition3D=previous3DComposition;PM._renderComposition3DPass=previous3DPass;PM._renderCamera3D=previous3DCamera;}
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
    const W = GL.canvas.width, H = GL.canvas.height;
    PM.scope.push(PM.proj); PM.beginEval(time);
    previewViewportActive = !!GL.previewViewport; previewSourceClipping = true; preparingSource = true;
    try {
      if (!PM.active(layer, time)) return;
      // Old zoom/animation variants are disposable; the last visible frame's
      // sources are not. Reclaim only unused variants for speculative uploads.
      const target = MAX_TEXTURE_BYTES - 8 * 1024 * 1024;
      if (textureBytes > target) trimTextures(target, visibleTextures);
      if (textureBytes <= target) contentQuad(layer, time, W, H, undefined, 4 * 1024 * 1024);
    } finally {
      bindTex(0, null);
      previewViewportActive = false; previewSourceClipping = false; preparingSource = false;
      PM.scope.pop(); PM.beginEval(PM.time);
    }
  },
  run => window.requestIdleCallback?.(run),
  () => performance.now(),
  (layer, time) => PM.active(layer, time) && PM.worldOpacity(layer, time) > .001,
);

GL.render = (T: any, opt: any = {}) => {
  visibleTextures.clear();
  framePrograms.clear(); compileSubmitMs = 0;
  const gl = GL.gl; if (!gl) return;
  const W = GL.canvas.width, H = GL.canvas.height;
  let liveInputFrames: Map<string,ImageBitmap> | null = null;
  GL.inputPreviewPresentedTime = undefined;
  const inputPreviewKey = previewStateKey()+JSON.stringify([opt.mblur,opt.mbSamples,opt.shutter]);
  if (inputPreviewJob && (inputPreviewJob.key!==inputPreviewKey || inputPreviewJob.project!==PM.proj || inputPreviewJob.gl!==gl)) cancelInputPreview();
  if (!opt.exporting && !PM.agentFrameCapture && !PM.preparedVideoFrames && hasLayerInputs(PM.proj, (fx:any)=>PM.effectDef?.(fx) ?? PM.Kernel?.effects?.get?.(fx.type))) {
    if (!inputPreviewJob) {
      const requests = GL.frameVideoRequests(PM.proj,T,opt);
      // A streaming frame remains sufficient for single-time inputs. Private
      // immutable frames are needed only when this picture consumes subframes.
      if (requests.length && (requests.some((r:any)=>r.time!==T) || requests.length > videoClipsAt(PM,T).length)) {
        const job = {key:inputPreviewKey,project:PM.proj,gl,time:T,pending:true,frames:null as Map<string,ImageBitmap>|null};
        inputPreviewJob=job;
        void inputPreviewVideos.prepare(T,()=>inputPreviewJob===job&&job.key===previewStateKey()+JSON.stringify([opt.mblur,opt.mbSamples,opt.shutter])&&GL.gl===gl,opt).then(frames=>{
          if(inputPreviewJob!==job){if(frames)inputPreviewVideos.release(frames);return;}
          job.pending=false;job.frames=frames;PM.invalidate?.('render');
        });
      }
    }
    if (inputPreviewJob?.pending) return false;
    if (inputPreviewJob && !inputPreviewJob.frames) {
      if(inputPreviewJob.time!==T){cancelInputPreview();return false;}
      throw new EffectInputError('Exact input preview: '+(inputPreviewVideos.error || 'frames could not be prepared'));
    }
    if(inputPreviewJob?.frames){T=inputPreviewJob.time;liveInputFrames=inputPreviewJob.frames;}
  } else if (inputPreviewJob) cancelInputPreview();
  refreshPreviewState();
  const cacheable = canCachePreview(opt);
  const frameKey = previewFrameKey(T, opt);
  const cached = cacheable ? previewFrames.get(frameKey) : undefined;
  GL.stats.previewHit = !!cached;
  frameFailed = false;
  frameVideoExact = true;
  videoPath = '';
  previewVideoTimes = new Map();
  if (!liveInputFrames && !cached && !opt.exporting && !PM.agentFrameCapture && !PM.preparedVideoFrames) {
    let ready = true;
    for (const clip of videoClipsAt(PM, T)) {
      previewVideoTimes.set(clip.id, clip.time);
      const video = layerVideoElement(PM, clip.asset, clip.id);
      // Register completion invalidation even when we cannot draw yet.
      videoTextureVersion(video);
      if (PM.playing && PM.playbackVideoFrame) {
        if (PM.playbackVideoFrame.time !== T || PM.playbackVideoFrame.frames.get(clip.id)?.video !== video) ready = false;
      } else if (!PM.playing || PM.videoFrameSync) {
        seekPreviewVideo(video, clip.at, .0005);
        if (!previewSeekFrame(video, clip.at)
            && (video.seeking || video.readyState < 2 || Math.abs(video.currentTime - clip.at) > .0005)) ready = false;
      } else if (clip.rate != null && (video.seeking || video.readyState < 2 || !video.videoWidth)) ready = false;
    }
    // Keep the last complete composition until all synchronized clips decode.
    // Mixing old video pixels with new transforms creates visible ghost frames.
    if (!ready) return false;
  }
  gpuTiming?.poll();
  const t0 = window.performance.now();
  GL.stats.draws = 0; GL.stats.passes = 0; GL.stats.viewportVectors = 0;
  viewportPathFrame++;

  PM.beginEval(T);
  PM.scope.push(PM.proj);
  previewViewportActive = !!GL.previewViewport;
  // Full sources remain available as a reference for pixel/performance checks.
  useVideoPreviews = !opt.exporting;
  previewSourceClipping = opt.sourceClipping !== false && !opt.exporting;
  let acc;
  const priorIdleFrames=idleVideoFrames;
  if(liveInputFrames){idleVideoFrames=liveInputFrames;exactInputPreviewRendering=true;}
  const priorParallel = allowParallelCompile; allowParallelCompile = !opt.exporting;
  rememberingPreview = cacheable && (PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024) >= colorBufferBytes(W, H, workingColorDepth());
  try {
    acc = cached || GL.renderProject(PM.proj, T, W, H, {...opt,editorViewport:!opt.exporting&&!PM.agentFrameCapture});
  } finally {
    allowParallelCompile = priorParallel;
    if(liveInputFrames){idleVideoFrames=priorIdleFrames;exactInputPreviewRendering=false;inputPreviewVideos.release(liveInputFrames);inputPreviewJob=null;}
    rememberingPreview = false;
    PM.scope.pop();
    previewViewportActive = false;
    previewSourceClipping = false;
  }

  GL.inputPreviewPresentedTime = liveInputFrames ? T : undefined;
  /* present */
  bind(null);
  gl.clearColor(0, 0, 0, 1); gl.clear(gl.COLOR_BUFFER_BIT);
  const p = program('present', PM.FRAG_COPY);
  const g = use(p);
  bindTex(0, acc.tex); setI(p, 'u_tex', 0);
  g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
  gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
  if (presentedFrame && presentedFrame !== acc && !presentedFrame.previewCached) free(presentedFrame);
  presentedFrame = acc;
  GL.pool.forEach((f: any) => f.busy = f === presentedFrame);
  trimPool();
  trimTextures();
  trimViewportPathRasters();
  // Keep every program used by this frame: an active scene larger than the
  // cache target must not recompile its shaders on every scrub.
  for (const key of GL.progs.keys()) {
    if (cached) break;
    if (GL.progs.size <= 256) break;
    if (!framePrograms.has(key)) GL.dropProgram(key);
  }
  GL.stats.progs = GL.progs.size;
  if (!cached && cacheable && frameVideoExact && !frameFailed && !pendingPrograms.size && !GL.errors.size
      && typeof document !== 'undefined' && document.fonts?.status !== 'loading') {
    rememberPreview(frameKey, acc);
    trimPool(PM.Memory?.budget?.('framebuffers') ?? MAX_FBO_BYTES);
  }
  GL.stats.ms = window.performance.now() - t0;
  if (!opt.exporting && typeof window.requestIdleCallback === 'function') requestSourceWarmup();
  if (!opt.exporting) PM.bus?.emit?.('preview:presented', { viewport: GL.previewViewport, time: T, version: PM.animVersion?.(), project: PM.proj, quality: PM.quality });
};

/** Prepare a finished display image without drawing to the visible canvas.
 * Working passes retain the project's color precision. Only the final picture
 * uses RGBA8, exactly like the viewer, so 32-bit artwork does not require a
 * 16-byte-per-pixel preview cache. No pixels are read back or written to disk. */
GL.preparePreviewFrame = async (T: number, opt: any, frames: Map<string, ImageBitmap>, valid: () => boolean): Promise<boolean> => {
  const gl = GL.gl, W = GL.canvas?.width, H = GL.canvas?.height;
  if (!gl || !W || !H || cancelPreviewPreparation || !valid() || !canCachePreview(opt)) return false;
  refreshPreviewState();
  const key = previewFrameKey(T, opt), signature = previewStateKey();
  if (previewFrames.has(key)) return true;
  const prior = {
    stats: { ...GL.stats }, visible: [...visibleTextures], programs: [...framePrograms],
    previews: useVideoPreviews, clipping: previewSourceClipping, viewport: previewViewportActive,
    videoFrames: idleVideoFrames, videoTimes: previewVideoTimes, path: videoPath,
    prefix: isolatedVideoTexturePrefix, noBlur: isolatedCaptureNoMotionBlur,
    failed: frameFailed, exact: frameVideoExact, parallel: allowParallelCompile, fbo: boundFbo,
    compileMs: compileSubmitMs, busy: new Set(GL.pool.filter((f: any) => f.busy)),
  };
  const prefix = 'idle-video:' + ++isolatedVideoTextureGeneration + ':';
  let acc: any, image: any, retained = false;
  try {
    visibleTextures.clear(); framePrograms.clear();
    frameFailed = false; frameVideoExact = true; compileSubmitMs = 0;
    idleVideoFrames = frames; isolatedVideoTexturePrefix = prefix;
    isolatedCaptureNoMotionBlur = opt.mblur === false;
    videoPath = ''; previewVideoTimes = new Map();
    useVideoPreviews = true; previewSourceClipping = opt.sourceClipping !== false;
    previewViewportActive = !!GL.previewViewport; allowParallelCompile = true;
    PM.beginEval(T); PM.scope.push(PM.proj);
    try { acc = GL.renderProject(PM.proj, T, W, H, { ...opt, editorViewport: true }); }
    finally { PM.scope.pop(); }
    if (!valid() || frameFailed || !frameVideoExact || pendingPrograms.size || GL.errors.size
        || document.fonts?.status === 'loading') return false;
    image = grab(W, H, false, 8);
    bind(image);
    const p = program('present', PM.FRAG_COPY), g = use(p);
    bindTex(0, acc.tex); setI(p, 'u_tex', 0);
    g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
    gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
    image.programKeys = [...framePrograms];
    // Detach while the GPU finishes: normal playback must never recycle this
    // still-pending image as one of its working render targets.
    const index = GL.pool.indexOf(image);
    if (index >= 0) { GL.pool.splice(index, 1); poolBytes -= image.bytes; }
    GL.previewPreparationBytes = image.bytes;
    retained = true;
  } catch {
    return false; // Normal foreground rendering remains responsible for errors.
  } finally {
    if (acc) free(acc);
    if (image && !retained) free(image);
    useVideoPreviews = prior.previews; previewSourceClipping = prior.clipping;
    previewViewportActive = prior.viewport; idleVideoFrames = prior.videoFrames;
    previewVideoTimes = prior.videoTimes; videoPath = prior.path;
    isolatedVideoTexturePrefix = prior.prefix; isolatedCaptureNoMotionBlur = prior.noBlur;
    frameFailed = prior.failed; frameVideoExact = prior.exact; allowParallelCompile = prior.parallel;
    compileSubmitMs = prior.compileMs;
    // Capture uploads are private and must not evict or replace live decoders.
    dropTextures(prefix);
    visibleTextures.clear(); for (const key of prior.visible) visibleTextures.add(key);
    framePrograms.clear(); for (const key of prior.programs) framePrograms.add(key);
    Object.assign(GL.stats, prior.stats);
    PM.beginEval(PM.time);
    bind(prior.fbo);
    for (const frame of GL.pool) frame.busy = prior.busy.has(frame) || frame === presentedFrame;
    trimPool(); trimTextures(MAX_TEXTURE_BYTES, visibleTextures);
  }
  if (!image || !retained) return false;
  // A zero-timeout fence poll avoids synchronous GPU waits and caps the queue
  // at one prepared frame. Input can cancel the poll and discard the result.
  return new Promise(resolve => {
    const sync = gl.fenceSync(gl.SYNC_GPU_COMMANDS_COMPLETE, 0);
    let timer: ReturnType<typeof setTimeout> | undefined, done = false;
    const started = performance.now();
    const finish = (keep: boolean) => {
      if (done) return; done = true;
      clearTimeout(timer);
      if (sync) gl.deleteSync(sync);
      cancelPreviewPreparation = undefined; GL.previewPreparationBytes = 0;
      const budget = Math.max(0, (PM.Memory?.budget?.('preview') ?? 256 * 1024 * 1024) - (PM.Preview?.bytes || 0));
      if (keep && previewFrames.bytes + image.bytes <= budget && previewFrames.put(key, image, budget)) {
        image.previewCached = true; resolve(true);
      } else {
        for (let unit = 0; unit < boundTex.length; unit++) if (boundTex[unit] === image.tex) bindTex(unit, null);
        gl.deleteFramebuffer(image.fb); gl.deleteTexture(image.tex); resolve(false);
      }
    };
    cancelPreviewPreparation = () => finish(false);
    const poll = () => {
      if (done) return;
      if (GL.gl !== gl || gl.isContextLost() || !valid() || previewStateKey() !== signature
          || performance.now() - started > 5000) { finish(false); return; }
      const status = sync ? gl.clientWaitSync(sync, 0, 0) : gl.WAIT_FAILED;
      if (status === gl.ALREADY_SIGNALED || status === gl.CONDITION_SATISFIED) finish(true);
      else if (status === gl.WAIT_FAILED) finish(false);
      else timer = setTimeout(poll, 16);
    };
    gl.flush(); poll();
  });
};

/** Render one frame and read back raw RGBA pixels (bottom-up, premultiplied).
    Opaque callers can request topDownOpaque for directly consumable image data.
    `opt.comp` renders another composition record instead of the open one. */
function renderToPixels(T: any, W: any, H: any, opt: any = {}, readOpaque?: () => void,
  readHalf?: ((texture: WebGLTexture) => Uint8Array<ArrayBuffer> | null) | DeferredAlphaReader,
  readFloat?: ((texture: WebGLTexture) => Uint8Array<ArrayBuffer> | null) | DeferredAlphaReader) {
  const gl = GL.gl; if (!gl) return null;
  const previousPreviews = useVideoPreviews; useVideoPreviews = false;
  const previousTexturePrefix = isolatedVideoTexturePrefix, previousNoMotionBlur = isolatedCaptureNoMotionBlur;
  const previousVideoPath = videoPath;
  const ownedTexturePrefix = opt.isolatedVideoFrames ? 'capture-video:' + ++isolatedVideoTextureGeneration + ':' : null;
  isolatedVideoTexturePrefix = ownedTexturePrefix;
  isolatedCaptureNoMotionBlur = !!ownedTexturePrefix && opt.mblur === false;
  if (ownedTexturePrefix) videoPath = '';
  const comp = opt.comp || PM.proj;
  PM.beginEval(T);
  let acc;
  try {
    PM.scope.push(comp);
    try { acc = GL.renderProject(comp, T, W, H, opt); }
    finally { PM.scope.pop(); }
    bind(acc);
    const reader = !opt.opaque && (acc.colorDepth === 16 ? readHalf : acc.colorDepth === 32 ? readFloat : undefined);
    if (reader) {
      if (typeof reader !== 'function') { reader.queue(acc.tex); return null; }
      const pixels = reader(acc.tex);
      if (pixels) return pixels;
    }
    // A deferred capture can encounter a hardware depth fallback. Leave its
    // serial recapture to the caller, without allocating speculative CPU pixels.
    if (readHalf && typeof readHalf === 'object' || readFloat && typeof readFloat === 'object') return null;
    const px = readOpaque ? null : new Uint8Array(W * H * 4);
    if (opt.opaque) {
      // Match the visible RGBA8 presentation's quantization without touching its
      // canvas, viewport metadata, or retained frame. Read just the capture size.
      const target = gl.createFramebuffer(), color = gl.createRenderbuffer();
      try {
        if (readOpaque && (!target || !color)) throw new Error('The GPU could not capture the export frame');
        gl.bindRenderbuffer(gl.RENDERBUFFER, color);
        gl.renderbufferStorage(gl.RENDERBUFFER, gl.RGBA8, W, H);
        gl.bindFramebuffer(gl.FRAMEBUFFER, target);
        gl.framebufferRenderbuffer(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.RENDERBUFFER, color);
        if (readOpaque && gl.checkFramebufferStatus(gl.FRAMEBUFFER) !== gl.FRAMEBUFFER_COMPLETE) {
          throw new Error('The GPU could not capture the export frame');
        }
        gl.viewport(0, 0, W, H);
        const p = opt.topDownOpaque ? program('capture-opaque', PM.FRAG_CAPTURE_OPAQUE) : program('present', PM.FRAG_COPY), g = use(p);
        bindTex(0, acc.tex); setI(p, 'u_tex', 0);
        g.u('u_m', fullQuad(W, H)); g.u('u_res', W, H); g.u('u_uv', 0, 0, 1, 1);
        gl.disable(gl.BLEND); draw(); gl.enable(gl.BLEND);
        if (readOpaque) readOpaque();
        else gl.readPixels(0, 0, W, H, gl.RGBA, gl.UNSIGNED_BYTE, px);
      } finally {
        gl.bindRenderbuffer(gl.RENDERBUFFER, null);
        gl.deleteFramebuffer(target); gl.deleteRenderbuffer(color);
        bind(null);
      }
    } else if (acc.colorDepth === 8) {
      gl.readPixels(0, 0, W, H, gl.RGBA, gl.UNSIGNED_BYTE, px);
    } else {
      const floats=new Float32Array(W*H*4);
      gl.readPixels(0,0,W,H,gl.RGBA,gl.FLOAT,floats);
      for(let i=0;i<px!.length;i++)px![i]=Math.round(Math.max(0,Math.min(1,floats[i]!))*255);
    }
    return px;
  } finally {
    useVideoPreviews = previousPreviews;
    isolatedVideoTexturePrefix = previousTexturePrefix;
    isolatedCaptureNoMotionBlur = previousNoMotionBlur;
    if (ownedTexturePrefix) videoPath = previousVideoPath;
    // Private prepared frames and their uploads belong only to this capture.
    // Drop them before budget trimming so they cannot evict viewer textures.
    if (ownedTexturePrefix) {
      for (const [key, entry] of GL.texes) if (key.startsWith(ownedTexturePrefix)) {
        for (let unit = 0; unit < boundTex.length; unit++) if (boundTex[unit] === entry.tex) bindTex(unit, null);
      }
      dropTextures(ownedTexturePrefix);
    }
    if (acc) free(acc);
    GL.pool.forEach((f: any) => f.busy = f === presentedFrame);
    trimPool();
    trimTextures();
    viewportPathFrame++;
    trimViewportPathRasters();
    bind(null);
  }
}
GL.renderToPixels = renderToPixels;
GL.createOpaqueReadback = (W: number, H: number) => {
  const gl = GL.gl;
  return createOpaqueReadback(gl, W, H, (time, options, read) => {
    renderToPixels(time, W, H, { ...options, opaque: true, topDownOpaque: true }, read);
  }, () => GL.gl === gl && !GL.contextLost);
};
GL.createHalfReadback = (W: number, H: number) => {
  const gl = GL.gl, generation = readbackGeneration;
  return createHalfReadback(gl, W, H, generation, (time, options, readHalf) => {
    return renderToPixels(time, W, H, options, undefined, readHalf);
  }, () => GL.gl === gl && !GL.contextLost && generation === readbackGeneration);
};
GL.createFloatReadback = (W: number, H: number) => {
  const gl = GL.gl, generation = readbackGeneration;
  return createFloatReadback(gl, W, H, generation, (time, options, readFloat) => {
    return renderToPixels(time, W, H, options, undefined, undefined, readFloat);
  }, () => GL.gl === gl && !GL.contextLost && generation === readbackGeneration);
};
GL.createAlphaReadback = (W: number, H: number) => {
  const gl = GL.gl, generation = readbackGeneration, depth = PM.proj.colorDepth === 32 ? 32 : 16;
  const matchesDepth = () => PM.proj.colorDepth !== 8 && (PM.proj.colorDepth === 32 ? 32 : 16) === depth
    && availableColorDepth(normalizeColorDepth(PM.proj.colorDepth), GL.supportedColorDepths || [8, 16]) === depth;
  if (!matchesDepth()) return null;
  return createAlphaReadback(gl, W, H, generation, depth, (time, options, reader) => {
    renderToPixels(time, W, H, options, undefined, depth === 16 ? reader : undefined, depth === 32 ? reader : undefined);
  }, () => GL.gl === gl && !GL.contextLost && generation === readbackGeneration, matchesDepth);
};

/* Hit test: which layer is under a comp-space point (top-most first). */
GL.pick = (x: any, y: any, T: any, options: { includeLocked?: boolean } = {}) => {
  pruneSceneRuntimes(PM);
  const pickCamera=PM.proj.layers.some(layer3DRole)?viewportCamera(PM,compositionRuntime(PM,T).camera):undefined;
  const layers = compositionOrderedLayers(PM, PM.proj.layers, T,pickCamera);
  for (const L of layers) {
    if (!PM.active(L, T) || !options.includeLocked && (L.lock || (PM.groupAncestors?.(L) || []).some((group: any) => group.lock)) || (PM.TYPE_META[L.type] && PM.TYPE_META[L.type].pickable === false)) continue;
    if(layer3DRole(L)){
      if(layer3DRole(L)!=='object')continue;
      const runtime=compositionRuntime(PM,T),depthIds=compositionDepthIds(PM,L,T);
      const picked=runtime.pick(x/PM.proj.w*2-1,1-y/PM.proj.h*2,depthIds,pickCamera);
      if(picked===L.id)return L;
      continue;
    }
    const b = GL.bounds(L, T);
    if (!b) continue;
    if (planeContains(PM, L, T, x, y, b)) {
      const runtime=L.type==='extension'?sceneRuntime(PM,L,T):null;
      if(runtime && !runtime.scene.background) {
        const inverse=inversePlane(planeMatrix(PM,L,T));
        if(!inverse)continue;
        const {x:px,y:py}=projectPoint(inverse,{x,y});
        const w=L.d.w||PM.proj.w,h=L.d.h||PM.proj.h;
        runtime.resizeCamera(w,h);
        if(!runtime.pick(px/w*2-1,1-py/h*2))continue;
      }
      return L;
    }
  }
  return null;
};
/** Layer-space bounds (before transform), relative to the layer anchor origin. */
GL.bounds = (L: any, T: any) => {
  const bounds = nativeBounds(L, T);
  if(layer3DRole(L))return bounds;
  if (!bounds) return bounds;
  const offset = sizeAnchorOffset(PM, L, T);
  if (!offset.x && !offset.y) return bounds;
  return { ...bounds, x0: bounds.x0 + offset.x, x1: bounds.x1 + offset.x,
    y0: bounds.y0 + offset.y, y1: bounds.y1 + offset.y,
    ax: bounds.ax - offset.x, ay: bounds.ay - offset.y };
};
function nativeBounds(L: any, T: any) {
  if(layer3DRole(L)){
    if(layer3DRole(L)!=='object')return null;
    const runtime=compositionRuntime(PM,T);
    return runtime.bounds(PM.proj.w,PM.proj.h,[L.id],viewportCamera(PM,runtime.camera));
  }
  if (L.type === 'group') return PM.groupBounds(L, T);
  if(L.type==='extension') {
    const runtime=sceneRuntime(PM,L,T);
    if(runtime && !runtime.scene.background) return runtime.bounds(L.d.w||PM.proj.w,L.d.h||PM.proj.h);
  }
  const d = resolveContent(PM, L, T);
  let w, h, ax, ay;
  if (L.type === 'solid' || L.type === 'shader' || L.type === 'extension') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'null') { w = d.w || 100; h = d.h || 100; ax = 0; ay = 0; }
  else if (L.type === 'precomp') { w = d.w || PM.proj.w; h = d.h || PM.proj.h; ax = 0; ay = 0; }
  else if (L.type === 'captions') return PM.captionBounds(L, T);
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
