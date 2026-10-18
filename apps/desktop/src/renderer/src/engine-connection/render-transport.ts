/*
 * The renderer half of the Phase 3c render transport (plan sections 18 to
 * 20; docs/engine/rfcs/16-render-transport.md), behind "Use New Engine".
 *
 * `EngineRenderTransport` is what the kernel's render facade, the
 * extension compatibility layer, and the UI's consumers call for pixels
 * from the engine:
 *
 * - **Synchronous snapshots** (`snapshotSync`, and on top of it
 *   `renderFrameToSync`, `snapshotDataUrlSync`, `rasterSync`): the calling
 *   thread blocks in `sendSync` while main asks the interactive engine.
 *   Pixels are pinned to the window's current revision (pending edits are
 *   flushed to the engine first) and tagged with the revision and view
 *   they show. The engine never waits on this renderer: when a frame needs
 *   pixels only this renderer draws (the shader fallback runner), the
 *   answer names them, the transport draws them on this thread
 *   (`renderSync`), posts them on the session's port, and asks again, at
 *   most `MAX_ROUNDS` times. So a call from inside a `raster` wrapper or a
 *   WebGL callback cannot deadlock. Failures return `null` (the facades
 *   return `''` or `null` only then) and are logged with the engine's
 *   reason.
 * - **Async siblings** (`snapshot`, `renderFrameTo`, `snapshotDataUrl`,
 *   `raster`): the same over `invoke`; the UI never blocks.
 * - **UI snapshots** (`EngineThumbnails`): idle-class snapshots for
 *   thumbnails, library cards, posters, and the project thumbnail,
 *   rendered by the engine only while nothing plays or gestures; results
 *   are revision-tagged and a stale one (superseded by a newer request for
 *   the same key, or older than the revision the consumer requires) is
 *   rejected.
 * - **Compatibility pixels**: `uploadCompat` posts a layer's raster (a
 *   `raster` override's result, an extension's compatibility WebGL2
 *   context) on the session's port; `provideCompat` registers the producer
 *   the job engine's export frames are drawn by (main asks the window).
 */
import type { CompatFormat, CompatFrameRequest, CompatNeed, CompatSource, FrameQuality, RasterGeometry, ShaderFallbackRequest, SnapshotClass, SnapshotPixels, SnapshotRequest } from '../../../shared/engine-protocol';
import type { EngineBridge, EngineCompatAsk, EngineSnapshotReply, EngineSnapshotRequest } from '../../../shared/engine-ipc';
import { ticks } from './transport';

/** Rounds of `needsRenderer` a snapshot goes through before it fails. */
export const MAX_ROUNDS = 3;
/** Default waits: a blocked thread waits less than an async caller. */
export const SYNC_WAIT_MS = 5_000;
export const ASYNC_WAIT_MS = 30_000;
export const IDLE_WAIT_MS = 120_000;
/** Options of a layer raster: the composition it is in (default: the open
 *  one), the caller's layer object (JSON) when it is not the document's,
 *  and the largest side. */
export type RasterOptions = { composition?: string; content?: Uint8Array | null; maxEdge?: number };

/** Largest side of a layer raster asked for: 6,000² is under the engine's
 *  40-million-pixel snapshot cap. */
export const MAX_RASTER_EDGE = 6000;
/** How long a UI snapshot waits for the viewer's session before the
 *  connection opens a standby one. */
export const STANDBY_AFTER_MS = 5_000;
/** Largest compatibility band in one message (pm_protocol MAX_COMPAT_BAND_BYTES). */
export const MAX_COMPAT_BAND_BYTES = 8 * 1024 * 1024;

/** A snapshot to ask for. Times are composition seconds. */
export type EngineSnapshotOptions = {
  time: number;
  width: number;
  height: number;
  /** Default: the composition the window has open. */
  composition?: string;
  /** Default `opaque` (today's `renderOpaqueFrame`). */
  pixels?: SnapshotPixels;
  /** Motion blur samples for layers with their switch on (0, the default: off). */
  motionBlurSamples?: number;
  /** Default true. */
  background?: boolean;
  /** Native shading may stand in for Blender (thumbnails, posters). Default false. */
  draft3d?: boolean;
  /** How long the engine may wait for the revision and its turn (ms). */
  waitMs?: number;
  /** A layer's raster instead of the frame (`render.raster` base pixels).
   *  `content`: the layer object the caller passed, as JSON, when it is not
   *  the document's own (a wrapper's modified copy). */
  layer?: { id: string; scale: number; crop?: { x: number; y: number; width: number; height: number } | null; content?: Uint8Array | null };
};

/** A rendered snapshot. `data` is RGBA8, top-down, in `pixels`'s encoding. */
export type EngineSnapshot = {
  width: number;
  height: number;
  data: Uint8ClampedArray;
  pixels: SnapshotPixels;
  revision: number;
  viewSeq: number | null;
  origin: [number, number];
  /** A layer raster's box in layer pixels (`RasterSurface` w, h); null for frames. */
  layerBox: [number, number] | null;
  /** A layer raster's canvas, crop window, and selection; null for frames. */
  raster: RasterGeometry | null;
  quality: FrameQuality;
  /** Time spent in the engine waiting and rendering (µs), and in main (ms). */
  waitedUs: number;
  renderUs: number;
  mainMs: number;
};

/** Why a snapshot has no pixels. `code` is the engine's error code or the
 *  transport's (`noSession`, `resyncPending`, `unavailable`, `refused`, …). */
export class EngineSnapshotError extends Error {
  override name = 'EngineSnapshotError';
  constructor(readonly code: string, message: string) {
    super(message);
  }
}

/** The window's engine session, as the transport needs it (DocumentSession). */
export interface TransportSession {
  readonly id: number | null;
  readonly closed: boolean;
  readonly generation: number;
  readonly resyncPending: boolean;
  /** The revision the engine has applied (0: the document has not arrived). */
  readonly applied?: number;
  uploadFallback(pixels: { key: string; width: number; height: number; rgba16f: Uint16Array }): void;
  post(event: Record<string, unknown> & { type: string }): boolean;
}

/** What draws fallback programs (ShaderFallbackRunner). */
export interface TransportFallbackRunner {
  render(request: ShaderFallbackRequest): Promise<{ key: string; width: number; height: number; rgba16f: Uint16Array }>;
  renderSync(request: ShaderFallbackRequest): { key: string; width: number; height: number; rgba16f: Uint16Array };
}

/** Draws one layer's compatibility pixels for an export frame (the `raster`
 *  override and compatibility-context producers register one each). */
export type CompatProducer = (request: CompatFrameRequest) =>
  | { key: string; width: number; height: number; format: CompatFormat; origin: [number, number]; scale: number; data: Uint8Array }
  | null
  | Promise<{ key: string; width: number; height: number; format: CompatFormat; origin: [number, number]; scale: number; data: Uint8Array } | null>;

export type RenderTransportOptions = {
  native: Pick<EngineBridge, 'snapshotSync' | 'snapshot' | 'onCompatAsk' | 'answerCompat'>;
  /** The session snapshots are taken of (the newest viewer session). */
  session: () => TransportSession | null;
  /** The document revision the window has, after flushing pending edits to
   *  the engine (`PM.EngineDoc.barrier`). */
  revision: () => number | null;
  /** The composition the window has open. */
  composition: () => string;
  /** The open document's id (the engine's document id). */
  document: () => string;
  /** The viewer state on screen (`ViewerState.viewSeq`), if any. */
  viewSeq: () => number | null;
  fallbackRunner: () => TransportFallbackRunner | null;
  /** Opens a session to take snapshots of when none is open (the
   *  connection's standby); resolves once it has the document. */
  wake?: () => Promise<void>;
  /** The window's document for a sync snapshot its session cannot serve
   *  (none yet, the document not arrived, or a staged resync in flight):
   *  main serves it from a session of its own. `full` serializes it. */
  windowDocument?: () => { id: string; revision: number; full(): { json: Uint8Array; hash: string; revision: number } } | null;
  log: (level: 'info' | 'warn' | 'error', message: string) => void;
  createCanvas?: (width: number, height: number) => HTMLCanvasElement;
};

export type TransportStats = { sync: number; async: number; idle: number; failed: number; rounds: number; lastFailure: string | null; slowestSyncMs: number; documentSync: number };

function failure(reply: Extract<EngineSnapshotReply, { status: 'failed' }>): EngineSnapshotError {
  return new EngineSnapshotError(reply.code, reply.message);
}

export class EngineRenderTransport {
  private readonly producers = new Map<CompatSource, CompatProducer>();
  private readonly offAsk: () => void;
  readonly stats: TransportStats = { sync: 0, async: 0, idle: 0, failed: 0, rounds: 0, lastFailure: null, slowestSyncMs: 0, documentSync: 0 };
  /** UI snapshots (thumbnails, library cards, posters, the project thumbnail). */
  readonly thumbnails: EngineThumbnails;

  constructor(private readonly options: RenderTransportOptions) {
    this.offAsk = options.native.onCompatAsk?.((ask) => void this.answerAsk(ask)) ?? (() => {});
    this.thumbnails = new EngineThumbnails(this);
  }

  dispose(): void {
    this.offAsk();
    this.producers.clear();
  }

  /** Resolves once a session is open, or after `ms`. */
  private async waitForSession(ms: number): Promise<void> {
    const until = performance.now() + ms;
    while (!this.ready && performance.now() < until) await new Promise((resolve) => setTimeout(resolve, 100));
  }

  /** Whether a session is open to take snapshots of. */
  get ready(): boolean {
    const s = this.options.session();
    return !!s && s.id !== null && !s.closed;
  }

  /** The request main forwards, or why there is none. */
  private request(options: EngineSnapshotOptions, cls: SnapshotClass, uploaded: string[], detached = false): EngineSnapshotRequest {
    const session = this.options.session();
    if (!detached && (!session || session.id === null || session.closed)) {
      // A blocked thread cannot wait for one to open: start it for the next call.
      if (cls === 'sync') void this.options.wake?.();
      throw new EngineSnapshotError('noSession', 'The window has no engine session yet.');
    }
    const width = Math.round(options.width), height = Math.round(options.height);
    if (!(width >= 1 && height >= 1 && width <= 16384 && height <= 16384)) throw new EngineSnapshotError('invalidMessage', `A ${options.width}×${options.height} snapshot is outside 1..16384.`);
    const time = Number(options.time);
    if (!Number.isFinite(time)) throw new EngineSnapshotError('invalidMessage', 'A snapshot needs a finite time.');
    const wait = options.waitMs ?? (cls === 'sync' ? SYNC_WAIT_MS : cls === 'async' ? ASYNC_WAIT_MS : IDLE_WAIT_MS);
    const layer = options.layer;
    const snapshot: SnapshotRequest = {
      composition: options.composition ?? this.options.composition(),
      time: ticks(time),
      outputSize: { width, height },
      target: layer
        ? { kind: 'layer', layer: layer.id, scale: layer.scale, crop: layer.crop ?? null, content: layer.content ?? null }
        : { kind: 'frame', motionBlurSamples: Math.max(0, Math.min(256, Math.round(options.motionBlurSamples ?? 0))), background: options.background !== false, draft3d: options.draft3d === true },
      pixels: options.pixels ?? 'opaque',
      class: cls,
      viewSeq: this.options.viewSeq(),
      waitMs: Math.max(1, Math.min(120_000, Math.round(wait))),
      uploaded
    };
    return { generation: session?.generation ?? 0, session: session?.id ?? null, revision: this.options.revision(), snapshot };
  }

  private result(reply: Extract<EngineSnapshotReply, { status: 'rendered' }>): EngineSnapshot {
    const data = new Uint8ClampedArray(reply.data.buffer, reply.data.byteOffset, reply.data.byteLength);
    return { width: reply.width, height: reply.height, data, pixels: reply.pixels, revision: reply.revision, viewSeq: reply.viewSeq, origin: reply.origin, layerBox: reply.layerBox ?? null, raster: reply.raster ?? null, quality: reply.quality, waitedUs: reply.waitedUs, renderUs: reply.renderUs, mainMs: reply.mainMs };
  }

  private fail(error: unknown, where: string): EngineSnapshotError {
    const e = error instanceof EngineSnapshotError ? error : new EngineSnapshotError('internal', error instanceof Error ? error.message : String(error));
    this.stats.failed++;
    this.stats.lastFailure = `${e.code}: ${e.message}`;
    this.options.log('warn', `engine ${where} failed (${e.code}): ${e.message}`);
    return e;
  }

  /** Compatibility needs this transport cannot draw yet (the engine names
   *  none until pm-render consumes compatibility pixels). */
  private compatUnmet(compat: CompatNeed[]): EngineSnapshotError | null {
    return compat.length ? new EngineSnapshotError('notHeadlessRenderable', `The frame needs ${compat.length} compatibility raster(s) this snapshot path does not draw.`) : null;
  }

  /**
   * A snapshot the calling thread waits for. Throws `EngineSnapshotError`
   * on failure. Safe inside `raster` wrappers and WebGL callbacks: nothing
   * here waits for a task.
   */
  snapshotSync(options: EngineSnapshotOptions): EngineSnapshot {
    const started = performance.now();
    const native = this.options.native;
    if (!native.snapshotSync) throw this.fail(new EngineSnapshotError('unavailable', 'This host has no synchronous engine snapshots.'), 'snapshot');
    // This thread blocks until the answer: the window's own session serves
    // it only when it has the document and no staged resync is on its way
    // (the resync's next step would need this thread). Otherwise main serves
    // it from a session of its own, with the document sent along.
    const session = this.options.session();
    const direct = !!session && session.id !== null && !session.closed && !session.resyncPending && (session.applied ?? 1) > 0;
    if (!direct && this.options.windowDocument?.()) {
      // The window's own session (the standby) serves later ones.
      if (!session || session.closed) void this.options.wake?.();
      try {
        return this.documentSnapshotSync(options, started);
      } catch (error) {
        if (error instanceof EngineSnapshotError && error.code === 'empty') throw error;
        throw this.fail(error, 'snapshot');
      }
    }
    const uploaded: string[] = [];
    try {
      for (let round = 0; round < MAX_ROUNDS; round++) {
        const reply = native.snapshotSync(this.request(options, 'sync', uploaded));
        if (reply.status === 'rendered') {
          this.stats.sync++;
          this.stats.slowestSyncMs = Math.max(this.stats.slowestSyncMs, performance.now() - started);
          return this.result(reply);
        }
        if (reply.status === 'empty') throw new EngineSnapshotError('empty', 'The layer draws nothing at that time.');
        if (reply.status === 'failed') throw failure(reply);
        this.stats.rounds++;
        const unmet = this.compatUnmet(reply.compat);
        if (unmet) throw unmet;
        const runner = this.options.fallbackRunner();
        const session = this.options.session();
        if (!runner || !session) throw new EngineSnapshotError('unavailable', 'No shader fallback runner to draw the frame’s programs.');
        for (const request of reply.fallback) {
          if (uploaded.includes(request.key)) throw new EngineSnapshotError('internal', `The engine asked again for program ${request.key.slice(0, 12)}.`);
          session.uploadFallback(runner.renderSync(request));
          uploaded.push(request.key);
        }
      }
      throw new EngineSnapshotError('internal', `The snapshot still needed the renderer after ${MAX_ROUNDS} rounds.`);
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'empty') throw error;
      throw this.fail(error, 'snapshot');
    }
  }

  /** The last document revision main's session for this window has. */
  private sentToMain: { id: string; revision: number } | null = null;

  /** A sync snapshot main serves from its own session for this window. */
  private documentSnapshotSync(options: EngineSnapshotOptions, started: number): EngineSnapshot {
    const native = this.options.native;
    const doc = this.options.windowDocument!()!;
    const revision = this.options.revision() ?? doc.revision;
    const uploaded: string[] = [];
    let fallbackPixels: { key: string; width: number; height: number; rgba16f: Uint8Array }[] = [];
    let send = !(this.sentToMain && this.sentToMain.id === doc.id && this.sentToMain.revision === doc.revision);
    let body: { json: Uint8Array; hash: string; revision: number } | null = null;
    for (let round = 0; round < MAX_ROUNDS + 1; round++) {
      if (send && !body) body = doc.full();
      const request = this.request(options, 'sync', uploaded, true);
      const reply = native.snapshotSync!({
        ...request,
        session: null,
        revision,
        document: body && send ? { id: doc.id, revision: body.revision, hash: body.hash, json: body.json } : { id: doc.id, revision: doc.revision },
        ...(fallbackPixels.length ? { fallbackPixels } : {})
      });
      fallbackPixels = [];
      if (reply.status === 'failed' && reply.code === 'needsDocument' && !send) {
        // Main's session lost that revision (an engine restart): send it.
        send = true;
        continue;
      }
      if (send && body && reply.status !== 'failed') this.sentToMain = { id: doc.id, revision: body.revision };
      send = false;
      if (reply.status === 'rendered') {
        this.stats.sync++;
        this.stats.documentSync++;
        this.stats.slowestSyncMs = Math.max(this.stats.slowestSyncMs, performance.now() - started);
        return this.result(reply);
      }
      if (reply.status === 'empty') throw new EngineSnapshotError('empty', 'The layer draws nothing at that time.');
      if (reply.status === 'failed') throw failure(reply);
      this.stats.rounds++;
      const unmet = this.compatUnmet(reply.compat);
      if (unmet) throw unmet;
      const runner = this.options.fallbackRunner();
      if (!runner) throw new EngineSnapshotError('unavailable', 'No shader fallback runner to draw the frame’s programs.');
      for (const r of reply.fallback) {
        if (uploaded.includes(r.key)) throw new EngineSnapshotError('internal', `The engine asked again for program ${r.key.slice(0, 12)}.`);
        const pixels = runner.renderSync(r);
        fallbackPixels.push({ key: pixels.key, width: pixels.width, height: pixels.height, rgba16f: new Uint8Array(pixels.rgba16f.buffer, pixels.rgba16f.byteOffset, pixels.rgba16f.byteLength) });
        uploaded.push(r.key);
      }
    }
    throw new EngineSnapshotError('internal', `The snapshot still needed the renderer after ${MAX_ROUNDS} rounds.`);
  }

  /** The same, asynchronously (`async` or `idle` class). */
  async snapshot(options: EngineSnapshotOptions, cls: Exclude<SnapshotClass, 'sync'> = 'async'): Promise<EngineSnapshot> {
    const native = this.options.native;
    if (!native.snapshot) throw this.fail(new EngineSnapshotError('unavailable', 'This host has no engine snapshots.'), 'snapshot');
    const uploaded: string[] = [];
    try {
      if (!this.ready) {
        // UI snapshots wait for the viewer's own session first (it opens at
        // boot); a standby opens only when none comes.
        if (cls === 'idle') await this.waitForSession(STANDBY_AFTER_MS);
        if (!this.ready) await this.options.wake?.();
      }
      for (let round = 0; round < MAX_ROUNDS; round++) {
        const reply = await native.snapshot(this.request(options, cls, uploaded));
        if (reply.status === 'rendered') {
          if (cls === 'idle') this.stats.idle++;
          else this.stats.async++;
          return this.result(reply);
        }
        if (reply.status === 'empty') throw new EngineSnapshotError('empty', 'The layer draws nothing at that time.');
        if (reply.status === 'failed') throw failure(reply);
        this.stats.rounds++;
        const unmet = this.compatUnmet(reply.compat);
        if (unmet) throw unmet;
        const runner = this.options.fallbackRunner();
        const session = this.options.session();
        if (!runner || !session) throw new EngineSnapshotError('unavailable', 'No shader fallback runner to draw the frame’s programs.');
        for (const request of reply.fallback) {
          if (uploaded.includes(request.key)) throw new EngineSnapshotError('internal', `The engine asked again for program ${request.key.slice(0, 12)}.`);
          session.uploadFallback(await runner.render(request));
          uploaded.push(request.key);
        }
      }
      throw new EngineSnapshotError('internal', `The snapshot still needed the renderer after ${MAX_ROUNDS} rounds.`);
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'empty') throw error;
      throw this.fail(error, 'snapshot');
    }
  }

  /**
   * A layer's raster (the base pixels of `render.raster()`): the raster at
   * `scale`, cut to `crop` where today's raster cuts (plain text, shapes),
   * premultiplied, with its anchor (`origin`), box (`layerBox`), and
   * canvas, window, and selection (`raster`) in today's terms. Null when
   * the layer draws nothing then (a captions layer between cues). At most
   * `maxEdge` pixels a side (`MAX_RASTER_EDGE`, today's 8,192 cap).
   */
  rasterSync(layer: string, scale: number, time: number, crop: { x: number; y: number; width: number; height: number } | null = null, options: RasterOptions = {}): EngineSnapshot | null {
    try {
      return this.snapshotSync(this.rasterOptions(layer, scale, time, crop, options));
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'empty') return null;
      throw error;
    }
  }

  async raster(layer: string, scale: number, time: number, crop: { x: number; y: number; width: number; height: number } | null = null, options: RasterOptions = {}): Promise<EngineSnapshot | null> {
    try {
      return await this.snapshot(this.rasterOptions(layer, scale, time, crop, options));
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'empty') return null;
      throw error;
    }
  }

  private rasterOptions(layer: string, scale: number, time: number, crop: { x: number; y: number; width: number; height: number } | null, options: RasterOptions): EngineSnapshotOptions {
    const edge = options.maxEdge ?? MAX_RASTER_EDGE;
    return { time, width: edge, height: edge, pixels: 'premultiplied', composition: options.composition, layer: { id: layer, scale, crop, content: options.content ?? null } };
  }

  /* ── today's render API shapes (apiVersion 1 to 3 and the async siblings) ── */

  /** A snapshot as a canvas (today's capture shapes build on it). */
  toCanvas(snapshot: EngineSnapshot): HTMLCanvasElement {
    return this.canvas(snapshot);
  }

  private canvas(snapshot: EngineSnapshot): HTMLCanvasElement {
    const out = this.options.createCanvas?.(snapshot.width, snapshot.height) ?? document.createElement('canvas');
    out.width = snapshot.width;
    out.height = snapshot.height;
    // `putImageData` takes straight alpha; opaque pixels are the same either way.
    const straight = snapshot.pixels === 'premultiplied' ? unpremultiply(snapshot.data) : snapshot.data;
    out.getContext('2d')!.putImageData(new ImageData(new Uint8ClampedArray(straight), snapshot.width, snapshot.height), 0, 0);
    return out;
  }

  /** `render.renderFrameTo(time, w, h, options)`: today's `renderOpaqueFrame`
   *  (motion blur on with 16 samples unless `mblur: false`). */
  private frameOptions(time: number, width: number, height: number, options: Record<string, unknown> = {}): EngineSnapshotOptions {
    const blur = options['mblur'] !== false;
    const samples = blur ? Math.max(1, Number(options['mbSamples']) || 16) : 0;
    return { time, width, height, pixels: 'opaque', motionBlurSamples: samples, background: true };
  }

  renderFrameToSync(time: number, width: number, height: number, options?: Record<string, unknown>): HTMLCanvasElement | null {
    try {
      return this.renderFrameToCanvasSync(time, width, height, options);
    } catch {
      return null;
    }
  }

  /** The same, throwing `EngineSnapshotError` with the engine's reason. */
  renderFrameToCanvasSync(time: number, width: number, height: number, options?: Record<string, unknown>): HTMLCanvasElement {
    return this.canvas(this.snapshotSync(this.frameOptions(time, width, height, options)));
  }

  async renderFrameTo(time: number, width: number, height: number, options?: Record<string, unknown>): Promise<HTMLCanvasElement> {
    return this.canvas(await this.snapshot(this.frameOptions(time, width, height, options)));
  }

  /** `render.snapshot(time, maxWidth)`: today's JPEG data URL at quality
   *  0.74, at most `maxWidth` wide, motion blur off. `''` only on failure. */
  private snapshotOptions(time: number, maxWidth: number, size: { w: number; h: number }): EngineSnapshotOptions {
    const scale = Math.min(1, maxWidth / size.w);
    return { time, width: Math.max(2, Math.round(size.w * scale)), height: Math.max(2, Math.round(size.h * scale)), pixels: 'opaque', motionBlurSamples: 0 };
  }

  snapshotDataUrlSync(time: number, maxWidth: number, size: { w: number; h: number }): string {
    try {
      return this.canvas(this.snapshotSync(this.snapshotOptions(time, maxWidth, size))).toDataURL('image/jpeg', 0.74);
    } catch (error) {
      // Today's capture of a Blender composition with no prepared frame
      // throws (`compositor.ts`); it does not come back empty.
      if (error instanceof EngineSnapshotError && error.code === 'blenderFailed') throw new Error(error.message);
      return '';
    }
  }

  async snapshotDataUrl(time: number, maxWidth: number, size: { w: number; h: number }): Promise<string> {
    return this.canvas(await this.snapshot(this.snapshotOptions(time, maxWidth, size))).toDataURL('image/jpeg', 0.74);
  }

  /* ── compatibility pixels (the extcompat lane builds on these) ── */

  /**
   * Posts a layer's compatibility raster on the session's port, in bands
   * that fit a message. Returns false when no session is open.
   */
  uploadCompat(upload: { composition?: string; layer: string; time: number; source: CompatSource; key: string; revision?: number | null; width: number; height: number; format: CompatFormat; origin: [number, number]; scale: number; data: Uint8Array }): boolean {
    const session = this.options.session();
    if (!session || session.id === null || session.closed) return false;
    const bpp = upload.format === 'rgba16fPremultiplied' ? 8 : 4;
    if (upload.data.byteLength !== upload.width * upload.height * bpp) throw new EngineSnapshotError('invalidMessage', 'Compatibility pixels do not match their size.');
    const id = { composition: upload.composition ?? this.options.composition(), layer: upload.layer, time: ticks(upload.time), source: upload.source, key: upload.key };
    const revision = upload.revision ?? this.options.revision() ?? 0;
    const rowBytes = upload.width * bpp;
    const rowsPerBand = Math.max(1, Math.floor(MAX_COMPAT_BAND_BYTES / rowBytes));
    for (let row = 0; row < upload.height; row += rowsPerBand) {
      const rows = Math.min(rowsPerBand, upload.height - row);
      if (!session.post({ type: 'compat', compat: 'pixels', id, revision, width: upload.width, height: upload.height, format: upload.format, origin: upload.origin, scale: upload.scale, row, rows, data: upload.data.slice(row * rowBytes, (row + rows) * rowBytes) })) return false;
    }
    return true;
  }

  /** The layer's compatibility rasters are gone (override or extension disposed). */
  forgetCompat(layer: string, composition?: string): boolean {
    const session = this.options.session();
    return !!session && session.post({ type: 'compat', compat: 'forget', composition: composition ?? this.options.composition(), layer });
  }

  /** Registers the producer of one source's compatibility pixels for export
   *  frames the job engine asks for. Returns an unregister. */
  provideCompat(source: CompatSource, producer: CompatProducer): () => void {
    this.producers.set(source, producer);
    return () => {
      if (this.producers.get(source) === producer) this.producers.delete(source);
    };
  }

  private async answerAsk(ask: EngineCompatAsk): Promise<void> {
    const answer = this.options.native.answerCompat;
    if (!answer) return;
    const no = (message: string) => answer({ ask: ask.ask, status: 'unavailable', message });
    if (ask.request.document !== this.options.document()) return no('This window has another document open.');
    const producer = this.producers.get(ask.request.source);
    if (!producer) return no(`No ${ask.request.source === 'rasterOverride' ? 'raster override' : 'compatibility context'} draws layer ${ask.request.layer} in this window.`);
    try {
      const pixels = await producer(ask.request);
      if (!pixels) return no(`Layer ${ask.request.layer} has no compatibility pixels at that frame.`);
      answer({ ask: ask.ask, status: 'pixels', ...pixels });
    } catch (error) {
      no(error instanceof Error ? error.message : String(error));
    }
  }
}

/** Straight alpha from premultiplied RGBA8 (canvas `ImageData` takes straight). */
export function unpremultiply(data: Uint8ClampedArray): Uint8ClampedArray {
  const out = new Uint8ClampedArray(data.length);
  for (let i = 0; i < data.length; i += 4) {
    const a = data[i + 3]!;
    out[i + 3] = a;
    if (a === 0) continue;
    out[i] = Math.min(255, Math.round((data[i]! * 255) / a));
    out[i + 1] = Math.min(255, Math.round((data[i + 1]! * 255) / a));
    out[i + 2] = Math.min(255, Math.round((data[i + 2]! * 255) / a));
  }
  return out;
}

/** A UI snapshot request: one per key (a composition's thumbnail, a library
 *  card, a poster, the project thumbnail). */
export type ThumbnailRequest = EngineSnapshotOptions & {
  key: string;
  /** Results older than this revision are stale (default: the revision
   *  the window had when it asked). */
  minRevision?: number;
};

/**
 * Engine snapshots for the UI's consumers. Idle class: the engine renders
 * them only while nothing plays or gestures, so playback and gestures
 * never stall. Each key keeps only its newest request: an older one's
 * result is rejected (`stale`), as is a result older than the revision the
 * consumer requires.
 */
export class EngineThumbnails {
  private readonly latest = new Map<string, number>();
  private next = 0;
  stale = 0;

  constructor(private readonly transport: EngineRenderTransport) {}

  /** Resolves with the snapshot, or `null` when a newer request for the
   *  same key replaced this one or the result came back stale. Rejects
   *  with `EngineSnapshotError` on failure. */
  async request(request: ThumbnailRequest, revision: () => number | null = () => null): Promise<EngineSnapshot | null> {
    const ticket = ++this.next;
    this.latest.set(request.key, ticket);
    const required = request.minRevision ?? revision() ?? 0;
    try {
      const result = await this.transport.snapshot(request, 'idle');
      if (this.latest.get(request.key) !== ticket || result.revision < required) {
        this.stale++;
        return null;
      }
      return result;
    } finally {
      if (this.latest.get(request.key) === ticket) this.latest.delete(request.key);
    }
  }

  /** Forgets a key: its request in flight comes back `null`. */
  cancel(key: string): void {
    this.latest.delete(key);
  }
}
