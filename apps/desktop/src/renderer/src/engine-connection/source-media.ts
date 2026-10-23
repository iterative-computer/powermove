/** Source media consumers. Decoding, source color and audio belong to the engine. */
import type { AudioMessage } from '../../../shared/engine-protocol';
import type { EngineBridge, EngineMediaJobRequest } from '../../../shared/engine-ipc';
import { EngineSnapshotError, type EngineSnapshot, type EngineRenderTransport } from './render-transport';
import type { DocumentSession } from './session';
import { TICKS_PER_SECOND, ticks } from './transport';

export const SOURCE_SERVICE = 'engine.source';
const CACHE_BYTES = 64 * 1024 * 1024;
const FILMSTRIP_CACHE_BYTES = CACHE_BYTES / 4;
type Fit = 'stretch' | 'contain' | 'cover';
type Color = 'display' | 'interpreted';
type FrameOptions = { signal?: AbortSignal; fit?: Fit; color?: Color; idle?: boolean };
type Host = {
  render: EngineRenderTransport;
  native: EngineBridge;
  project(): Record<string, any>;
  revision(): number;
  session(): DocumentSession | null;
  wake(): Promise<void>;
  bind(): Promise<void>;
  changed(): void;
  error(error: unknown): void;
};

const abort = () => new DOMException('The source request was cancelled.', 'AbortError');
function cancellable<T>(promise: Promise<T>, signal?: AbortSignal, cancel?: () => void): Promise<T> {
  if (!signal) return promise;
  if (signal.aborted) { cancel?.(); return Promise.reject(abort()); }
  return new Promise((resolve, reject) => {
    const stopped = () => { cancel?.(); reject(abort()); };
    signal.addEventListener('abort', stopped, { once: true });
    promise.then(resolve, reject).finally(() => signal.removeEventListener('abort', stopped));
  });
}

/** Same rounding and poster time as capturePoster; no browser media reads. */
export function sourcePosterSize(asset: Record<string, any>, edge = 320): { width: number; height: number } | null {
  const w = Number(asset.w), h = Number(asset.h);
  if (!(w > 0 && h > 0 && edge > 0)) return null;
  const scale = edge / Math.max(w, h);
  return { width: Math.max(1, Math.round(w * scale)), height: Math.max(1, Math.round(h * scale)) };
}

export class EngineSourceMedia {
  readonly stats = { frames: 0, hits: 0, filmstrips: 0, renditions: 0, cancelled: 0, frameMs: 0 };
  private readonly cache = new Map<string, EngineSnapshot>();
  private bytes = 0;
  private readonly strips = new Map<string, Uint8Array[]>();
  private stripBytes = 0;
  private readonly posters = new Map<string, { key: string; url: string }>();
  private readonly pendingPosters = new Map<string, string>();
  private readonly bound = new Map<string, { identity: string; promise: Promise<void> }>();
  private readonly prepared = new Map<string, Promise<void>>();
  private warming = 0;
  private monitorValue: SourceMonitor | null = null;
  private disposed = false;
  private generation = 0;
  private lifetime = 0;

  constructor(private readonly host: Host) {}
  nextGeneration(): number {
    if (this.generation >= 0xffffffff) throw new Error('The source monitor command counter is exhausted.');
    return ++this.generation;
  }

  private identity(asset: string): string {
    const project = this.host.project(), meta = project.assets?.[asset];
    if (!meta) throw new EngineSnapshotError('mediaOffline', 'This source is no longer in the project.');
    const { poster: _poster, ...reading } = meta;
    return JSON.stringify([project.id, asset, reading]);
  }

  /** A source's acknowledged binding stays valid until its reading changes. */
  bindSource(asset: string): Promise<void> {
    if (this.disposed) return Promise.reject(abort());
    const identity = this.identity(asset), cached = this.bound.get(asset);
    if (cached?.identity === identity) return cached.promise;
    const pending = this.host.bind().catch(error => {
      if (this.bound.get(asset)?.promise === pending) this.bound.delete(asset);
      throw error;
    });
    this.bound.delete(asset); this.bound.set(asset, { identity, promise: pending });
    while (this.bound.size > 128) this.bound.delete(this.bound.keys().next().value!);
    return pending;
  }

  /** Prime the real job lane from the source surface, before a filmstrip is needed. */
  private prepare(asset: string, time: number, width: number, height: number, opportunistic = false): void {
    if (!this.host.native.prepareSource || this.disposed || (opportunistic && this.warming)) return;
    const key = JSON.stringify([this.identity(asset), this.host.revision(), time, width, height]);
    if (this.prepared.has(key)) return;
    this.warming++;
    const lifetime = this.lifetime;
    const pending = this.jobRequest(asset).then(request => {
      if (this.disposed || this.lifetime !== lifetime) throw abort();
      return this.host.native.prepareSource!({ ...request, time, width, height, fit: 'stretch', color: 'display' });
    }).catch(error => {
      if (this.prepared.get(key) === pending) this.prepared.delete(key);
      // Preparation is speculative. An explicit frame/job still reports its
      // own failure; a later surface can retry preparation after a relink.
    }).finally(() => { this.warming--; });
    this.prepared.set(key, pending);
    while (this.prepared.size > 32) this.prepared.delete(this.prepared.keys().next().value!);
  }

  async frame(asset: string, time: number, width: number, height: number, options: FrameOptions = {}): Promise<EngineSnapshot> {
    if (this.disposed || options.signal?.aborted) throw abort();
    if (!Number.isFinite(time) || time < 0 || !Number.isSafeInteger(ticks(time))) throw new EngineSnapshotError('invalidMessage', 'A source frame needs a finite, non-negative source time.');
    const identity = this.identity(asset);
    const key = JSON.stringify([identity, ticks(time), width, height, options.fit ?? 'stretch', options.color ?? 'display']);
    const cached = this.cache.get(key);
    if (cached) {
      this.stats.hits++;
      this.cache.delete(key); this.cache.set(key, cached);
      return cached;
    }
    await cancellable(this.bindSource(asset), options.signal);
    if (this.disposed || identity !== this.identity(asset)) throw abort();
    const started = performance.now();
    const snapshot = await cancellable(this.host.render.snapshot({ time, width, height, pixels: 'straight', signal: options.signal,
      source: { asset, fit: options.fit, color: options.color } }, options.idle ? 'idle' : 'async'), options.signal);
    if (this.disposed || identity !== this.identity(asset)) throw abort();
    this.stats.frames++; this.stats.frameMs += performance.now() - started;
    if (snapshot.data.byteLength <= CACHE_BYTES) {
      this.bytes -= this.cache.get(key)?.data.byteLength ?? 0;
      this.cache.set(key, snapshot); this.bytes += snapshot.data.byteLength;
      while (this.bytes > CACHE_BYTES - FILMSTRIP_CACHE_BYTES && this.cache.size) {
        const first = this.cache.keys().next().value!;
        this.bytes -= this.cache.get(first)!.data.byteLength; this.cache.delete(first);
      }
    }
    return snapshot;
  }

  async poster(asset: string, edge = 320, signal?: AbortSignal): Promise<Blob | null> {
    const meta = this.host.project().assets?.[asset];
    if (!meta || !['image', 'video'].includes(meta.kind)) return null;
    const size = sourcePosterSize(meta, edge);
    if (!size) return null;
    const time = meta.kind === 'video' ? Math.min(.5, Math.max(0, Number(meta.dur) || 0) * .1) : 0;
    this.prepare(asset, time, size.width, size.height, true);
    const snapshot = await this.frame(asset, time, size.width, size.height, { signal });
    const canvas = this.host.render.toCanvas(snapshot);
    const encode = (type: string) => new Promise<Blob | null>(resolve => canvas.toBlob(resolve, type, .8));
    return await encode('image/webp') || await encode('image/jpeg');
  }

  /** Render actions call this synchronously; arrival invalidates existing asset rows. */
  posterUrl(asset: string): string {
    const key = this.identity(asset);
    const previous = this.posters.get(asset);
    if (previous?.key === key) return previous.url;
    if (this.pendingPosters.get(asset) !== key) {
      this.pendingPosters.set(asset, key);
      void this.poster(asset).then(blob => {
        if (!blob || this.disposed || this.pendingPosters.get(asset) !== key || key !== this.identity(asset)) return;
        if (previous) URL.revokeObjectURL(previous.url);
        this.posters.set(asset, { key, url: URL.createObjectURL(blob) }); this.host.changed();
      }).catch(error => this.host.error(error)).finally(() => {
        if (this.pendingPosters.get(asset) === key) this.pendingPosters.delete(asset);
      });
    }
    return '';
  }

  private async jobRequest(asset: string, signal?: AbortSignal): Promise<EngineMediaJobRequest> {
    const project = this.host.project();
    const revision = this.host.revision();
    // Copy before awaiting bindings, so later edits never mix source identities.
    const frozen = JSON.stringify(project);
    await cancellable(this.bindSource(asset), signal);
    return { asset, revision, project: frozen, requestId: crypto.randomUUID() };
  }

  async filmstrip(asset: string, times: number[], width: number, height: number, options: FrameOptions = {}): Promise<Uint8Array[]> {
    if (this.disposed || options.signal?.aborted) throw abort();
    if (!this.host.native.sourceFilmstrip) throw new EngineSnapshotError('unavailable', 'Source filmstrips are unavailable.');
    const identity = this.identity(asset), requestedTimes = [...times];
    const key = JSON.stringify([identity, requestedTimes.map(ticks), width, height, options.fit ?? 'stretch', options.color ?? 'display']);
    const cached = this.strips.get(key);
    if (cached) {
      this.strips.delete(key); this.strips.set(key, cached);
      return cached.map(frame => frame.slice());
    }
    const lifetime = this.lifetime;
    const request = await this.jobRequest(asset, options.signal);
    if (this.disposed || lifetime !== this.lifetime) throw abort();
    const result = await cancellable(this.host.native.sourceFilmstrip({ ...request, times: requestedTimes, width, height,
      fit: options.fit ?? 'stretch', color: options.color ?? 'display' }), options.signal,
    () => { this.stats.cancelled++; void this.host.native.cancelMediaJob?.(request.requestId); });
    this.stats.filmstrips++;
    const frameBytes = width * height * 4;
    if (result.width !== width || result.height !== height || result.count !== requestedTimes.length || result.data.byteLength !== frameBytes * requestedTimes.length) {
      throw new EngineSnapshotError('internal', 'The engine returned an incomplete filmstrip.');
    }
    const frames = requestedTimes.map((_, i) => result.data.subarray(i * frameBytes, (i + 1) * frameBytes));
    if (!this.disposed && lifetime === this.lifetime && this.host.project().assets?.[asset] && identity === this.identity(asset) && result.data.byteLength <= FILMSTRIP_CACHE_BYTES) {
      // Retain one packed backing buffer; callers receive their own mutable
      // frame copies so edits cannot corrupt a later filmstrip's pixels.
      this.stripBytes -= this.strips.get(key)?.reduce((n, frame) => n + frame.byteLength, 0) ?? 0;
      this.strips.delete(key); this.strips.set(key, frames); this.stripBytes += result.data.byteLength;
      while (this.stripBytes > FILMSTRIP_CACHE_BYTES && this.strips.size) {
        const oldest = this.strips.keys().next().value!;
        this.stripBytes -= this.strips.get(oldest)!.reduce((n, frame) => n + frame.byteLength, 0);
        this.strips.delete(oldest);
      }
      return frames.map(frame => frame.slice());
    }
    return frames;
  }

  async rendition(project: Record<string, any>, asset: string, format: 'png' | 'webm', signal?: AbortSignal): Promise<Blob> {
    if (!this.host.native.browserRendition) throw new EngineSnapshotError('unavailable', 'Browser renditions are unavailable.');
    const request = { asset, format, project: JSON.stringify(project), revision: this.host.revision(), requestId: crypto.randomUUID() };
    await cancellable(this.host.bind(), signal);
    const result = await cancellable(this.host.native.browserRendition(request), signal,
      () => { this.stats.cancelled++; void this.host.native.cancelMediaJob?.(request.requestId); });
    this.stats.renditions++;
    return new Blob([new Uint8Array(result.data)], { type: result.type });
  }

  monitor(asset: string): SourceMonitor {
    this.monitorValue?.dispose();
    const meta = this.host.project().assets?.[asset];
    const size = meta && ['image', 'video'].includes(meta.kind) ? sourcePosterSize(meta) : null;
    if (size) this.prepare(asset, 0, size.width, size.height);
    return this.monitorValue = new SourceMonitor(this, asset, this.host);
  }

  receive(message: AudioMessage): void {
    if (message.audio === 'sourceStatus') this.monitorValue?.receive(message);
  }

  reset(): void {
    this.lifetime++;
    this.monitorValue?.dispose(); this.monitorValue = null;
    for (const poster of this.posters.values()) URL.revokeObjectURL(poster.url);
    this.posters.clear(); this.pendingPosters.clear(); this.cache.clear(); this.bytes = 0;
    this.bound.clear(); this.prepared.clear();
    this.strips.clear(); this.stripBytes = 0;
    void this.host.native.releasePreparedSources?.().catch(() => {});
  }

  dispose(): void { this.disposed = true; this.reset(); }
}

/** Media-element shaped state for the existing source monitor controls. */
export class SourceMonitor extends EventTarget {
  error: Error | null = null;
  readonly canvas = document.createElement('canvas');
  readonly surface: HTMLCanvasElement | HTMLImageElement;
  readonly duration: number;
  private at = 0;
  private arrived = performance.now();
  private advancing = false;
  private disposed = false;
  private requested = 0;
  private generation = 0;
  levels = { peak: [] as number[], rms: [] as number[] };
  private rendering = false;
  private paintAgain = false;
  private frame = 0;
  private frameAbort: AbortController | null = null;
  private readonly cancel = new AbortController();
  private readonly resize: ResizeObserver;
  private imageUrl: string | null = null;
  private fail(error: unknown): void {
    if (this.disposed) return;
    this.error = error instanceof Error ? error : new Error(String(error));
    this.host.error(error);
    this.dispatchEvent(new ErrorEvent('error', { message: error instanceof Error ? error.message : String(error) }));
  }

  constructor(private readonly source: EngineSourceMedia, readonly asset: string, private readonly host: Host) {
    super();
    const meta = host.project().assets?.[asset];
    this.duration = Number(meta?.dur) || 0;
    // Still images used to paint as an <img> below the transport. A canvas
    // changes Chromium's shadow compositing even when its RGBA bytes match.
    // Keep that presentation while every pixel still comes from the engine.
    this.surface = meta?.kind === 'image' ? document.createElement('img') : this.canvas;
    if (this.surface instanceof HTMLImageElement) this.surface.alt = '';
    this.canvas.width = this.canvas.height = 1;
    this.resize = new ResizeObserver(() => void this.paint());
    this.resize.observe(this.surface);
    void this.command(false);
    queueMicrotask(() => { if (this.surface.isConnected) void this.paint(); });
  }

  get paused(): boolean { return !this.advancing; }
  get currentTime(): number { return Math.min(this.duration || Infinity, this.at + (this.advancing ? Math.max(0, performance.now() - this.arrived) / 1000 : 0)); }
  set currentTime(value: number) {
    this.frameAbort?.abort();
    this.at = Math.max(0, Math.min(this.duration || Infinity, value));
    this.arrived = performance.now();
    void this.command(this.advancing); void this.paint();
    this.dispatchEvent(new Event('timeupdate'));
  }

  private async command(playing: boolean): Promise<void> {
    this.error = null;
    const request = ++this.requested;
    const generation = this.generation = this.source.nextGeneration();
    const time = ticks(this.currentTime);
    try {
      await this.source.bindSource(this.asset); await this.host.wake();
      if (this.disposed || request !== this.requested) return;
      this.host.session()?.audio({ audio: 'sourceMonitor', generation, asset: this.asset, time, playing });
    } catch (error) { this.fail(error); }
  }

  async play(): Promise<void> {
    if (this.disposed) return;
    this.arrived = performance.now();
    this.advancing = true;
    await this.command(true);
    if (this.disposed) return;
    this.dispatchEvent(new Event('play')); this.follow();
  }

  pause(): void {
    this.at = this.currentTime;
    this.advancing = false; void this.command(false);
    if (this.frame) cancelAnimationFrame(this.frame); this.frame = 0;
    this.dispatchEvent(new Event('pause'));
  }

  receive(message: Extract<AudioMessage, { audio: 'sourceStatus' }>): void {
    if (message.asset !== this.asset || message.generation !== this.generation || this.disposed) return;
    const was = this.advancing, before = this.currentTime;
    this.at = message.time / TICKS_PER_SECOND;
    this.arrived = performance.now();
    this.advancing = message.playing;
    this.levels = message.levels;
    this.dispatchEvent(new Event('timeupdate'));
    if (was && !this.advancing) this.dispatchEvent(new Event(this.at >= this.duration ? 'ended' : 'pause'));
    if (!was && this.advancing) this.dispatchEvent(new Event('play'));
    if (message.error) this.fail(new Error(message.error));
    if (this.advancing) this.follow();
    else if (this.frame) { cancelAnimationFrame(this.frame); this.frame = 0; }
    if (was || this.advancing || before !== this.currentTime) void this.paint();
  }

  private follow(): void {
    if (!this.advancing || this.disposed || this.frame) return;
    this.frame = requestAnimationFrame(() => { this.frame = 0; void this.paint(); this.follow(); });
  }

  private async paint(): Promise<void> {
    if (this.disposed || this.host.project().assets?.[this.asset]?.kind === 'audio') return;
    if (this.rendering) { this.paintAgain = true; return; }
    this.rendering = true;
    const controller = this.frameAbort = new AbortController();
    const generation = this.requested;
    try {
      const meta = this.host.project().assets?.[this.asset];
      const dpr = devicePixelRatio || 1;
      const width = Math.max(1, Math.round((this.surface.clientWidth || meta?.w || 1) * dpr));
      const height = Math.max(1, Math.round((this.surface.clientHeight || meta?.h || 1) * dpr));
      const snapshot = await this.source.frame(this.asset, this.currentTime, width, height, { fit: 'contain', signal: AbortSignal.any([this.cancel.signal, controller.signal]) });
      if (this.disposed || this.requested !== generation) return;
      if (this.canvas.width !== width) this.canvas.width = width;
      if (this.canvas.height !== height) this.canvas.height = height;
      this.canvas.getContext('2d')?.putImageData(new ImageData(new Uint8ClampedArray(snapshot.data), width, height), 0, 0);
      if (this.surface instanceof HTMLImageElement) {
        const blob = await new Promise<Blob | null>(resolve => this.canvas.toBlob(resolve, 'image/png'));
        if (!blob) throw new Error('The source image could not be presented.');
        if (this.disposed || this.requested !== generation) return;
        const previous = this.imageUrl;
        this.surface.src = this.imageUrl = URL.createObjectURL(blob);
        try { await this.surface.decode(); }
        finally { if (previous) URL.revokeObjectURL(previous); }
        if (this.disposed || this.requested !== generation) return;
      }
      this.dispatchEvent(new Event('loadeddata'));
    } catch (error) { if (!controller.signal.aborted) this.fail(error); }
    finally {
      if (this.frameAbort === controller) this.frameAbort = null;
      this.rendering = false;
      if (this.paintAgain) { this.paintAgain = false; void this.paint(); }
    }
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true; this.requested++; this.cancel.abort(); this.resize.disconnect();
    if (this.frame) cancelAnimationFrame(this.frame); this.frame = 0;
    this.host.session()?.audio({ audio: 'sourceMonitor', generation: this.source.nextGeneration(), asset: null, time: 0, playing: false });
    if (this.imageUrl) { URL.revokeObjectURL(this.imageUrl); this.imageUrl = null; }
    if (this.surface instanceof HTMLImageElement) this.surface.removeAttribute('src');
    this.canvas.width = this.canvas.height = 1;
  }
}
