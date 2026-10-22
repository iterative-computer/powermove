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
 * - Playback prepares complete answers ahead of the clock. A first read of
 *   an unavailable base suspends the attempt; the engine prepares it
 *   asynchronously, then the chain retries with those exact pixels. This
 *   includes detached layers computed inside a wrapper. A completed result
 *   is retained, so presentation never runs a synchronous base snapshot.
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
import { COMPAT_PREFETCH_IN_FLIGHT, COMPAT_PREPARE_IN_FLIGHT } from '../engine-connection/render-transport';
import type { Disposable } from './api';
import { compositorPixels } from './raster-pixels';
import { createRasterReadback } from './raster-readback';
export { compositorPixels } from './raster-pixels';

type LegacyPM = Record<string, any>;
type Raster = (layer: any, scale?: number, time?: number, uploaded?: any, crop?: any) => any;

/** Protocol ticks per second (pm_protocol TICKS_PER_SECOND). */
export const TICKS_PER_SECOND = 705_600_000;

const LAZY = Symbol('powermove.lazyRaster');

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
  /** Async playback readback; synchronous callers retain readPixels. */
  readPixelsAsync?: (source: Parameters<typeof compositorPixels>[0]) => Promise<ReturnType<typeof compositorPixels>>;
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
  /** Prepare an interactive answer and all its base dependencies asynchronously. */
  prepare(need: CompatNeed, revision: number): Promise<void>;
  /** Where base pixels came from (diagnostics; the cadence gates log it). */
  readonly stats: RasterCompatStats;
  dispose(): void;
}

export interface RasterCompatStats {
  /** Base reads answered from this preparation's asynchronous dependencies. */
  prepared: number;
  /** Base reads that waited on the engine synchronously. */
  syncLoads: number;
  /** Asynchronous base preparations started, and reused from the content cache. */
  asyncLoads: number;
  cacheHits: number;
  /** Completed pass-through answers: no base pixels were needed. */
  passThroughSkips: number;
  /** Wall time of the asynchronous preparations (ms). */
  asyncMs: number;
  asyncMsMax: number;
  prepareCpuMs: number;
  prepareCpuMsMax: number;
  slowPreparations: { time: number; ms: number }[];
}

/** Content-identical base surfaces kept per revision (today's raster cache
 *  shares one surface per resolved content, `legacy/gl/raster.ts`). */
export const BASE_CACHE_ENTRIES = 16;
export const BASE_CACHE_BYTES = 32 * 1024 * 1024;
/** Prepared base pixels waiting for their frames' answers. */
export const READY_BYTES = 64 * 1024 * 1024;
/** Includes zero-byte pass-through answers for every layer in the horizon. */
export const READY_ENTRIES = 4096;

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
      value = load();
      done = true;
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

/** The canvas of a surface (`cv`, `canvas`, or `bitmap`). */
function canvasOf(surface: any): (CanvasImageSource & { width: number; height: number }) | null {
  const source = surface?.cv ?? surface?.canvas ?? surface?.bitmap;
  return source && Number(source.width) > 0 && Number(source.height) > 0 ? source : null;
}

/** Today's `uploaded(key)` lookup, answering a miss. */
const NOT_UPLOADED = (_key: string): undefined => undefined;

const TRANSPARENT: CompatAnswerResult = { kind: 'raster', key: 'powermove:empty', width: 1, height: 1, format: 'rgba8Premultiplied', data: new Uint8Array(4), anchor: [0, 0], size: [1, 1] };

/** Give frame delivery and input a turn before speculative wrapper work.
 *  Native replies often arrive together; chaining their continuations as
 *  microtasks would run several wrappers before a presentation event. */
async function preparationTurn(): Promise<void> {
  const scheduler = (globalThis as typeof globalThis & { scheduler?: { postTask: (callback: () => void, options: { priority: 'background' }) => Promise<void> } }).scheduler;
  if (scheduler?.postTask) await scheduler.postTask(() => {}, { priority: 'background' });
}

export function createRasterCompat(PM: LegacyPM, options: RasterCompatOptions): RasterCompatHandle {
  let producing = 0;
  let transport: EngineRenderTransport | null = null;
  let unanswer: (() => void) | null = null;
  /** The key last posted per layer, time and density (for `reuse`). */
  const posted = new Map<string, string>();
  const ready = new Map<string, { layer: unknown; answer: CompatAnswerResult; bytes: number }>();
  type Dependency = { key: string; layer: any; scale: number; time: number; crop?: RasterWindowLike };
  // A missing base unwinds only this preparation attempt. Retry with its
  // real pixels; never hand a wrapper a placeholder it could cache.
  const suspended = Symbol('raster preparation suspended');
  let preparation: { samples: Map<string, any>; missing: Dependency | null; tainted: WeakSet<object>; readback: (() => Promise<CompatAnswerResult>) | null } | null = null;
  const preparing = new Map<string, { layer: unknown; promise: Promise<void> }>();
  const running = new Set<Promise<void>>();
  const bases = new Map<string, { layer: unknown; surface: any; bytes: number }>();
  let baseBytes = 0;
  let pending = 0, generation = 0, readyRevision: number | null = null;
  let serial = 0;
  const stats: RasterCompatStats = { prepared: 0, syncLoads: 0, asyncLoads: 0, cacheHits: 0, passThroughSkips: 0, asyncMs: 0, asyncMsMax: 0, prepareCpuMs: 0, prepareCpuMsMax: 0, slowPreparations: [] };
  const log = options.log ?? ((message: string) => console.warn(`[raster-compat] ${message}`));
  const readPixels = options.readPixels ?? compositorPixels;
  const readback = createRasterReadback();
  const readPixelsAsync = options.readPixelsAsync ?? (options.readPixels ? async (source: Parameters<typeof compositorPixels>[0]) => options.readPixels!(source) : readback.read);

  const cropKey = (crop: RasterWindowLike | undefined) => crop ? `${crop.x},${crop.y},${crop.width},${crop.height}` : '';
  const sampleKey = (need: CompatNeed, revision: number) => `${need.composition}\u0000${need.layer}\u0000${need.time}\u0000${need.scale}\u0000${cropKey(need.crop ?? undefined)}\u0000${revision}`;
  const sampleRevision = (revision: number) => {
    if (readyRevision === revision) return;
    readyRevision = revision;
    ready.clear(); bases.clear(); baseBytes = 0;
    generation++;
  };
  const clearReady = () => { ready.clear(); preparing.clear(); bases.clear(); baseBytes = 0; readyRevision = null; generation++; };
  const rememberBase = (key: string | null, layer: unknown, surface: any) => {
    if (!key || !surface) return;
    const bytes = Number(surface.cv?.width) * Number(surface.cv?.height) * 4;
    if (!(bytes > 0 && bytes <= BASE_CACHE_BYTES)) return;
    const old = bases.get(key);
    if (old) baseBytes -= old.bytes;
    bases.delete(key);
    bases.set(key, { layer, surface, bytes }); baseBytes += bytes;
    while (bases.size > BASE_CACHE_ENTRIES || baseBytes > BASE_CACHE_BYTES) {
      const first = bases.keys().next().value!;
      baseBytes -= bases.get(first)!.bytes; bases.delete(first);
    }
  };

  /** Prepared samples wait for their frame's answer: at most the whole
   *  prefetch horizon, within today's raster budget. */
  const keepReady = () => {
    let bytes = 0;
    for (const entry of ready.values()) bytes += entry.bytes;
    while (ready.size > READY_ENTRIES || (ready.size > 1 && bytes > READY_BYTES)) {
      const first = ready.keys().next().value!;
      bytes -= ready.get(first)!.bytes;
      ready.delete(first);
    }
  };

  /** A cached base for `key`, most recently used first out of eviction. */
  const cachedBase = (key: string | null) => {
    const entry = key ? bases.get(key) : undefined;
    if (!entry) return undefined;
    bases.delete(key!); bases.set(key!, entry);
    return entry;
  };

  const wrapped = (): boolean => {
    const chain = options.chain();
    return typeof chain === 'function' && !options.isBase(chain);
  };

  const handle: RasterCompatHandle = {
    get producing() { return producing > 0; },

    stats,

    wrapped,

    base([layer, scale = 1, time, , crop]: Parameters<Raster>): any {
      const s = Number(scale) || 1;
      const t = Number(time ?? PM?.time ?? 0);
      // Content identity covers the answered layer and the detached copies a
      // wrapper rasterizes through the service (a typed prefix, an outline).
      const load = () => {
        const key = options.rasterKey?.(layer, s, t, crop ?? undefined) ?? null;
        const cached = cachedBase(key);
        if (cached) { stats.cacheHits++; return cached.surface; }
        if (preparation) {
          const dependencyKey = key ?? JSON.stringify([layer, s, t, crop ?? null]);
          if (preparation.samples.has(dependencyKey)) {
            stats.prepared++;
            return preparation.samples.get(dependencyKey);
          }
          preparation.missing = { key: dependencyKey, layer, scale: s, time: t, crop: crop ?? undefined };
          throw suspended;
        }
        stats.syncLoads++;
        const surface = options.engineRaster(layer, s, t, crop ?? undefined);
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
      const preparedKey = sampleKey(need, revision);
      const completed = ready.get(preparedKey);
      if (!preparation && answerOptions?.prepared && answerOptions.layer === undefined) {
        // A reset/eviction cannot turn an asynchronous playback answer into
        // a synchronous raster wait on the presentation thread.
        if (!completed || completed.layer !== layer) return null;
        ready.delete(preparedKey);
        return completed.answer;
      }
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
      let result: any;
      producing++;
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
        if (error === suspended || preparation?.missing) throw error;
        log(`the raster override failed for layer ${String(need.layer)}: ${error instanceof Error ? error.message : String(error)}`);
        throw error;
      } finally {
        producing--;
      }
      if (preparation?.missing) {
        // A wrapper may catch the suspension. Discard its incomplete result
        // and detect it if the wrapper cached that object before its base
        // was available; such a result can never become the viewer's pixels.
        if (result && typeof result === 'object') preparation.tainted.add(result);
        throw suspended;
      }
      if (result && typeof result === 'object' && preparation?.tainted.has(result)) throw new Error('The raster override cached a result before its base pixels were available.');
      const mark: LazyMark | undefined = result?.[LAZY];
      // The base itself, for this layer, density and window, untouched: the
      // engine draws it. A wrapper that wrote a field or took its canvas
      // (it may have drawn on it) gets its result drawn as returned.
      if (mark && mark.layer === layer && mark.scale === scale && mark.crop === crop && !mark.touched) return { kind: 'native' };
      if (result == null) return TRANSPARENT;
      const tag = `${String(need.composition)}\u0000${String(need.layer)}\u0000${need.time}\u0000${scale}\u0000${crop ? `${crop.x},${crop.y},${crop.width},${crop.height}` : ''}`;
      const ownKey = typeof result.key === 'string' && result.key ? result.key : '';
      if (ownKey && !answerOptions?.fresh && posted.get(tag) === ownKey) return { kind: 'reuse', key: ownKey };
      const source = canvasOf(result);
      if (!source) return TRANSPARENT;
      // Freeze geometry too: a wrapper may reuse and mutate its surface
      // object for the next frame while this frame's pixels are in flight.
      const w = Number(result.w) || source.width / scale;
      const h = Number(result.h) || source.height / scale;
      const centered = layer.type === 'shape' && !layer.d?.paths?.length;
      const anchor: [number, number] = centered ? [w / 2, h / 2] : [Number(result.anchorX) || 0, Number(result.anchorY) || 0];
      const window = resultUv(result, source.width, source.height);
      const uv = window ? [...window] as [number, number, number, number] : undefined;
      const key = (ownKey || `powermove:${++serial}:${revision}`).slice(0, 512);
      const finish = (pixels: ReturnType<typeof compositorPixels>): CompatAnswerResult => {
        if (!pixels) return TRANSPARENT;
        if (!answerOptions?.fresh && !preparation) posted.set(tag, key);
        if (posted.size > 8192) posted.clear();
        return { kind: 'raster', key, ...pixels, anchor, size: [w, h], ...(uv ? { uv } : {}) };
      };
      if (preparation) {
        // Keep the completed canvas. Reading it back on this thread can
        // wait hundreds of milliseconds on the GPU under load; the worker
        // uses the same premultiplied WebGL upload/readback as sync callers.
        preparation.readback = async () => finish(await readPixelsAsync(source));
        return null;
      }
      return finish(readPixels(source));
    },

    async prepare(need: CompatNeed, revision: number): Promise<void> {
      if (!options.engineRasterAsync || !wrapped()) return;
      sampleRevision(revision);
      const layer = findLayer(PM, String(need.composition), String(need.layer));
      if (!layer || (layer.type === 'captions' && PM?.captionsHidden)) return;
      const key = sampleKey(need, revision);
      if (ready.get(key)?.layer === layer) return;
      const existing = preparing.get(key);
      if (existing) { if (existing.layer === layer) await existing.promise; return; }
      if (pending >= COMPAT_PREPARE_IN_FLIGHT + COMPAT_PREFETCH_IN_FLIGHT) {
        const waitingGeneration = generation;
        await Promise.race(running);
        if (generation !== waitingGeneration) return;
        return handle.prepare(need, revision);
      }
      const epoch = generation;
      pending++;
      const valid = () => generation === epoch && findLayer(PM, String(need.composition), String(need.layer)) === layer && wrapped();
      const promise = Promise.resolve().then(async () => {
        const samples = new Map<string, any>();
        const tainted = new WeakSet<object>();
        try {
          // A wrapper may ask for detached/retimed/cropped bases only after
          // reading an earlier one (the typewriter's prefix). Discover each
          // dependency with the actual previous pixels, then resume on a
          // later turn. The finite bound also rejects a nonconvergent chain.
          for (let phase = 0; phase < 64 && valid(); phase++) {
            await preparationTurn();
            if (!valid()) return;
            const attempt = { samples, tainted, missing: null as Dependency | null, readback: null as (() => Promise<CompatAnswerResult>) | null };
            let answer: CompatAnswerResult = null;
            preparation = attempt;
            const beganCpu = performance.now();
            try { answer = handle.answer(need, revision, { fresh: true }); }
            catch (error) { if (!attempt.missing) throw error; }
            finally {
              preparation = null;
              const ms = performance.now() - beganCpu;
              stats.prepareCpuMs += ms; stats.prepareCpuMsMax = Math.max(stats.prepareCpuMsMax, ms);
              if (ms > 8) { stats.slowPreparations.push({ time: Number(need.time) / TICKS_PER_SECOND, ms }); if (stats.slowPreparations.length > 20) stats.slowPreparations.shift(); }
            }
            const missing = attempt.missing;
            if (!missing) {
              if (attempt.readback) answer = await attempt.readback();
              if (!valid()) return;
              if (answer?.kind === 'native') stats.passThroughSkips++;
              ready.set(key, { layer, answer, bytes: answer?.kind === 'raster' ? answer.data.byteLength : 0 });
              keepReady();
              return;
            }
            stats.asyncLoads++;
            const began = performance.now();
            const surface = await options.engineRasterAsync!(missing.layer, missing.scale, missing.time, missing.crop);
            const ms = performance.now() - began;
            stats.asyncMs += ms; stats.asyncMsMax = Math.max(stats.asyncMsMax, ms);
            if (!valid()) return;
            samples.set(missing.key, surface);
            const baseKey = options.rasterKey?.(missing.layer, missing.scale, missing.time, missing.crop) ?? null;
            rememberBase(baseKey, missing.layer, surface);
          }
          if (valid()) throw new Error('The raster override did not converge after 64 asynchronous base dependencies.');
        } catch (error) {
          log(`asynchronous raster preparation failed: ${error instanceof Error ? error.message : String(error)}`);
          throw error;
        }
        finally { pending--; running.delete(promise); if (preparing.get(key)?.promise === promise) preparing.delete(key); }
      });
      preparing.set(key, { layer, promise });
      running.add(promise);
      await promise;
    },

    attach(next: EngineRenderTransport): () => void {
      transport = next;
      sync();
      return () => {
        if (transport !== next) return;
        clearReady();
        unanswer?.();
        unanswer = null;
        transport = null;
      };
    },

    reset(): void {
      posted.clear();
      clearReady();
      if (!transport || !unanswer) return;
      // A producer list without the source drops its answers, then the
      // source comes back: the next frame asks afresh.
      unanswer();
      unanswer = transport.answerCompat('rasterOverride', (need, revision, options) => handle.answer(need, revision, options), options.engineRasterAsync ? (need, revision) => handle.prepare(need, revision) : undefined);
    },

    dispose(): void {
      readback.dispose();
      clearReady();
      chainChange.dispose();
      offFonts?.();
      unanswer?.();
      unanswer = null;
      transport = null;
    }
  };

  function sync(): void {
    clearReady();
    if (!transport) return;
    if (wrapped() && !unanswer) {
      unanswer = transport.answerCompat('rasterOverride', (need, revision, options) => handle.answer(need, revision, options), options.engineRasterAsync ? (need, revision) => handle.prepare(need, revision) : undefined);
    } else if (!wrapped() && unanswer) {
      unanswer();
      unanswer = null;
      posted.clear();
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
