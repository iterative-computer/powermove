/*
 * The `services` `raster` override with "Use New Engine" on (plan section
 * 20; owner amendments 23 and 24; docs/engine/extensions-compat.md).
 *
 * Today an extension at apiVersion 1 to 3 can wrap the `raster` service:
 * the compositor calls `PM.raster(L, scale, T, uploaded, crop)` for every
 * text, captions, and shape layer it draws, the wrapper returns its own
 * surface for the layers it changes and the captured service's result for
 * the rest, and the compositor draws whatever comes back on the quad it
 * describes (`w`, `h`, `anchorX`, `anchorY`, `uv`).
 *
 * With the engine, while a wrapper is registered the window is the
 * session's `rasterOverride` producer: the engine asks for every such layer
 * at the frame's time and raster density (`pm_render::compat`), and this
 * module runs the wrapper chain exactly as the compositor called it, over a
 * base that is the engine's raster of the layer:
 *
 * - The base surface is lazy: nothing is drawn until the wrapper reads a
 *   field. A wrapper that returns it untouched (the documented pass-through)
 *   costs no round trip, and the layer is answered `native`: the engine
 *   draws it itself, pixel for pixel what it draws without a wrapper.
 * - Any other result is the layer's raster: its canvas is read back exactly
 *   as the compositor uploads a canvas (a WebGL texture with premultiplied
 *   alpha), and posted with today's quad (shapes without paths are anchored
 *   at their center, as `nativeContentQuad` does).
 * - A result whose cache `key` is the one already posted for that layer,
 *   time and density is answered `reuse`: no pixels are sent again.
 *
 * Several wrappers stack LIFO as today (the services registry); the engine
 * sees only the chain's result. Registering or disposing a wrapper, and a
 * font catalogue change, drop every earlier answer (the producer list is
 * sent again), so the next frame asks afresh.
 */
import type { CompatNeed } from '../../../shared/engine-protocol';
import type { CompatAnswerOptions, CompatAnswerResult, EngineRenderTransport } from '../engine-connection/render-transport';
import { compatKey } from '../../../shared/protocol-strings';
import type { Disposable } from './api';

type LegacyPM = Record<string, any>;
type Raster = (layer: any, scale?: number, time?: number, uploaded?: any, crop?: any) => any;

/** Protocol ticks per second (pm_protocol TICKS_PER_SECOND). */
export const TICKS_PER_SECOND = 705_600_000;

const LAZY = Symbol('powermove.lazyRaster');

/** Today's raster cache entry bound (legacy/gl/raster.ts `MAX`). */
const MAX_BASES = 4096;

type LazyMark = {
  layer: unknown; scale: number; time: number; crop: unknown;
  /** The wrapper wrote a field, or read the canvas (it may have drawn on it). */
  touched: boolean;
  /** The engine raster the fields came from, once read. */
  loaded: () => any;
};

export interface RasterCompatOptions {
  /** Whether `fn` is the base raster (the kernel's own `raster` registration). */
  isBase: (fn: unknown) => boolean;
  /** What the chain resolves to now (`kernel.services.get('raster')`). */
  chain: () => Raster | null;
  /** `fn` runs when the chain changes (`kernel.services.onChange`). */
  onChainChange: (fn: () => void) => Disposable;
  /** The engine's raster of a layer object (today's surface shape, with
   *  today's crop window applied: `sourceWindow` for text, `uv` for shapes), or null. */
  engineRaster: (layer: unknown, scale: number, time: number, crop?: RasterWindowLike) => any;
  /** Prepare the same base surface without blocking the renderer's event loop. */
  engineRasterAsync?: (layer: unknown, scale: number, time: number, crop?: RasterWindowLike) => Promise<any>;
  /** Native-evaluated content identity for a base whose pixels are time independent. */
  rasterKey?: (layer: any, scale: number, time: number, crop?: RasterWindowLike) => string | null;
  /** Reads a surface's pixels as the compositor uploads them. */
  readPixels?: (source: CanvasImageSource & { width: number; height: number }) => { width: number; height: number; format: 'rgba8Premultiplied' | 'rgba8Straight'; data: Uint8Array } | null;
  log?: (message: string) => void;
}

/** Today's crop window (`legacy/gl/shape-raster-window.ts` `RasterWindow`). */
export type RasterWindowLike = { x: number; y: number; width: number; height: number };

export interface RasterCompatHandle {
  /** True while the chain runs for the engine: the base returns lazy engine rasters. */
  readonly producing: boolean;
  /** Whether a wrapper is installed (the chain is not the base). */
  wrapped(): boolean;
  /** The base raster while producing. */
  base(args: Parameters<Raster>): any;
  /** Answers the transport's needs while a wrapper is registered. */
  attach(transport: EngineRenderTransport): () => void;
  /** Drops every answer the engine holds (a font change, an extension reload). */
  reset(): void;
  /** Answers one need (tests, the transport); `fresh`: never `reuse` (an export frame). */
  answer(need: CompatNeed, revision: number, options?: CompatAnswerOptions): CompatAnswerResult;
  /** Optional interactive-playback preparation before a synchronous answer. */
  prepare(need: CompatNeed, revision: number): Promise<void>;
  /** Whether `answer` can run now without asking the engine for base pixels
   *  on this thread: the chain passed this layer through the last time it
   *  ran (it read no base), or the base it reads is already here. */
  ready(need: CompatNeed, revision: number): boolean;
  /** Whether the need's layer is in its composition's frame at its time
   *  (on, and within its in and out points, a frame either side). */
  active(need: CompatNeed): boolean;
  dispose(): void;
}

/** Finds a layer object by id in the open document (the root or a nested composition). */
export function findLayer(PM: LegacyPM, composition: string, layer: string): any {
  const proj = PM?.proj;
  if (!proj) return null;
  const rootId = String(proj.compId ?? proj.id ?? '');
  const lists: unknown[] = [];
  if (composition === rootId) lists.push(proj.layers);
  const nested = proj.comps?.[composition];
  if (nested) lists.push(nested.layers);
  if (!lists.length) lists.push(proj.layers);
  for (const list of lists) {
    if (!Array.isArray(list)) continue;
    const found = list.find((l: any) => l?.id === layer);
    if (found) return found;
  }
  return null;
}

/**
 * The layer an export's snapshot holds (`json`): the window's own layer
 * object when it is the same (wrappers keep their per-layer caches), else a
 * copy of the snapshot's, so an export draws the project as it was
 * exported (a render queue job, or an export of an edited project).
 */
export function snapshotLayer(PM: LegacyPM, composition: string, layer: string, json: string): any {
  const live = findLayer(PM, composition, layer);
  try {
    if (live && JSON.stringify(live) === json) return live;
  } catch {
    // A live layer that does not serialize: use the snapshot's.
  }
  try {
    return JSON.parse(json);
  } catch {
    return live;
  }
}

/** The canvas fields of a surface: reading one hands out the pixels. */
const CANVAS_KEYS = new Set<PropertyKey>(['cv', 'canvas', 'bitmap']);

/** A surface whose fields come from `load` on first read. */
function lazySurface(mark: Omit<LazyMark, 'touched' | 'loaded'>, load: () => any): any {
  let done = false;
  let value: any = null;
  const get = () => {
    if (!done) {
      done = true;
      value = load();
    }
    return value;
  };
  const full: LazyMark = { ...mark, touched: false, loaded: () => (done ? value : null) };
  return new Proxy(Object.create(null), {
    get(_target, key) {
      if (key === LAZY) return full;
      const surface = get();
      if (CANVAS_KEYS.has(key)) full.touched = true;
      return surface == null ? undefined : surface[key];
    },
    has(_target, key) {
      if (key === LAZY) return true;
      const surface = get();
      return surface != null && key in surface;
    },
    ownKeys() {
      const surface = get();
      return surface == null ? [] : Reflect.ownKeys(surface);
    },
    getOwnPropertyDescriptor(_target, key) {
      const surface = get();
      if (surface == null) return undefined;
      const descriptor = Object.getOwnPropertyDescriptor(surface, key);
      if (descriptor) descriptor.configurable = true;
      return descriptor;
    },
    set(_target, key, v) {
      const surface = get();
      full.touched = true;
      if (surface != null) surface[key] = v;
      return true;
    },
    defineProperty(_target, key, descriptor) {
      const surface = get();
      full.touched = true;
      if (surface != null) Object.defineProperty(surface, key, descriptor);
      return true;
    },
    deleteProperty(_target, key) {
      const surface = get();
      full.touched = true;
      if (surface != null) delete surface[key];
      return true;
    }
  });
}

/** Today's quad sampling of a result (`contentQuad`): `uv` through the
 *  vertex shader, then `sourceWindow` in the bitmap's pixels (the
 *  draw-window program) — one `[ox, oy, sx, sy]` over the uploaded raster. */
export function resultUv(result: any, width: number, height: number): [number, number, number, number] | undefined {
  const finite = (v: unknown, n: number) => Array.isArray(v) && v.length === n && v.every((x: unknown) => typeof x === 'number' && Number.isFinite(x));
  const uv: [number, number, number, number] = finite(result?.uv, 4) ? result.uv : [0, 0, 1, 1];
  if (!finite(result?.sourceWindow, 4) || !(width > 0 && height > 0)) return finite(result?.uv, 4) ? uv : undefined;
  const [x, y, w, h] = result.sourceWindow as [number, number, number, number];
  return [(uv[0] * w - x) / width, (uv[1] * h - y) / height, (uv[2] * w) / width, (uv[3] * h) / height];
}

let gl: WebGL2RenderingContext | null | undefined;

/**
 * A canvas's pixels as the compositor's texture holds them: uploaded to a
 * WebGL2 texture with `UNPACK_PREMULTIPLY_ALPHA_WEBGL` and read back, rows
 * top-down. Without WebGL2, the 2D context's straight-alpha pixels.
 */
export function compositorPixels(source: CanvasImageSource & { width: number; height: number }): { width: number; height: number; format: 'rgba8Premultiplied' | 'rgba8Straight'; data: Uint8Array } | null {
  const width = Math.round(source.width), height = Math.round(source.height);
  if (!(width >= 1 && height >= 1)) return null;
  if (gl === undefined) {
    try {
      gl = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(1, 1).getContext('webgl2', { premultipliedAlpha: true, antialias: false }) : null;
    } catch {
      gl = null;
    }
  }
  if (gl && !gl.isContextLost()) {
    const texture = gl.createTexture();
    const framebuffer = gl.createFramebuffer();
    try {
      gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_PREMULTIPLY_ALPHA_WEBGL, true);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, false);
      gl.pixelStorei(gl.UNPACK_COLORSPACE_CONVERSION_WEBGL, gl.NONE);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, source as TexImageSource);
      gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
      gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, texture, 0);
      if (gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE) {
        const data = new Uint8Array(width * height * 4);
        gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, data);
        return { width, height, format: 'rgba8Premultiplied', data };
      }
    } catch {
      // Fall through to the 2D context.
    } finally {
      gl.bindFramebuffer(gl.FRAMEBUFFER, null);
      gl.deleteFramebuffer(framebuffer);
      gl.deleteTexture(texture);
    }
  }
  const canvas = typeof OffscreenCanvas !== 'undefined' ? new OffscreenCanvas(width, height) : Object.assign(document.createElement('canvas'), { width, height });
  const context = (canvas as HTMLCanvasElement).getContext('2d') as CanvasRenderingContext2D | null;
  if (!context) return null;
  context.drawImage(source, 0, 0);
  const pixels = context.getImageData(0, 0, width, height).data;
  return { width, height, format: 'rgba8Straight', data: new Uint8Array(pixels.buffer, pixels.byteOffset, pixels.byteLength) };
}

/** The canvas of a surface (`cv`, `canvas`, or `bitmap`). */
function canvasOf(surface: any): (CanvasImageSource & { width: number; height: number }) | null {
  const source = surface?.cv ?? surface?.canvas ?? surface?.bitmap;
  return source && Number(source.width) > 0 && Number(source.height) > 0 ? source : null;
}

/** Today's `uploaded(key)` lookup, answering a miss. */
const NOT_UPLOADED = (_key: string): undefined => undefined;

const TRANSPARENT: CompatAnswerResult = { kind: 'raster', key: 'powermove:empty', width: 1, height: 1, format: 'rgba8Premultiplied', data: new Uint8Array(4), anchor: [0, 0], size: [1, 1] };

export function createRasterCompat(PM: LegacyPM, options: RasterCompatOptions): RasterCompatHandle {
  let producing = 0;
  let transport: EngineRenderTransport | null = null;
  let unanswer: (() => void) | null = null;
  /** The key last posted per layer, time and density (for `reuse`). */
  const posted = new Map<string, string>();
  /** The quad each key was posted with per layer and density: the same key
   *  on the same quad at another time is a `reuse`. */
  const postedQuads = new Map<string, string>();
  const ready = new Map<string, { layer: unknown; surface: any }>();
  const preparing = new Map<string, { layer: unknown; promise: Promise<void> }>();
  const bases = new Map<string, { layer: unknown; surface: any; bytes: number }>();
  let baseBytes = 0;
  /** Layers (composition, layer) whose chain read the base the last time it
   *  answered them: only those wait for prepared pixels while playing. The
   *  rest (the chain passed them through) answer at once. */
  const readsBase = new Set<string>();
  /** Base surfaces loaded so far (a load during an answer marks its layer). */
  let loads = 0;
  let pending = 0, generation = 0, readyRevision: number | null = null;
  let active: { layer: unknown; key: string; scale: number; time: number; crop: string } | null = null;
  let serial = 0;
  const log = options.log ?? ((message: string) => console.warn(`[raster-compat] ${message}`));
  const readPixels = options.readPixels ?? compositorPixels;

  const cropKey = (crop: RasterWindowLike | undefined) => crop ? `${crop.x},${crop.y},${crop.width},${crop.height}` : '';
  const sampleKey = (need: CompatNeed, revision: number) => `${need.composition}\u0000${need.layer}\u0000${need.time}\u0000${need.scale}\u0000${cropKey(need.crop ?? undefined)}\u0000${revision}`;
  const sampleRevision = (revision: number) => {
    if (readyRevision === revision) return;
    readyRevision = revision;
    ready.clear(); bases.clear(); baseBytes = 0;
    generation++;
  };
  const clearReady = () => { ready.clear(); preparing.clear(); bases.clear(); baseBytes = 0; readyRevision = null; generation++; };
  // Today's raster cache bounds (legacy/gl/raster.ts: `MAX` entries and
  // the `raster` memory budget, 192 MiB by default).
  const maxBaseBytes = Number(PM?.Memory?.budget?.('raster')) || 192 * 1024 * 1024;
  const rememberBase = (key: string | null, layer: unknown, surface: any) => {
    if (!key || !surface) return;
    const bytes = Number(surface.cv?.width) * Number(surface.cv?.height) * 4;
    if (!(bytes > 0 && bytes <= maxBaseBytes)) return;
    const old = bases.get(key);
    if (old) baseBytes -= old.bytes;
    bases.delete(key);
    bases.set(key, { layer, surface, bytes }); baseBytes += bytes;
    while (bases.size > MAX_BASES || baseBytes > maxBaseBytes) {
      const first = bases.keys().next().value!;
      baseBytes -= bases.get(first)!.bytes; bases.delete(first);
    }
  };

  const wrapped = (): boolean => {
    const chain = options.chain();
    return typeof chain === 'function' && !options.isBase(chain);
  };

  const handle: RasterCompatHandle = {
    get producing() { return producing > 0; },

    wrapped,

    base([layer, scale = 1, time, , crop]: Parameters<Raster>): any {
      const s = Number(scale) || 1;
      const t = Number(time ?? PM?.time ?? 0);
      const prepared = active && active.layer === layer ? ready.get(active.key) : undefined;
      const matches = prepared?.layer === layer && active?.scale === s && active.time === t && active.crop === cropKey(crop ?? undefined);
      const load = () => {
        loads++;
        if (matches) return prepared!.surface;
        // Today's raster cache: the same content at the same density and
        // window is the same surface, in any frame (the key is the evaluated
        // content, so a layer copy a wrapper draws hits it as well).
        const key = options.rasterKey?.(layer, s, t, crop ?? undefined) ?? null;
        const cached = key ? bases.get(key) : undefined;
        if (cached) {
          bases.delete(key!); bases.set(key!, cached);
          return cached.surface;
        }
        const surface = options.engineRaster(layer, s, t, crop ?? undefined);
        // Today's surfaces carry their cache key (wrappers key their own
        // results by it); the same content gets the same key.
        if (surface && typeof surface === 'object' && surface.key === undefined) surface.key = key ? `powermove-raster:${key}` : `powermove-raster:#${++serial}`;
        rememberBase(key, layer, surface);
        return surface;
      };
      // A captions layer between cues is null today: decide it now.
      if (layer?.type === 'captions') return load();
      return lazySurface({ layer, scale: s, time: t, crop }, load);
    },

    answer(need: CompatNeed, revision: number, answerOptions?: CompatAnswerOptions): CompatAnswerResult {
      sampleRevision(revision);
      const layer = answerOptions?.layer !== undefined
        ? snapshotLayer(PM, String(need.composition), String(need.layer), answerOptions.layer)
        : findLayer(PM, String(need.composition), String(need.layer));
      // A layer the window no longer has: the engine draws its own.
      if (!layer) return { kind: 'native' };
      const chain = options.chain();
      if (typeof chain !== 'function' || options.isBase(chain)) return { kind: 'native' };
      if (layer.type === 'captions' && PM?.captionsHidden) return { kind: 'native' };
      const time = Number(need.time) / TICKS_PER_SECOND;
      const scale = Number(need.scale);
      // Today's preview crop window, as the engine planned it for a viewer
      // frame (pm_render::compat_crop); exports and snapshots have none.
      const crop: RasterWindowLike | undefined = need.crop ? { x: need.crop.x, y: need.crop.y, width: need.crop.width, height: need.crop.height } : undefined;
      const loadsBefore = loads;
      try {
        return answerWith(need, revision, answerOptions, layer, chain, time, scale, crop);
      } finally {
        rememberReads(need, loads > loadsBefore);
      }
    },

    ready(need: CompatNeed, revision: number): boolean {
      if (!readsBase.has(readTag(need)) || !wrapped()) return true;
      if (readyRevision === revision && ready.has(sampleKey(need, revision))) return true;
      const layer = findLayer(PM, String(need.composition), String(need.layer));
      if (!layer) return true;
      const baseKey = options.rasterKey?.(layer, Number(need.scale), Number(need.time) / TICKS_PER_SECOND, need.crop ?? undefined) ?? null;
      return readyRevision === revision && !!baseKey && bases.has(baseKey);
    },

    async prepare(need: CompatNeed, revision: number): Promise<void> {
      return prepareBase(need, revision);
    },

    active(need: CompatNeed): boolean {
      const layer = indexedLayer(String(need.composition), String(need.layer));
      if (!layer) return false;
      if (layer.on === false) return false;
      const time = Number(need.time) / TICKS_PER_SECOND;
      const from = Number(layer.from), dur = Number(layer.dur);
      if (!Number.isFinite(from) || !Number.isFinite(dur)) return true;
      const frame = 1 / (Number(PM?.proj?.fps) || 30);
      return time >= from - frame && time < from + dur + frame;
    },

    attach(next: EngineRenderTransport): () => void {
      return attachTo(next);
    },

    reset(): void {
      resetAnswers();
    },

    dispose(): void {
      disposeAll();
    }
  };

  /** Layer lookups by id for the lookahead (it asks for every known
   *  layer at every frame ahead): an index per composition's layer list,
   *  checked on each use and built again when the list changed. */
  const indexes = new Map<string, { list: unknown; at: Map<string, number> }>();
  function indexedLayer(composition: string, id: string): any {
    const proj = PM?.proj;
    if (!proj) return null;
    const rootId = String(proj.compId ?? proj.id ?? '');
    const list: any[] | undefined = composition === rootId ? proj.layers : (proj.comps?.[composition]?.layers ?? proj.layers);
    if (!Array.isArray(list)) return null;
    let index = indexes.get(composition);
    let at = index?.list === list ? index.at.get(id) : undefined;
    if (at === undefined || list[at]?.id !== id) {
      const map = new Map<string, number>();
      list.forEach((l, i) => { if (l && typeof l.id === 'string' && !map.has(l.id)) map.set(l.id, i); });
      index = { list, at: map };
      indexes.set(composition, index);
      at = map.get(id);
    }
    return at === undefined ? null : list[at];
  }

  const readTag = (need: CompatNeed) => `${need.composition}\u0000${need.layer}`;
  function rememberReads(need: CompatNeed, read: boolean): void {
    const tag = readTag(need);
    readsBase.delete(tag);
    if (!read) return;
    readsBase.add(tag);
    if (readsBase.size > 4096) readsBase.delete(readsBase.values().next().value!);
  }

  function answerWith(need: CompatNeed, revision: number, answerOptions: CompatAnswerOptions | undefined, layer: any, chain: Raster, time: number, scale: number, crop: RasterWindowLike | undefined): CompatAnswerResult {
    let result: any;
    producing++;
    active = answerOptions?.prepared === true && answerOptions.layer === undefined
      ? { layer, key: sampleKey(need, revision), scale, time, crop: cropKey(crop) } : null;
    try {
      // `uploaded`: today the compositor's lookup of a raster it holds by
      // key; here none is held (the engine keeps the pixels), so it answers
      // undefined, a lookup miss.
      result = chain(layer, scale, time, NOT_UPLOADED, crop);
    } catch (error) {
      // Today the exception leaves the compositor's draw and fails the
      // frame; here it fails the answer (render-transport.ts reports it,
      // and an export frame fails with its message). The layer is never
      // drawn without the wrapper instead.
      log(`the raster override failed for layer ${String(need.layer)}: ${error instanceof Error ? error.message : String(error)}`);
      throw error;
    } finally {
      producing--;
      if (active) ready.delete(active.key);
      active = null;
    }
    const mark: LazyMark | undefined = result?.[LAZY];
    // The base itself, for this layer and density, untouched: the engine
    // draws it. Its window is the need's, or none: a wrapper that asked for
    // the whole raster (it dropped the preview crop) gets the whole layer
    // drawn, which is what the engine's native draw is (the preview crop
    // only skips pixels outside the view). A wrapper that wrote a field or
    // took its canvas (it may have drawn on it) gets its result drawn as
    // returned.
    if (mark && mark.layer === layer && mark.scale === scale && (mark.crop === crop || mark.crop == null) && !mark.touched) return { kind: 'native' };
    if (result == null) return TRANSPARENT;
    const tag = `${String(need.composition)}\u0000${String(need.layer)}\u0000${need.time}\u0000${scale}\u0000${crop ? `${crop.x},${crop.y},${crop.width},${crop.height}` : ''}`;
    // The engine holds the key in at most MAX_COMPAT_KEY_BYTES of UTF-8; a
    // longer or broken one becomes its digest (a cut would merge keys).
    const ownKey = typeof result.key === 'string' && result.key ? compatKey(result.key) : '';
    if (ownKey && !answerOptions?.fresh && posted.get(tag) === ownKey) return { kind: 'reuse', key: ownKey };
    const source = canvasOf(result);
    if (!source) return TRANSPARENT;
    // Today's quad (`contentQuad`), from the canvas's size.
    const width = Math.round(source.width), height = Math.round(source.height);
    const w = Number(result.w) || width / scale;
    const h = Number(result.h) || height / scale;
    const centered = layer.type === 'shape' && !layer.d?.paths?.length;
    const anchor: [number, number] = centered ? [w / 2, h / 2] : [Number(result.anchorX) || 0, Number(result.anchorY) || 0];
    const uv = resultUv(result, width, height);
    // Today's compositor drew a result whose key it had uploaded from that
    // texture, in any frame, on the quad the result describes. The engine
    // holds the pixels posted under a key; the same key on the same quad at
    // another time is answered \`reuse\`, without reading the canvas again.
    const across = ownKey ? `${String(need.composition)}\u0000${String(need.layer)}\u0000${scale}\u0000${ownKey}` : '';
    const quad = JSON.stringify([width, height, w, h, anchor, uv ?? null, crop ? cropKey(crop) : '']);
    if (across && !answerOptions?.fresh && postedQuads.get(across) === quad) {
      posted.set(tag, ownKey);
      return { kind: 'reuse', key: ownKey };
    }
    const pixels = readPixels(source);
    if (!pixels) return TRANSPARENT;
    const key = ownKey || `powermove:${++serial}:${revision}`;
    if (!answerOptions?.fresh) posted.set(tag, key);
    if (posted.size > 8192) posted.clear();
    if (across) {
      postedQuads.set(across, quad);
      if (postedQuads.size > 8192) postedQuads.delete(postedQuads.keys().next().value!);
    }
    return { kind: 'raster', key, ...pixels, anchor, size: [w, h], ...(uv ? { uv } : {}) };
  }

  async function prepareBase(need: CompatNeed, revision: number): Promise<void> {
    if (!options.engineRasterAsync || !wrapped()) return;
    sampleRevision(revision);
    const layer = findLayer(PM, String(need.composition), String(need.layer));
    if (!layer || (layer.type === 'captions' && PM?.captionsHidden)) return;
    const key = sampleKey(need, revision);
    if (ready.get(key)?.layer === layer) return;
    const baseKey = options.rasterKey?.(layer, Number(need.scale), Number(need.time) / TICKS_PER_SECOND, need.crop ?? undefined) ?? null;
    const cached = baseKey ? bases.get(baseKey) : undefined;
    if (cached) {
      ready.set(key, { layer, surface: cached.surface });
      while (ready.size > 2) ready.delete(ready.keys().next().value!);
      return;
    }
    const existing = preparing.get(key);
    if (existing) { if (existing.layer === layer) await existing.promise; return; }
    if (pending >= 2) return;
    const epoch = generation;
    const crop = need.crop ? { ...need.crop } : undefined;
    pending++;
    const promise = Promise.resolve().then(async () => {
      try {
        const surface = await options.engineRasterAsync!(layer, Number(need.scale), Number(need.time) / TICKS_PER_SECOND, crop);
        if (generation !== epoch || findLayer(PM, String(need.composition), String(need.layer)) !== layer || !wrapped()) return;
        rememberBase(baseKey, layer, surface);
        ready.set(key, { layer, surface });
        while (ready.size > 2) ready.delete(ready.keys().next().value!);
      } catch (error) {
        log(`asynchronous base raster preparation failed: ${error instanceof Error ? error.message : String(error)}`);
        throw error;
      }
      finally { pending--; if (preparing.get(key)?.promise === promise) preparing.delete(key); }
    });
    preparing.set(key, { layer, promise });
    await promise;
  }

  function attachTo(next: EngineRenderTransport): () => void {
    transport = next;
    sync();
    return () => {
      if (transport !== next) return;
      clearReady();
      unanswer?.();
      unanswer = null;
      transport = null;
    };
  }

  function register(target: EngineRenderTransport): () => void {
    return options.engineRasterAsync
      ? target.answerCompat('rasterOverride', (need, revision, o) => handle.answer(need, revision, o), (need, revision) => handle.prepare(need, revision), (need, revision) => handle.ready(need, revision), (need) => handle.active(need))
      : target.answerCompat('rasterOverride', (need, revision, o) => handle.answer(need, revision, o), undefined, undefined, (need) => handle.active(need));
  }

  function resetAnswers(): void {
    posted.clear();
    postedQuads.clear();
    readsBase.clear();
    clearReady();
    if (!transport || !unanswer) return;
    // A producer list without the source drops its answers, then the
    // source comes back: the next frame asks afresh.
    unanswer();
    unanswer = register(transport);
  }

  function disposeAll(): void {
    clearReady();
    chainChange.dispose();
    offFonts?.();
    unanswer?.();
    unanswer = null;
    transport = null;
  }

  function sync(): void {
    clearReady();
    if (!transport) return;
    if (wrapped() && !unanswer) {
      unanswer = register(transport);
    } else if (!wrapped() && unanswer) {
      unanswer();
      unanswer = null;
      posted.clear();
      postedQuads.clear();
      readsBase.clear();
      clearReady();
    } else if (unanswer) {
      // Another wrapper on top, or one of several disposed: what the chain
      // answers may have changed for every layer.
      handle.reset();
    }
  }

  const chainChange = options.onChainChange(sync);
  const fontsOff = PM?.bus?.on?.('fonts', () => handle.reset());
  const offFonts = typeof fontsOff === 'function' ? fontsOff : null;
  return handle;
}
