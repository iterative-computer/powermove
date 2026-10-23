/** Source media consumers. Decoding, source color and audio belong to the engine. */
import type { AudioMessage } from '../../../shared/engine-protocol';
import type { EngineBridge, EngineMediaJobRequest } from '../../../shared/engine-ipc';
import { EngineSnapshotError, type EngineSnapshot, type EngineRenderTransport } from './render-transport';
import type { DocumentSession } from './session';
import { TICKS_PER_SECOND, ticks } from './transport';

export const SOURCE_SERVICE = 'engine.source';
const CACHE_BYTES = 64 * 1024 * 1024;
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
  private readonly posters = new Map<string, { key: string; url: string }>();
  private readonly pendingPosters = new Map<string, string>();
  private monitorValue: SourceMonitor | null = null;
  private disposed = false;
  private generation = 0;

  constructor(private readonly host: Host) {}
  nextGeneration(): number {
    if (this.generation >= 0xffffffff) throw new Error('The source monitor command counter is exhausted.');
    return ++this.generation;
  }

  private identity(asset: string): string {
    const project = this.host.project(), meta = project.assets?.[asset];
    if (!meta) throw new EngineSnapshotError('mediaOffline', 'This source is no longer in the project.');
    return JSON.stringify([project.id, asset, meta.contentHash, meta.link, meta.storageKey, meta.size, meta.interpret]);
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
    await cancellable(this.host.bind(), options.signal);
    if (this.disposed || identity !== this.identity(asset)) throw abort();
    const started = performance.now();
    const snapshot = await cancellable(this.host.render.snapshot({ time, width, height, pixels: 'straight', signal: options.signal,
      source: { asset, fit: options.fit, color: options.color } }, options.idle ? 'idle' : 'async'), options.signal);
    if (this.disposed || identity !== this.identity(asset)) throw abort();
    this.stats.frames++; this.stats.frameMs += performance.now() - started;
    if (snapshot.data.byteLength <= CACHE_BYTES) {
      this.bytes -= this.cache.get(key)?.data.byteLength ?? 0;
      this.cache.set(key, snapshot); this.bytes += snapshot.data.byteLength;
      while (this.bytes > CACHE_BYTES && this.cache.size) {
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
    await cancellable(this.host.bind(), signal);
    return { asset, revision, project: frozen, requestId: crypto.randomUUID() };
  }

  async filmstrip(asset: string, times: number[], width: number, height: number, options: FrameOptions = {}): Promise<Uint8Array[]> {
    if (!this.host.native.sourceFilmstrip) throw new EngineSnapshotError('unavailable', 'Source filmstrips are unavailable.');
    const request = await this.jobRequest(asset, options.signal);
    const result = await cancellable(this.host.native.sourceFilmstrip({ ...request, times, width, height,
      fit: options.fit ?? 'stretch', color: options.color ?? 'display' }), options.signal,
    () => { this.stats.cancelled++; void this.host.native.cancelMediaJob?.(request.requestId); });
    this.stats.filmstrips++;
    return result.frames;
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
    return this.monitorValue = new SourceMonitor(this, asset, this.host);
  }

  receive(message: AudioMessage): void {
    if (message.audio === 'sourceStatus') this.monitorValue?.receive(message);
  }

  reset(): void {
    this.monitorValue?.dispose(); this.monitorValue = null;
    for (const poster of this.posters.values()) URL.revokeObjectURL(poster.url);
    this.posters.clear(); this.pendingPosters.clear(); this.cache.clear(); this.bytes = 0;
  }

  dispose(): void { this.disposed = true; this.reset(); }
}

/** Media-element shaped state for the existing source monitor controls. */
export class SourceMonitor extends EventTarget {
  error: Error | null = null;
  readonly canvas = document.createElement('canvas');
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
  private fail(error: unknown): void {
    if (this.disposed) return;
    this.error = error instanceof Error ? error : new Error(String(error));
    this.host.error(error);
    this.dispatchEvent(new ErrorEvent('error', { message: error instanceof Error ? error.message : String(error) }));
  }

  constructor(private readonly source: EngineSourceMedia, readonly asset: string, private readonly host: Host) {
    super();
    this.duration = Number(host.project().assets?.[asset]?.dur) || 0;
    this.canvas.width = this.canvas.height = 1;
    this.resize = new ResizeObserver(() => void this.paint());
    this.resize.observe(this.canvas);
    void this.command(false);
    queueMicrotask(() => { if (this.canvas.isConnected) void this.paint(); });
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
      await this.host.bind(); await this.host.wake();
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
    const was = this.advancing;
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
    void this.paint();
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
      const width = Math.max(1, Math.round((this.canvas.clientWidth || meta?.w || 1) * dpr));
      const height = Math.max(1, Math.round((this.canvas.clientHeight || meta?.h || 1) * dpr));
      const snapshot = await this.source.frame(this.asset, this.currentTime, width, height, { fit: 'contain', signal: AbortSignal.any([this.cancel.signal, controller.signal]) });
      if (this.disposed || this.requested !== generation) return;
      if (this.canvas.width !== width) this.canvas.width = width;
      if (this.canvas.height !== height) this.canvas.height = height;
      this.canvas.getContext('2d')?.putImageData(new ImageData(new Uint8ClampedArray(snapshot.data), width, height), 0, 0);
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
    this.canvas.width = this.canvas.height = 1;
  }
}
