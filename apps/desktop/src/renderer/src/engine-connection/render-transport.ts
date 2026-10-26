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
 *   bounded dependency phases, rejecting repeated unresolved needs. A call
 *   from inside a `raster` wrapper or a WebGL callback cannot deadlock.
 *   Failures return `null` (the facades
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
import type { CompatFormat, CompatFrameRequest, CompatNeed, CompatSource, FrameQuality, RasterGeometry, ShaderFallbackRequest, SnapshotClass, PixelRect, SnapshotPixels, SnapshotRequest, SnapshotViewer } from '../../../shared/engine-protocol';
import { OTHER_DOCUMENT_OPEN, type EngineBridge, type EngineCompatAnswer, type EngineCompatAsk, type EngineCompatEvent, type EngineSnapshotReply, type EngineSnapshotRequest } from '../../../shared/engine-ipc';
import { ticks } from './transport';
import { RenderPerfTrace } from './perf-trace';
import { SnapshotSlots } from './snapshot-slots';

/** Repetitions of the same unresolved dependency set before a snapshot fails. */
export const MAX_ROUNDS = 3;
/** The protocol admits 64 uploaded programs. Allow their
 * dependent passes plus compatibility/reuse phases and the final render. */
const MAX_DEPENDENCY_PHASES = 64 + MAX_ROUNDS + 1;
/** Default waits: a blocked thread waits less than an async caller. */
export const SYNC_WAIT_MS = 5_000;
export const ASYNC_WAIT_MS = 30_000;
export const IDLE_WAIT_MS = 120_000;
/** How long a capture that prepares its Blender frame (today's
 *  `snapshotAsync`) lets Blender run: today's 10-minute Blender timeout,
 *  plus the queue ahead of it (pm_protocol MAX_BLENDER_WAIT_MS). */
export const BLENDER_WAIT_MS = 900_000;
/** Options of a layer raster: the composition it is in (default: the open
 *  one), the caller's layer object (JSON) when it is not the document's,
 *  and the largest side. */
export type RasterOptions = { composition?: string; content?: Uint8Array | null; maxEdge?: number };

/** Today's largest layer bitmap side. Layer rasters have their own
 *  bounded 8192² admission limit; frame captures retain their 40 MP cap. */
export const MAX_RASTER_EDGE = 8192;
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
  /** Omit captions for this frame capture only. Default false. */
  captionsHidden?: boolean;
  /** Default true. */
  background?: boolean;
  /** Native shading may stand in for Blender (thumbnails, posters). Default false. */
  draft3d?: boolean;
  /** A pinned editor view, rendered before the viewer scales its canvas. */
  viewer?: SnapshotViewer;
  /** How long the engine may wait for the revision and its turn (ms). */
  waitMs?: number;
  /** Today's `snapshotAsync`: Blender renders the frame's Blender
   *  surfaces first (`prepareFrame`), then the capture draws them. Off (the
   *  default, today's synchronous capture), a Blender composition draws a
   *  frame an earlier capture prepared or fails with today's error. Async
   *  and idle frames only; ignored with `draft3d`. */
  prepareBlender?: boolean;
  /** Cancel an asynchronous source request while queued or decoding. */
  signal?: AbortSignal;
  /** A layer's raster instead of the frame (`render.raster` base pixels).
   *  `content`: the layer object the caller passed, as JSON, when it is not
   *  the document's own (a wrapper's modified copy). */
  layer?: { id: string; scale: number; crop?: { x: number; y: number; width: number; height: number } | null; content?: Uint8Array | null };
  /** Original media at source time, without composition effects or transforms. */
  source?: { asset: string; fit?: 'stretch' | 'contain' | 'cover'; color?: 'display' | 'interpreted' | 'compositor' };
};

/** A rendered snapshot. `data` is RGBA8, top-down, in `pixels`'s encoding. */
export type EngineSnapshot = {
  width: number;
  height: number;
  data: Uint8ClampedArray;
  /** Source monitor video: `data` holds these decoded planes, not RGBA. */
  videoPlanes?: import('../../../shared/engine-protocol').SourceVideoFrame & { timestamp: number };
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

/** A nested effect can reveal its next program only after its input was
 * rendered. Count actual dependency progress, rather than treating three
 * different successful source/program phases as a stuck renderer. */
class SnapshotDependencies {
  private readonly seen = new Map<string, number>();

  observe(reply: Extract<EngineSnapshotReply, { status: 'needsRenderer' }>): void {
    const key = JSON.stringify([reply.revision, reply.compat, reply.fallback.map((r) => r.key)]);
    const count = (this.seen.get(key) ?? 0) + 1;
    this.seen.set(key, count);
    if (count >= MAX_ROUNDS) throw new EngineSnapshotError('internal', `The engine repeated the same unresolved snapshot dependencies ${MAX_ROUNDS} times.`);
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
  uploadFallback(pixels: { key: string; width: number; height: number; rgba16f: Uint16Array; rgba32f?: Float32Array }): void;
  post(event: Record<string, unknown> & { type: string }): boolean;
}

/** What draws fallback programs (ShaderFallbackRunner). `textures` are an
 *  effect's or a transition's (binary16 RGBA, GL row order). */
export interface TransportFallbackRunner {
  render(request: ShaderFallbackRequest, textures?: (Uint16Array | Float32Array)[]): Promise<{ key: string; width: number; height: number; rgba16f: Uint16Array; rgba32f?: Float32Array }>;
  renderSync(request: ShaderFallbackRequest, textures?: (Uint16Array | Float32Array)[]): { key: string; width: number; height: number; rgba16f: Uint16Array; rgba32f?: Float32Array };
}

/** The textures a fallback request samples, from the answer that names it
 *  (main hands them back with `needsRenderer`; main/engine/fallback-inputs.ts). */
export function fallbackTextures(reply: { fallbackTextures?: { key: string; textures: Uint8Array[] }[] }, request: ShaderFallbackRequest): (Uint16Array | Float32Array)[] {
  const count = request.textures ?? 0;
  if (count === 0) return [];
  const found = reply.fallbackTextures?.find((t) => t.key === request.key);
  const stride = request.colorDepth === 32 ? 16 : 8;
  const size = request.width * request.height * stride;
  if (!found || found.textures.length !== count || found.textures.some((t) => t.byteLength !== size)) {
    throw new EngineSnapshotError('internal', `The textures of program ${request.key.slice(0, 12)} did not arrive with the answer.`);
  }
  return found.textures.map((t) => {
    // Aligned copies: an IPC buffer may start at an odd offset.
    const values = request.colorDepth === 32 ? new Float32Array(t.byteLength / 4) : new Uint16Array(t.byteLength / 2);
    new Uint8Array(values.buffer).set(t);
    return values;
  });
}

/** Draws one layer's compatibility pixels for an export frame (the `raster`
 *  override and compatibility-context producers register one each). */
type CompatProduced = { key: string; width: number; height: number; format: CompatFormat; origin: [number, number]; scale: number; data: Uint8Array };
/** An export frame's compatibility pixels, `'native'` when the producer
 *  leaves it to the engine (a compatibility context nobody drew into), or
 *  null when it has none. */
export type CompatProducer = (request: CompatFrameRequest) =>
  | CompatProduced
  | 'native'
  | null
  | Promise<CompatProduced | 'native' | null>;

/** The WebM formats (today's `exportWebCodecs`/`exportRecorder` read their
 *  frames from the compositor's canvas after `frame:rendered`). */
export function exportDrawsOverFrames(format: unknown): boolean {
  return format === 'webm' || format === 'webm-vp9' || format === 'webm-vp8' || format === 'rec';
}

/**
 * A producer's answer to one of the engine's compatibility needs (a layer at
 * a frame, at the density today's compositor rasterizes it): its raster on
 * the quad it describes, a pass-through, a raster already uploaded under
 * `key`, or no answer (the producer cannot decide: the frame stays waiting
 * and asks again).
 */
export type CompatAnswerResult =
  | { kind: 'native' }
  | { kind: 'reuse'; key: string }
  | {
      kind: 'raster'; key: string; width: number; height: number; format: CompatFormat; data: Uint8Array;
      /** Today's `anchorX`, `anchorY` (layer pixels). */
      anchor: [number, number];
      /** Today's `w`, `h` (layer pixels). */
      size: [number, number];
      uv?: [number, number, number, number];
    }
  | null;

/** Answers the engine's needs for one source (the raster override's producer). */
/** Frames of compatibility answers kept ahead of the clock during playback. */
export const COMPAT_LOOKAHEAD = 18;
/** A bounded paused prefill; never an ongoing paused timer. A changing
 *  base needs enough turns to fill the pipeline, including worker startup. */
export const COMPAT_WARM_MS = 500;
/** Main-thread time one lookahead call may take (ms); the rest waits for the next frame. */
const LOOKAHEAD_BUDGET_MS = 6;
const LOOKAHEAD_MAX_ANSWERS = 16;
const LOOKAHEAD_POLL_MS = 4;
/** Base-raster preparations for the answer horizon in flight at once. */
export const COMPAT_PREPARE_IN_FLIGHT = 2;
/** Frames of asynchronously prepared compatibility work ahead of the clock.
 *  The producer resolves every base dependency before retaining its result;
 *  only the shorter answer horizon is uploaded to the engine. */
export const COMPAT_PREFETCH = 24;
/** Speculative base preparations in flight at once (beyond the answer horizon's). */
export const COMPAT_PREFETCH_IN_FLIGHT = 1;

/** The playable span the clock wraps by while it loops, in ticks, `[start, end)`. */
export type PlaybackLoop = { start: number; end: number };

const modulo = (value: number, length: number) => ((value % length) + length) % length;

/**
 * How far ahead of `cursor` playback reaches `time`, running by `step`
 * (a signed frame), in ticks. In a loop the clock wraps: `time` is ahead
 * unless it is the frame just shown, so the result is in
 * `[-|step|, length - |step|)` and the frames after the wrap follow the
 * last ones before it.
 */
export function runAhead(cursor: number, time: number, step: number, loop: PlaybackLoop | null): number {
  const distance = Math.sign(step) * (time - cursor);
  const length = loop ? loop.end - loop.start : 0;
  if (!loop || !(length > 0)) return distance;
  const frame = Math.abs(step);
  return modulo(distance + frame, length) - frame;
}

/** `time` brought into the loop the clock wraps by (unchanged without one). */
export function intoLoop(time: number, loop: PlaybackLoop | null): number {
  if (!loop || !(loop.end > loop.start) || (time >= loop.start && time < loop.end)) return time;
  return loop.start + modulo(time - loop.start, loop.end - loop.start);
}

/** `loop`: the clock's loop in the entry's own time (a nested composition's
 *  times are the open one's shifted), while playback loops and the shift
 *  is known. */
type AheadEntry = { need: CompatNeed; session: TransportSession; step: number; cursor: number; posted: number; sent: Set<number>; pending: Set<number>; required: Set<number>; prefetched: Set<number>; revision: number | null; clockCursor: number | null; loop?: PlaybackLoop | null;
  /** Ends the waiting turns of a non-urgent preparation in flight, per time. */
  promote?: Map<number, () => void> };

/** `urgent`: the paused frame the engine waits for (no yielded turns);
 *  `promoted`: settles when a preparation that began speculatively became
 *  that frame's, so it goes on as urgent; `due`: a frame the running clock
 *  shows within the answer horizon (its turns go ahead of ordinary tasks). */
export type CompatPrepareOptions = { urgent?: boolean; promoted?: Promise<void>; due?: boolean };
export type CompatPreparer = (need: CompatNeed, revision: number, options?: CompatPrepareOptions) => Promise<void>;

export type CompatAnswerer = (need: CompatNeed, revision: number, options?: CompatAnswerOptions) => CompatAnswerResult;

/** `fresh`: never `reuse` (an export frame, or a need asked again after a
 *  reuse); `layer`: the layer as an export's snapshot holds it (JSON);
 *  `prepared`: an interactive answer with its exact base pixels ready. */
export type CompatAnswerOptions = { fresh?: boolean; layer?: string; prepared?: boolean };

export type RenderTransportOptions = {
  /** CPU hidden tabs use host-owned document sessions for every snapshot. */
  documentOnly?: boolean;
  native: Pick<EngineBridge, 'snapshotSync' | 'snapshot' | 'cancelSnapshot' | 'onCompatAsk' | 'answerCompat'>;
  /** The session snapshots are taken of (the newest viewer session). */
  session: () => TransportSession | null;
  /** A producer threw while answering a frame's need (today's failed frame). */
  onProducerError?: (error: unknown, need: CompatNeed) => void;
  /** Named snapshot failures also reach the connection status (never the viewer's stage: the snapshot is its caller's). */
  onFailure?: (code: string, message: string) => void;
  /** Whether the editor plays (compatibility answers then run ahead of the clock). */
  playing?: () => boolean;
  /** The open composition's frame duration in ticks (the first lookahead step). */
  frameTicks?: () => number | null;
  /** Audio-clock playhead in ticks, independent of frames reaching the viewer. */
  playhead?: () => { time: number; rate: number } | null;
  /** The open composition's playable span in ticks, `[start, end)`: the
   *  loop the clock wraps by, else the composition. Playback never shows a
   *  frame outside it, so the lookahead prepares none there. `loop`: the
   *  clock wraps by it (today's looping playback, the default): the frames
   *  after the wrap are prepared before it, as the frames ahead. */
  span?: () => { start: number; end: number; loop?: boolean } | null;
  /** Every open session of the window (viewer sessions and the standby):
   *  each learns which compatibility producers the window has. */
  sessions?: () => TransportSession[];
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
  windowDocument?: () => { id: string; revision: number; nativeKernel?: true; full(): { json: Uint8Array; hash: string; revision: number } } | null;
  /** Whether the window's document has a composition Blender renders
   *  (`render3d.enabled`): only then does a capture that prepares its
   *  Blender frame ask the engine (and main) to wait for Blender. */
  usesBlender?: () => boolean;
  /** Current PM.captionsHidden for frame captures without an explicit option. */
  captionsHidden?: () => boolean;
  log: (level: 'info' | 'warn' | 'error', message: string) => void;
  createCanvas?: (width: number, height: number) => HTMLCanvasElement;
};

export type TransportStats = { sync: number; async: number; idle: number; failed: number; rounds: number; lastFailure: string | null; slowestSyncMs: number; documentSync: number };

function pinViewer(options: EngineSnapshotOptions): EngineSnapshotOptions {
  return options.viewer ? { ...options, viewer: structuredClone(options.viewer) } : options;
}

function failure(reply: Extract<EngineSnapshotReply, { status: 'failed' }>): EngineSnapshotError {
  return new EngineSnapshotError(reply.code, reply.message);
}

/** The scale a layer raster asks the engine for (`render.raster(layer,
 *  scale)` at apiVersion 1 to 3). Today's `PM.raster` takes any scale and
 *  caps only its density, at `min(scale, 8192 / w, 8192 / h)` (legacy
 *  gl/raster.ts `textRasterGeometry`, `shapeRasterGeometry`, `rasterPaths`,
 *  the captions rasterizer), and the engine's rasterizers apply the same
 *  cap. So a positive scale goes as it is, up to the largest safe integer:
 *  every number above it is an integer the wire's MessagePack encoder
 *  refuses, and any raster box wider than 1e-12 pixels reaches the 8192
 *  cap well below it, so Infinity and 1e300 give today's density there.
 *  Zero, a negative scale, or NaN, whose raster today is a canvas of at
 *  most one pixel with nothing on it (captions: density 0.05), go as a
 *  vanishing density that gives a one-pixel transparent canvas with the
 *  same geometry. */
export function layerRasterScale(scale: number): number {
  if (scale > Number.MAX_SAFE_INTEGER) return Number.MAX_SAFE_INTEGER;
  return scale > 0 ? scale : VANISHING_RASTER_SCALE;
}

/** A density at which even a layer 100,000 pixels wide covers a
 *  ten-thousandth of a pixel: a one-pixel canvas with no visible ink. */
export const VANISHING_RASTER_SCALE = 1e-9;

/** Answers {@link EngineRenderTransport.answeredAt} remembers. */
export const ANSWER_LOG_LIMIT = 4096;

export class EngineRenderTransport {
  readonly trace = new RenderPerfTrace();
  private readonly producers = new Map<CompatSource, CompatProducer>();
  /** Producers of viewer frames' and snapshots' compatibility answers. */
  private readonly answerers = new Map<CompatSource, CompatAnswerer>();
  private readonly preparers = new Map<CompatSource, CompatPreparer>();
  private aheadInFlight = 0;
  /** Of those, the ones the paused frame on screen waits for. */
  private aheadUrgentInFlight = 0;
  private prefetchInFlight = 0;
  /** Needs answered with `reuse`, so a repeated ask gets pixels instead
   *  (the engine dropped that raster meanwhile). */
  private readonly reused = new Set<string>();
  readonly compatStats = { needs: 0, rasters: 0, natives: 0, reuses: 0, unanswered: 0, exportAsks: 0 };
  /** The latest answer posted for each layer and time (`layer`, `time` in
   *  ticks), with its revision and kind, most recent last; at most
   *  {@link ANSWER_LOG_LIMIT}. Counts answers by the frame they are for,
   *  wherever they were made (a need, the lookahead, the paused warm-up). */
  private readonly answerLog = new Map<string, { revision: number; kind: 'raster' | 'reuse' | 'native'; key?: string }>();
  /** What was last posted for each exact need (layer, time, source, density
   *  and window): `native`, or `raster:` and its key. A paused redraw tells
   *  a changed output by it. */
  private readonly postedFor = new Map<string, string>();
  private answering = 0;
  /** Clock-driven, bounded playback lookahead. Answers are keyed by the
   * layer's exact need (including scale/crop) and document revision. */
  private readonly ahead = new Map<string, AheadEntry>();
  private pausedEditRevision: number | null = null;
  private aheadTimer: ReturnType<typeof setTimeout> | null = null;
  private aheadTurn = 0;
  private pumpingAhead = false;
  /** How long one asynchronous preparation takes, on average (ms). */
  private prepareMs: number | null = null;
  readonly lookaheadStats = { prepareMs: 0, latePrepareSkips: 0, prefetched: 0, maxPrefetchInFlight: 0, answered: 0, asked: 0, askedAhead: 0, skipped: 0, recoveries: 0, pending: 0, maxPending: 0, inFlight: 0, maxInFlight: 0, obsolete: 0, slowestAnswerMs: 0, answerMs: 0, recentAnswers: [] as { time: number; ms: number }[], recent: [] as number[] };
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
    if (this.aheadTimer !== null) clearTimeout(this.aheadTimer);
    this.aheadTimer = null;
    this.ahead.clear();
    this.pausedEditRevision = null;
    this.producers.clear();
    this.answerers.clear();
    this.preparers.clear();
  }

  /* ── compatibility answers (the `raster` override; pm_render::compat) ── */

  /** The sources this window produces (an export names them so its job
   *  asks the window for those layers). */
  get producing(): CompatSource[] {
    return [...this.answerers.keys()];
  }

  /** The sources an export in `format` names: the layer producers, and the
   *  compatibility context for the WebM formats while it draws
   *  ([`exportDrawsOverFrames`]). */
  exportSources(format: unknown): CompatSource[] {
    const sources = this.producing;
    if (exportDrawsOverFrames(format) && this.producers.has('compatContext') && this.contextActive?.()) sources.push('compatContext');
    return sources;
  }

  /** Whether an extension holds the compatibility context (the kernel sets it). */
  contextActive: (() => boolean) | null = null;

  /**
   * Registers the producer that answers the engine's needs for `source`
   * (viewer frames ask over the session's port, snapshots in their
   * `needsRenderer` rounds). The session learns which producers it has; the
   * engine asks none without one. Returns an unregister.
   */
  answerCompat(source: CompatSource, answerer: CompatAnswerer, prepare?: CompatPreparer): () => void {
    for (const [key, entry] of this.ahead) if (entry.need.source === source) this.ahead.delete(key);
    this.answerers.set(source, answerer);
    if (prepare) this.preparers.set(source, prepare); else this.preparers.delete(source);
    this.syncProducers();
    return () => {
      if (this.answerers.get(source) !== answerer) return;
      this.answerers.delete(source);
      this.preparers.delete(source);
      for (const [key, entry] of this.ahead) if (entry.need.source === source) this.ahead.delete(key);
      this.syncProducers();
    };
  }

  /** Tells `session` (default: every open session) which producers this window has. */
  syncProducers(session?: TransportSession | null): void {
    const targets = session !== undefined ? [session] : (this.options.sessions?.() ?? [this.options.session()]);
    for (const s of new Set(targets)) {
      if (!s || s.id === null || s.closed) continue;
      s.post({ type: 'compat', compat: 'producers', sources: [...this.answerers.keys()] });
    }
  }

  /** The engine asked `session` for answers (`CompatMessage` `needs`). */
  receiveCompat(message: Record<string, unknown>, session: TransportSession | null = this.options.session()): void {
    if (message['compat'] !== 'needs' || !session) return;
    const needs = Array.isArray(message['needs']) ? message['needs'] as CompatNeed[] : [];
    const playing = !!this.options.playing?.();
    const clock = playing ? this.options.playhead?.() : null;
    const frame = this.options.frameTicks?.() ?? 0;
    for (const need of needs) {
      // A delayed request is no longer the frame the engine can present.
      // Answer the clock's frame instead of filling an obsolete FIFO.
      // In a loop a frame after the wrap is ahead of the clock's last ones.
      const loop = this.playbackLoop();
      const stale = !!clock && need.composition === this.options.composition() && frame > 0
        && (loop && loop.end - loop.start > (COMPAT_PREFETCH + 1) * frame
          ? runAhead(clock.time, need.time, Math.sign(clock.rate || 1) * frame, loop) > COMPAT_PREFETCH * frame
          : !loop && Math.sign(clock.rate) * (clock.time - need.time) > frame);
      // Compositor needs stay asynchronous across Play/Stop boundaries too.
      // The engine's stopped clock can arrive before its final frame is
      // presented; answering that need synchronously would block playback.
      const asynchronous = this.preparers.has(need.source);
      const answered = !stale && !asynchronous && this.answerNeed(need, session);
      if (stale) this.lookaheadStats.skipped++;
      if (playing) {
        this.lookaheadStats.asked++;
        const entry = this.ahead.get(this.aheadTag(need));
        if (entry?.sent.has(need.time)) this.lookaheadStats.askedAhead++;
        this.lookaheadStats.recent.push(need.time);
        if (this.lookaheadStats.recent.length > 12) this.lookaheadStats.recent.shift();
      }
      this.lookahead(need, session, answered);
      if (asynchronous && !stale) {
        const entry = this.ahead.get(this.aheadTag(need));
        entry?.sent.delete(need.time);
        if (!playing) entry?.required.add(need.time);
      }
    }
    if (playing || this.preparers.size) this.presentedWhilePlaying(!playing);
  }

  /** Clock-driven replenishment also runs when no frame was presented. */
  presentedWhilePlaying(warm = false, warmUntil = performance.now() + COMPAT_WARM_MS): void {
    if (warm && !this.options.playing?.() && performance.now() >= warmUntil
      && ![...this.ahead.values()].some(entry => entry.required.size > 0)) return;
    if ((!this.options.playing?.() && !warm) || this.pumpingAhead) return;
    this.pumpingAhead = true;
    try {
      const deadline = performance.now() + LOOKAHEAD_BUDGET_MS;
      const playhead = this.options.playhead?.();
      const clock = this.options.playing?.() ? playhead : null;
      const revision = this.options.revision();
      const span = this.options.span?.() ?? null;
      const loop = this.playbackLoop(span);
      const entries = [...this.ahead.values()].filter(entry => {
        if (entry.session.closed || !this.answerers.has(entry.need.source)) {
          this.ahead.delete(this.aheadTag(entry.need));
          return false;
        }
        const step = (this.options.frameTicks?.() ?? Math.abs(entry.step)) * (playhead && playhead.rate < 0 ? -1 : 1);
        const clockCursor = clock ? Math.max(0, Math.floor(clock.time / Math.abs(step)) * Math.abs(step)) : null;
        const cursor = clockCursor === null ? entry.cursor : entry.need.composition === this.options.composition()
          ? clockCursor : entry.cursor + (entry.clockCursor === null ? 0 : clockCursor - entry.clockCursor);
        entry.clockCursor = clockCursor;
        // The loop in this entry's time: the open composition's own, or a
        // nested one's shifted as the clock moves it (playing only).
        const open = entry.need.composition === this.options.composition();
        const shift = open ? 0 : clockCursor === null ? null : cursor - clockCursor;
        const entryLoop = loop && shift !== null ? { start: loop.start + shift, end: loop.end + shift } : null;
        // A wrap is the run moving on, not a seek: only a move back (or
        // further than the horizon at once) starts the windows over.
        const moved = runAhead(entry.cursor, cursor, step, entryLoop);
        if (revision !== entry.revision || Math.sign(step) !== Math.sign(entry.step)
          || moved < 0 || (entryLoop && moved > COMPAT_PREFETCH * Math.abs(step))) { entry.sent.clear(); entry.prefetched.clear(); }
        if (runAhead(entry.posted, cursor, step, entryLoop) > Math.abs(step)) this.lookaheadStats.recoveries++;
        entry.revision = revision;
        entry.step = step;
        entry.cursor = cursor;
        entry.loop = entryLoop;
        for (const time of entry.sent) {
          const distance = runAhead(cursor, time, step, entryLoop);
          if (distance < -Math.abs(step) || distance > COMPAT_LOOKAHEAD * Math.abs(step)) entry.sent.delete(time);
        }
        for (const time of entry.prefetched) {
          const distance = runAhead(cursor, time, step, entryLoop);
          if (distance < -Math.abs(step) || distance > COMPAT_PREFETCH * Math.abs(step)) entry.prefetched.delete(time);
        }
        for (const time of entry.required) {
          const distance = runAhead(cursor, time, step, entryLoop);
          if (distance < -Math.abs(step) || distance > COMPAT_LOOKAHEAD * Math.abs(step)) entry.required.delete(time);
        }
        return step !== 0;
      });
      // Breadth first: every layer's current frame precedes speculative
      // work. A slow producer gets at most one turn before another layer.
      // A preparation started for a frame the clock reaches before it can
      // finish is wasted, and when every one is, playback never catches up
      // (it would keep preparing the frame at the clock). Frames sooner than
      // a preparation takes are left to the engine's own asks; the horizon
      // starts where answers can still arrive in time.
      const frameMs = ((this.options.frameTicks?.() ?? 0) / ticks(1)) * 1000;
      // Only while the line moves: a Play held in the engine's preroll
      // (rate 0) waits for exactly the frames at the clock.
      const lead = clock && clock.rate !== 0 && this.prepareMs !== null && frameMs > 0
        ? Math.min(COMPAT_LOOKAHEAD, Math.ceil((this.prepareMs * Math.abs(clock.rate)) / frameMs)) : 0;
      let done = 0, pending = 0;
      const outside = (entry: AheadEntry, time: number) => time < 0 || (!!span && entry.need.composition === this.options.composition() && (time < span.start || time >= span.end));
      // An edit needs its current pixels first. Preparing an entire paused
      // playback horizon at every revision competes with the next pointer
      // move, and those future answers are invalidated by that move anyway.
      const horizon = !this.options.playing?.() && revision === this.pausedEditRevision ? 0 : COMPAT_LOOKAHEAD;
      for (let offset = 0; offset <= horizon; offset++) {
        for (let i = 0; i < entries.length; i++) {
          const entry = entries[(i + this.aheadTurn) % entries.length]!;
          // Past the loop's end, the frames after the wrap.
          const time = intoLoop(entry.cursor + offset * entry.step, entry.loop ?? null);
          if (outside(entry, time) || entry.sent.has(time)) continue;
          if (entry.pending.has(time)) {
            // The paused frame on screen waits for a preparation that began
            // speculatively: it goes on without yielding.
            if (this.urgentAhead(entry, time)) entry.promote?.get(time)?.();
            continue;
          }
          if (warm && performance.now() >= warmUntil && !entry.required.has(time)) continue;
          if (offset < lead && this.preparers.has(entry.need.source)) { this.lookaheadStats.latePrepareSkips++; continue; }
          pending++;
          if (done >= LOOKAHEAD_MAX_ANSWERS || (done > 0 && performance.now() >= deadline)) continue;
          const prepare = this.preparers.get(entry.need.source);
          // The paused frame on screen is never queued behind speculative
          // preparations: urgent ones have slots of their own.
          if (prepare && (this.urgentAhead(entry, time) ? this.aheadUrgentInFlight : this.aheadInFlight) >= COMPAT_PREPARE_IN_FLIGHT) continue;
          done++;
          if (prepare) {
            this.prepareAhead(entry, time, revision ?? 0, prepare, warm, warmUntil);
            pending--;
            continue;
          }
          if (this.answerNeed({ ...entry.need, time }, entry.session)) {
            entry.sent.add(time);
            entry.posted = time;
            this.lookaheadStats.answered++;
            pending--;
          }
        }
      }
      // Speculative preparation past the answer horizon, never ahead of it:
      // only while no layer's nearer frame is still waiting to start.
      if (pending === 0 && clock) {
        for (let offset = COMPAT_LOOKAHEAD + 1; offset <= COMPAT_PREFETCH && this.prefetchInFlight < COMPAT_PREFETCH_IN_FLIGHT; offset++) {
          for (const entry of entries) {
            if (this.prefetchInFlight >= COMPAT_PREFETCH_IN_FLIGHT) break;
            const prepare = this.preparers.get(entry.need.source);
            const time = intoLoop(entry.cursor + offset * entry.step, entry.loop ?? null);
            if (!prepare || outside(entry, time) || entry.prefetched.has(time)) continue;
            entry.prefetched.add(time);
            this.prefetchAhead(entry.need, time, revision ?? 0, prepare);
          }
        }
      }
      this.aheadTurn++;
      this.lookaheadStats.pending = pending;
      this.lookaheadStats.maxPending = Math.max(this.lookaheadStats.maxPending, pending);
    } finally { this.pumpingAhead = false; }
    this.scheduleAhead();
  }

  playbackStarted(): void {
    this.presentedWhilePlaying();
  }

  /** Prepares one frame's compatibility work ahead; the answer horizon reaching
   *  that frame finds them ready (raster-compat `prepare`). A failure is
   *  reported when that frame's own preparation repeats it. */
  private prefetchAhead(need: CompatNeed, time: number, revision: number, prepare: CompatPreparer): void {
    this.prefetchInFlight++;
    this.lookaheadStats.prefetched++;
    this.lookaheadStats.maxPrefetchInFlight = Math.max(this.lookaheadStats.maxPrefetchInFlight, this.prefetchInFlight);
    void Promise.resolve().then(() => prepare({ ...need, time }, revision)).catch(() => {}).finally(() => {
      this.prefetchInFlight--;
      if (this.options.playing?.()) this.presentedWhilePlaying();
    });
  }

  private prepareAhead(entry: AheadEntry, time: number, revision: number, prepare: CompatPreparer, warm = false, warmUntil = performance.now() + COMPAT_WARM_MS): void {
    const need = { ...entry.need, time };
    const urgent = this.urgentAhead(entry, time);
    let promote!: () => void;
    const promoted = new Promise<void>(resolve => { promote = resolve; });
    if (!urgent) (entry.promote ??= new Map()).set(time, promote);
    entry.pending.add(time);
    this.aheadInFlight++;
    if (urgent) this.aheadUrgentInFlight++;
    this.lookaheadStats.inFlight = this.aheadInFlight;
    this.lookaheadStats.maxInFlight = Math.max(this.lookaheadStats.maxInFlight, this.aheadInFlight);
    const began = performance.now();
    this.trace.record('prepare dispatched', revision, time);
    const due = !!this.options.playing?.();
    void Promise.resolve().then(() => prepare(need, revision, urgent ? { urgent } : { urgent, promoted, ...(due ? { due } : {}) })).then(() => {
      this.trace.record('prepare ready', revision, time, began);
      const ms = performance.now() - began;
      this.prepareMs = this.prepareMs === null ? ms : this.prepareMs * 0.75 + ms * 0.25;
      this.lookaheadStats.prepareMs = Math.round(this.prepareMs * 10) / 10;
      const clock = this.options.playing?.() ? this.options.playhead?.() : null;
      const cursor = clock && need.composition === this.options.composition()
        ? Math.floor(clock.time / Math.abs(entry.step)) * Math.abs(entry.step) : entry.cursor;
      const distance = runAhead(cursor, time, entry.step, entry.loop ?? null);
      if ((!this.options.playing?.() && !entry.required.has(time) && (!warm || performance.now() >= warmUntil)) || entry.session.closed || this.ahead.get(this.aheadTag(need)) !== entry
        || this.preparers.get(need.source) !== prepare || revision !== this.options.revision() || distance < -Math.abs(entry.step) || distance > COMPAT_LOOKAHEAD * Math.abs(entry.step)) {
        this.lookaheadStats.obsolete++;
        return;
      }
      const answered = this.answerNeed(need, entry.session, true);
      entry.sent.add(time);
      if (answered) {
        entry.posted = time;
        this.lookaheadStats.answered++;
      }
    }).catch(error => {
      // Retry only after a fresh native need or clock position, never a
      // self-sustaining chain of immediately rejected microtasks.
      entry.sent.add(time);
      if (this.options.onProducerError) this.options.onProducerError(error, need);
      else this.options.log('warn', `Preparing a compatibility raster failed: ${String(error)}`);
    }).finally(() => {
      entry.pending.delete(time);
      entry.required.delete(time);
      if (entry.promote?.get(time) === promote) entry.promote.delete(time);
      this.aheadInFlight--;
      if (urgent) this.aheadUrgentInFlight--;
      this.lookaheadStats.inFlight = this.aheadInFlight;
      // Each completed answer is independent: a slower sibling does not
      // hold its ready pixels behind a Promise.all barrier.
      this.presentedWhilePlaying(warm || [...this.ahead.values()].some(e => e.required.size > 0), warmUntil);
    });
  }

  /** Whether `time` is the paused frame the engine waits for (its own need, or the cursor's). */
  private urgentAhead(entry: AheadEntry, time: number): boolean {
    return !this.options.playing?.() && (time === entry.cursor || entry.required.has(time));
  }

  private scheduleAhead(): void {
    // A timer, rather than a presented-frame callback, breaks the deadlock
    // when one late answer consumed the entire presentation horizon.
    if (!this.options.playing?.() || !this.options.playhead || this.aheadTimer !== null || !this.ahead.size) return;
    this.aheadTimer = setTimeout(() => {
      this.aheadTimer = null;
      this.presentedWhilePlaying();
    }, LOOKAHEAD_POLL_MS);
  }

  /** An exact need's identity (layer, time, source, density, window). */
  private needTag(need: CompatNeed): string {
    return `${need.composition}\u0000${need.layer}\u0000${need.time}\u0000${need.source}\u0000${need.scale}\u0000${need.crop ? `${need.crop.x},${need.crop.y},${need.crop.width},${need.crop.height}` : ''}`;
  }

  /** The loop the clock wraps by while playback loops (`span.loop`). */
  private playbackLoop(span = this.options.span?.() ?? null): PlaybackLoop | null {
    return span?.loop && span.end > span.start ? { start: span.start, end: span.end } : null;
  }

  private aheadTag(need: CompatNeed): string {
    return `${need.composition}\u0000${need.layer}\u0000${need.source}`;
  }

  private lookahead(need: CompatNeed, session: TransportSession, answered: boolean): void {
    const tag = this.aheadTag(need);
    const frame = this.options.frameTicks?.() ?? 0;
    if (!frame) return;
    const previous = this.ahead.get(tag);
    const same = previous?.session === session && previous.need.scale === need.scale
      && JSON.stringify(previous.need.crop) === JSON.stringify(need.crop);
    const clock = this.options.playhead?.();
    const step = frame * (clock && clock.rate < 0 ? -1 : 1);
    const entry: AheadEntry = same ? previous : {
      need, session, step, cursor: need.time, posted: need.time,
      sent: new Set<number>(), pending: new Set<number>(), required: new Set<number>(), prefetched: new Set<number>(), revision: this.options.revision(), clockCursor: null
    };
    entry.need = need;
    // Never infer reverse playback from a late need. Only the transport's
    // rate determines direction; an explicit seek resets the sent window.
    if (!clock || !this.options.playing?.() || need.composition !== this.options.composition()) {
      entry.cursor = need.time;
      entry.clockCursor = clock ? Math.floor(clock.time / frame) * frame : null;
    }
    if (answered) entry.sent.add(need.time);
    this.ahead.set(tag, entry);
    if (this.ahead.size > 256) this.ahead.delete(this.ahead.keys().next().value!);
  }

  /** Draws one need with its producer and posts the answer; false when there was none. */
  /**
   * A paused redraw: today's frame loop calls every raster wrapper again on
   * `draw`, so a wrapper whose output follows its own state (a panel
   * setting, then `transport.invalidate()`) is drawn anew without an edit.
   * The producers answer the frame on screen again (`pts`: its media time;
   * nested compositions' layers at their last asked time). A wrapper that
   * returns its posted key answers `reuse`. Returns whether an answer
   * changed: the engine then draws and presents the frame (an answer that
   * replaces another at the same revision redraws it); otherwise the
   * caller announces the frame on screen again.
   */
  refreshShown(pts: number): boolean {
    if (this.options.playing?.()) return false;
    let changed = false;
    for (const entry of [...this.ahead.values()]) {
      if (entry.session.closed || !this.answerers.has(entry.need.source)) continue;
      // The engine's need for the frame on screen: its exact time (a layer's
      // time can differ from the frame's by a tick of rounding), else `pts`.
      const frame = this.options.frameTicks?.() ?? 0;
      const time = entry.need.composition !== this.options.composition() || Math.abs(entry.cursor - pts) < Math.max(1, frame) / 2
        ? entry.cursor : pts;
      const need = { ...entry.need, time };
      const was = this.postedFor.get(this.needTag(need)) ?? null;
      // The same pixels (a reuse, or a raster under the key already posted)
      // or a pass-through again: unchanged. A changed output makes every
      // answer for that layer stale (other frames, densities, windows: a
      // capture asks for them again): the layer is forgotten first, so the
      // engine draws the frame once, with the new answer. Without an earlier
      // answer posted here the engine decides alone (an answer that replaces
      // its own redraws the frame).
      this.answerNeed(need, entry.session, false, true, (now) => {
        if (was === null || now === was) return;
        changed = true;
        this.forgetCompat(String(need.layer), String(need.composition));
      });
    }
    return changed;
  }

  /** An edit while paused: prepares the frames around the playhead at the
   *  new revision at once (the paused warm-up the engine's ask would start). */
  prepareShown(): void {
    if (this.options.playing?.() || !this.ahead.size || !this.preparers.size) return;
    this.pausedEditRevision = this.options.revision();
    this.presentedWhilePlaying(true);
  }

  private answerNeed(need: CompatNeed, session: TransportSession, prepared = false, refresh = false, beforePost?: (answer: string) => void): boolean {
    const answerer = this.answerers.get(need.source);
    // A producer that asks for a frame from inside its own answer (an
    // override calling `render.snapshot`) gets none: that frame would need
    // the answer being made.
    if (!answerer || this.answering > 0) return false;
    this.compatStats.needs++;
    const began = performance.now();
    const revision = this.options.revision() ?? 0;
    const crop = need.crop ? { crop: { ...need.crop } } : {};
    const tag = this.needTag(need);
    // Asked again after a `reuse` at this revision: the engine no longer
    // holds that raster (evicted), so this answer carries the pixels.
    // A refresh asks the producer whether its output changed: never fresh.
    const fresh = !refresh && this.reused.has(`${tag}\u0000${revision}`);
    let answer: CompatAnswerResult;
    this.answering++;
    try {
      if (!(Number.isFinite(need.scale) && need.scale > 0 && need.scale <= 32)) throw new EngineSnapshotError('invalidMessage', `Compatibility raster scale ${need.scale} for layer ${need.layer} is outside 0..32.`);
      answer = answerer(need, revision, fresh || prepared ? { ...(fresh ? { fresh: true } : {}), ...(prepared ? { prepared: true } : {}) } : undefined);
    } catch (error) {
      // Today a throwing wrapper fails the compositor's frame: the frame is
      // not drawn and the error is reported. The engine's frame waits for an
      // answer that does not come (the last frame stays), and the error is
      // reported the same way.
      if (this.options.onProducerError) this.options.onProducerError(error, need);
      else this.options.log('warn', `the ${need.source} producer failed for layer ${need.layer}: ${error instanceof Error ? error.message : String(error)}`);
      answer = null;
    } finally {
      this.answering--;
      const ms = performance.now() - began;
      this.lookaheadStats.slowestAnswerMs = Math.max(this.lookaheadStats.slowestAnswerMs, ms);
      this.lookaheadStats.answerMs += ms;
      this.lookaheadStats.recentAnswers.push({ time: need.time, ms });
      if (this.lookaheadStats.recentAnswers.length > 12) this.lookaheadStats.recentAnswers.shift();
    }
    if (answer?.kind === 'reuse' && fresh) answer = null;
    if (!answer) {
      this.compatStats.unanswered++;
      return false;
    }
    const id = { composition: need.composition, layer: need.layer, time: need.time, source: need.source };
    beforePost?.(answer.kind === 'native' ? 'native' : `raster:${answer.key}`);
    let posted: boolean;
    if (answer.kind === 'native') {
      this.compatStats.natives++;
      posted = session.post({ type: 'compat', compat: 'native', ...id, scale: need.scale, revision, ...crop });
    } else if (answer.kind === 'reuse') {
      this.compatStats.reuses++;
      this.reused.add(`${tag}\u0000${revision}`);
      if (this.reused.size > 4096) this.reused.clear();
      posted = session.post({ type: 'compat', compat: 'reuse', id: { ...id, key: answer.key }, scale: need.scale, revision, ...crop });
    } else {
      this.compatStats.rasters++;
      posted = this.postRaster(session, { ...id, key: answer.key }, revision, need.scale, answer, need.crop ?? undefined);
    }
    if (posted) {
      this.trace.record(`answer posted: ${answer.kind}`, revision, need.time, began);
      this.logAnswer(need, revision, answer.kind, answer.kind === 'native' ? undefined : answer.key);
      this.postedFor.delete(tag);
      this.postedFor.set(tag, answer.kind === 'native' ? 'native' : `raster:${answer.key}`);
      if (this.postedFor.size > 4096) this.postedFor.delete(this.postedFor.keys().next().value!);
    }
    return posted;
  }

  private logAnswer(need: CompatNeed, revision: number, kind: 'raster' | 'reuse' | 'native', rasterKey?: string): void {
    const key = `${need.layer}\u0000${need.time}`;
    this.answerLog.delete(key);
    this.answerLog.set(key, { revision, kind, ...(rasterKey !== undefined ? { key: rasterKey } : {}) });
    if (this.answerLog.size > ANSWER_LOG_LIMIT) this.answerLog.delete(this.answerLog.keys().next().value!);
  }

  /** The latest answer this window posted for `layer` at `time` (ticks):
   *  its revision and kind, or null (diagnostics and the cadence gates). */
  answeredAt(layer: string, time: number): { revision: number; kind: 'raster' | 'reuse' | 'native' } | null {
    const entry = this.answerLog.get(`${layer}\u0000${time}`);
    return entry ? { revision: entry.revision, kind: entry.kind } : null;
  }

  private postRaster(session: TransportSession, id: { composition: string; layer: string; time: number; source: CompatSource; key: string }, revision: number, scale: number,
    raster: Extract<CompatAnswerResult, { kind: 'raster' }>, crop?: PixelRect): boolean {
    const bpp = raster.format === 'rgba16fPremultiplied' ? 8 : 4;
    if (raster.data.byteLength !== raster.width * raster.height * bpp) throw new EngineSnapshotError('invalidMessage', 'Compatibility pixels do not match their size.');
    const rowBytes = raster.width * bpp;
    const rowsPerBand = Math.max(1, Math.floor(MAX_COMPAT_BAND_BYTES / rowBytes));
    for (let row = 0; row < raster.height; row += rowsPerBand) {
      const rows = Math.min(rowsPerBand, raster.height - row);
      if (!session.post({
        type: 'compat', compat: 'pixels', id, revision, width: raster.width, height: raster.height, format: raster.format,
        origin: raster.anchor, scale, row, rows, data: raster.data.slice(row * rowBytes, (row + rows) * rowBytes),
        size: raster.size, ...(raster.uv ? { uv: raster.uv } : {}), ...(crop ? { crop: { ...crop } } : {})
      })) return false;
    }
    return true;
  }

  /** A snapshot round's compatibility needs: drawn and posted on the
   *  calling thread; throws when a need has no producer. */
  private answerRound(compat: CompatNeed[]): void {
    if (!compat.length) return;
    const session = this.options.session();
    if (!session) throw new EngineSnapshotError('unavailable', 'The window has no engine session to post compatibility pixels on.');
    for (const need of compat) {
      if (!this.answerNeed(need, session)) {
        throw new EngineSnapshotError('notHeadlessRenderable', `No ${need.source === 'rasterOverride' ? 'raster override' : 'compatibility context'} in this window answered for layer ${need.layer}.`);
      }
    }
  }

  /**
   * An asynchronous snapshot's round (a thumbnail, a capture, an async
   * `render.snapshot`): a producer that prepares asynchronously gets each
   * need's base dependencies over async IPC first, as playback does, and
   * the prepared answer is posted. Its caller never waits on a synchronous
   * base raster, which during playback would hold presentation and input.
   * A need whose prepared answer did not survive (a reset, a changed layer)
   * or whose preparation failed is answered as the synchronous round does,
   * so its pixels and its producer's error are today's.
   */
  private async answerRoundAsync(compat: CompatNeed[]): Promise<void> {
    if (!compat.length) return;
    const revision = this.options.revision() ?? 0;
    await Promise.allSettled(compat.map((need) => this.preparers.get(need.source)?.(need, revision)));
    const session = this.options.session();
    if (!session) throw new EngineSnapshotError('unavailable', 'The window has no engine session to post compatibility pixels on.');
    const unprepared = compat.filter((need) => !(this.preparers.has(need.source) && this.answerNeed(need, session, true)));
    this.answerRound(unprepared);
  }

  /** Resolves once a session is open, or after `ms`. */
  private async waitForSession(ms: number): Promise<void> {
    const until = performance.now() + ms;
    while (!this.ready && performance.now() < until) await new Promise((resolve) => setTimeout(resolve, 100));
  }

  /** Whether a session is open to take snapshots of. */
  get ready(): boolean {
    if (this.options.documentOnly) return !!this.options.windowDocument?.();
    const s = this.options.session();
    return !!s && s.id !== null && !s.closed;
  }

  /** The request main forwards, or why there is none. */
  private request(options: EngineSnapshotOptions, cls: SnapshotClass, uploaded: string[], detached = false, revision?: number | null): EngineSnapshotRequest {
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
    const blender = cls !== 'sync' && !layer && options.prepareBlender === true && options.draft3d !== true && (this.options.usesBlender?.() ?? true);
    const snapshot: SnapshotRequest = {
      composition: options.composition ?? this.options.composition(),
      time: ticks(time),
      outputSize: { width, height },
      target: options.source
        ? { kind: 'source', asset: options.source.asset, fit: options.source.fit ?? 'stretch', color: options.source.color ?? 'display' }
        : layer
        ? { kind: 'layer', layer: layer.id, scale: layer.scale, crop: layer.crop ?? null, content: layer.content ?? null }
        : { kind: 'frame', motionBlurSamples: Math.max(0, Math.min(256, Math.round(options.motionBlurSamples ?? 0))), background: options.background !== false, draft3d: options.draft3d === true },
      captionsHidden: !layer && !options.source && (options.captionsHidden ?? this.options.captionsHidden?.() ?? false),
      pixels: options.pixels ?? 'opaque',
      class: cls,
      viewSeq: options.viewer?.viewSeq ?? this.options.viewSeq(),
      ...(options.viewer ? { viewer: options.viewer } : {}),
      waitMs: Math.max(1, Math.min(120_000, Math.round(wait))),
      ...(blender ? { blenderWaitMs: BLENDER_WAIT_MS } : {}),
      uploaded
    };
    return { generation: session?.generation ?? 0, session: session?.id ?? null, revision: revision === undefined ? this.options.revision() : revision, snapshot };
  }

  private result(reply: Extract<EngineSnapshotReply, { status: 'rendered' }>): EngineSnapshot {
    const data = new Uint8ClampedArray(reply.data.buffer, reply.data.byteOffset, reply.data.byteLength);
    return { width: reply.width, height: reply.height, data, ...(reply.videoPlanes ? { videoPlanes: reply.videoPlanes } : {}), pixels: reply.pixels, revision: reply.revision, viewSeq: reply.viewSeq, origin: reply.origin, layerBox: reply.layerBox ?? null, raster: reply.raster ?? null, quality: reply.quality, waitedUs: reply.waitedUs, renderUs: reply.renderUs, mainMs: reply.mainMs };
  }

  private fail(error: unknown, where: string): EngineSnapshotError {
    const e = error instanceof EngineSnapshotError ? error : new EngineSnapshotError('internal', error instanceof Error ? error.message : String(error));
    this.stats.failed++;
    this.stats.lastFailure = `${e.code}: ${e.message}`;
    this.options.onFailure?.(e.code, e.message);
    this.options.log('warn', `engine ${where} failed (${e.code}): ${e.message}`);
    return e;
  }

  /** A round's needs for a snapshot of main's own session for the window:
   *  drawn here, and carried with the next request (main posts them on its
   *  session). Throws when a need has no producer. */
  private collectAnswers(compat: CompatNeed[]): EngineCompatEvent[] {
    const events: EngineCompatEvent[] = [];
    const carrier: TransportSession = {
      id: 0, closed: false, generation: 0, resyncPending: false, applied: 1,
      uploadFallback: () => {},
      post: (event) => { events.push(event as EngineCompatEvent); return true; }
    };
    for (const need of compat) {
      if (!this.answerNeed(need, carrier)) {
        throw new EngineSnapshotError('notHeadlessRenderable', `No ${need.source === 'rasterOverride' ? 'raster override' : 'compatibility context'} in this window answered for layer ${need.layer}.`);
      }
    }
    return events;
  }

  /**
   * A snapshot the calling thread waits for. Throws `EngineSnapshotError`
   * on failure. Safe inside `raster` wrappers and WebGL callbacks: nothing
   * here waits for a task.
   */
  snapshotSync(options: EngineSnapshotOptions): EngineSnapshot {
    options = pinViewer(options);
    const started = performance.now();
    const native = this.options.native;
    if (!native.snapshotSync) throw this.fail(new EngineSnapshotError('unavailable', 'This host has no synchronous engine snapshots.'), 'snapshot');
    // Flush pending edits first (once): a flush that starts a staged resync
    // must be seen before choosing the path below.
    const revision = this.options.revision();
    // This thread blocks until the answer: the window's own session serves
    // it only when it has the document and no staged resync is on its way
    // (the resync's next step would need this thread). Otherwise main serves
    // it from a session of its own, with the document sent along.
    const session = this.options.session();
    const document = this.options.windowDocument?.();
    // A native document reference also survives a killed utility before this
    // blocked renderer has received its port-close notification. Main restores
    // and adopts it independently of the stale viewer session.
    const direct = !document?.nativeKernel && !!session && session.id !== null && !session.closed && !session.resyncPending && (session.applied ?? 1) > 0;
    if (!direct && document) {
      // The window's own session (the standby) serves later ones.
      if (!session || session.closed) void this.options.wake?.();
      try {
        return this.documentSnapshotSync(options, started, revision);
      } catch (error) {
        if (error instanceof EngineSnapshotError && error.code === 'empty') throw error;
        throw this.fail(error, 'snapshot');
      }
    }
    const uploaded: string[] = [];
    let compatUploaded: CompatNeed[] = [];
    const dependencies = new SnapshotDependencies();
    try {
      for (let round = 0; round < MAX_DEPENDENCY_PHASES; round++) {
        const request = this.request(options, 'sync', uploaded, false, revision);
        request.snapshot.compatUploaded = compatUploaded;
        const reply = native.snapshotSync(request);
        if (reply.status === 'rendered') {
          this.stats.sync++;
          this.stats.slowestSyncMs = Math.max(this.stats.slowestSyncMs, performance.now() - started);
          return this.result(reply);
        }
        if (reply.status === 'empty') throw new EngineSnapshotError('empty', 'The layer draws nothing at that time.');
        if (reply.status === 'failed') throw failure(reply);
        dependencies.observe(reply);
        this.stats.rounds++;
        this.answerRound(reply.compat);
        compatUploaded = reply.compat;
        if (!reply.fallback.length) continue;
        const runner = this.options.fallbackRunner();
        const session = this.options.session();
        if (!runner || !session) throw new EngineSnapshotError('unavailable', 'No shader fallback runner to draw the frame’s programs.');
        for (const request of reply.fallback) {
          if (uploaded.includes(request.key)) throw new EngineSnapshotError('internal', `The engine asked again for program ${request.key.slice(0, 12)}.`);
          session.uploadFallback(runner.renderSync(request, fallbackTextures(reply, request)));
          uploaded.push(request.key);
        }
      }
      throw new EngineSnapshotError('internal', `The snapshot exceeded ${MAX_DEPENDENCY_PHASES} renderer dependency phases.`);
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'cancelled') throw error;
      if (error instanceof EngineSnapshotError && error.code === 'empty') throw error;
      throw this.fail(error, 'snapshot');
    }
  }

  /** The last document revision main's session for this window has. */
  private sentToMain: { id: string; revision: number } | null = null;

  /** A sync snapshot main serves from its own session for this window. */
  private documentSnapshotSync(options: EngineSnapshotOptions, started: number, flushed: number | null): EngineSnapshot {
    const native = this.options.native;
    const doc = this.options.windowDocument!()!;
    const revision = flushed ?? doc.revision;
    const uploaded: string[] = [];
    let compatUploaded: CompatNeed[] = [];
    const dependencies = new SnapshotDependencies();
    let fallbackPixels: { key: string; width: number; height: number; rgba16f: Uint8Array; rgba32f?: Uint8Array }[] = [];
    // The window's producers go with every request (main's session then
    // asks for those layers), and the answers to a round's needs with the next.
    let compatEvents: EngineCompatEvent[] = this.answerers.size ? [{ type: 'compat', compat: 'producers', sources: [...this.answerers.keys()] }] : [];
    let send = !(this.sentToMain && this.sentToMain.id === doc.id && this.sentToMain.revision === doc.revision);
    let body: { json: Uint8Array; hash: string; revision: number } | null = null;
    for (let round = 0; round < MAX_DEPENDENCY_PHASES + 1; round++) {
      if (send && !body && !doc.nativeKernel) body = doc.full();
      const request = this.request(options, 'sync', uploaded, true, revision);
      request.snapshot.compatUploaded = compatUploaded;
      const reply = native.snapshotSync!({
        ...request,
        session: null,
        revision,
        document: doc.nativeKernel ? { id: doc.id, revision: doc.revision, nativeKernel: true }
          : body && send ? { id: doc.id, revision: body.revision, hash: body.hash, json: body.json } : { id: doc.id, revision: doc.revision },
        ...(fallbackPixels.length ? { fallbackPixels } : {}),
        ...(compatEvents.length ? { compatEvents } : {})
      });
      fallbackPixels = [];
      compatEvents = this.answerers.size ? [{ type: 'compat', compat: 'producers', sources: [...this.answerers.keys()] }] : [];
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
      dependencies.observe(reply);
      this.stats.rounds++;
      if (reply.compat.length) compatEvents.push(...this.collectAnswers(reply.compat));
      compatUploaded = reply.compat;
      if (!reply.fallback.length) continue;
      const runner = this.options.fallbackRunner();
      if (!runner) throw new EngineSnapshotError('unavailable', 'No shader fallback runner to draw the frame’s programs.');
      for (const r of reply.fallback) {
        if (uploaded.includes(r.key)) throw new EngineSnapshotError('internal', `The engine asked again for program ${r.key.slice(0, 12)}.`);
        const pixels = runner.renderSync(r, fallbackTextures(reply, r));
        fallbackPixels.push({ key: pixels.key, width: pixels.width, height: pixels.height,
          rgba16f: new Uint8Array(pixels.rgba16f.buffer, pixels.rgba16f.byteOffset, pixels.rgba16f.byteLength),
          ...(pixels.rgba32f ? { rgba32f: new Uint8Array(pixels.rgba32f.buffer, pixels.rgba32f.byteOffset, pixels.rgba32f.byteLength) } : {}) });
        uploaded.push(r.key);
      }
    }
    throw new EngineSnapshotError('internal', `The snapshot exceeded ${MAX_DEPENDENCY_PHASES} renderer dependency phases.`);
  }

  /** The same, asynchronously (`async` or `idle` class). */
  async snapshot(options: EngineSnapshotOptions, cls: Exclude<SnapshotClass, 'sync'> = 'async'): Promise<EngineSnapshot> {
    return this.snapshotAsyncAt(pinViewer(options), cls);
  }

  /** This window's asynchronous snapshots, queued to what main admits (snapshot-slots.ts). */
  private readonly slots = new SnapshotSlots();

  /**
   * One snapshot request through the window's slots. Main can still refuse
   * one for the bytes every window has outstanding: that is a wait as well
   * (today's capture waits for the GPU), retried as slots free up, within
   * the class's own wait.
   */
  private async admitted(cls: SnapshotClass, ask: () => Promise<EngineSnapshotReply>): Promise<EngineSnapshotReply> {
    const until = performance.now() + (cls === 'idle' ? IDLE_WAIT_MS : ASYNC_WAIT_MS);
    for (let attempt = 0; ; attempt++) {
      const reply = await this.slots.run(cls, ask);
      if (reply.status !== 'failed' || reply.code !== 'refused' || performance.now() >= until) return reply;
      await new Promise((resolve) => setTimeout(resolve, Math.min(250, 10 * 2 ** attempt)));
    }
  }

  private async snapshotAsyncAt(options: EngineSnapshotOptions, cls: SnapshotClass): Promise<EngineSnapshot> {
    const native = this.options.native;
    if (!native.snapshot) throw this.fail(new EngineSnapshotError('unavailable', 'This host has no engine snapshots.'), 'snapshot');
    if (this.options.documentOnly) return this.documentSnapshot(options, cls);
    const uploaded: string[] = [];
    let compatUploaded: CompatNeed[] = [];
    const dependencies = new SnapshotDependencies();
    try {
      if (!this.ready) {
        // UI snapshots wait for the viewer's own session first (it opens at
        // boot); a standby opens only when none comes.
        if (cls === 'idle') await this.waitForSession(STANDBY_AFTER_MS);
        if (!this.ready) await this.options.wake?.();
      }
      for (let round = 0; round < MAX_DEPENDENCY_PHASES; round++) {
        const request = this.request(options, cls, uploaded);
        request.snapshot.compatUploaded = compatUploaded;
        const began = performance.now();
        const reply = await this.admitted(cls, () => this.askSnapshot(request, options.signal));
        this.trace.record('snapshot round trip', Number(request.revision ?? 0), request.snapshot.time, began);
        if (reply.status === 'rendered') {
          this.trace.record('snapshot main', reply.revision, request.snapshot.time, began, Math.round(reply.mainMs * 1000));
          this.trace.record('snapshot native wait', reply.revision, request.snapshot.time, began, reply.waitedUs);
          this.trace.record('snapshot native render', reply.revision, request.snapshot.time, began, reply.renderUs);
        }
        if (reply.status === 'rendered') {
          if (cls === 'idle') this.stats.idle++;
          else this.stats.async++;
          return this.result(reply);
        }
        if (reply.status === 'empty') throw new EngineSnapshotError('empty', 'The layer draws nothing at that time.');
        if (reply.status === 'failed') throw failure(reply);
        dependencies.observe(reply);
        this.stats.rounds++;
        await this.answerRoundAsync(reply.compat);
        compatUploaded = reply.compat;
        if (!reply.fallback.length) continue;
        const runner = this.options.fallbackRunner();
        const session = this.options.session();
        if (!runner || !session) throw new EngineSnapshotError('unavailable', 'No shader fallback runner to draw the frame’s programs.');
        for (const request of reply.fallback) {
          if (uploaded.includes(request.key)) throw new EngineSnapshotError('internal', `The engine asked again for program ${request.key.slice(0, 12)}.`);
          session.uploadFallback(await runner.render(request, fallbackTextures(reply, request)));
          uploaded.push(request.key);
        }
      }
      throw new EngineSnapshotError('internal', `The snapshot exceeded ${MAX_DEPENDENCY_PHASES} renderer dependency phases.`);
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'cancelled') throw error;
      if (error instanceof EngineSnapshotError && error.code === 'empty') throw error;
      throw this.fail(error, 'snapshot');
    }
  }

  private async askSnapshot(request: EngineSnapshotRequest, signal?: AbortSignal): Promise<EngineSnapshotReply> {
    if (signal?.aborted) throw new EngineSnapshotError('cancelled', 'The source request was cancelled.');
    const requestId = crypto.randomUUID();
    const cancel = () => { void this.options.native.cancelSnapshot?.(requestId); };
    signal?.addEventListener('abort', cancel, { once: true });
    try { return await this.options.native.snapshot!({ ...request, requestId }); }
    finally { signal?.removeEventListener('abort', cancel); }
  }

  private async documentSnapshot(options: EngineSnapshotOptions, cls: SnapshotClass): Promise<EngineSnapshot> {
    try {
      const revision = this.options.revision();
      const doc = this.options.windowDocument?.();
      if (!doc) throw new EngineSnapshotError('noSession', 'The native document is not ready.');
      const body = doc.full();
      const uploaded: string[] = [];
      let fallbackPixels: { key: string; width: number; height: number; rgba16f: Uint8Array }[] = [];
      for (let round = 0; round < MAX_ROUNDS; round++) {
        const reply = await this.askSnapshot({ ...this.request(options, cls, uploaded, true, revision), session: null,
          document: { id: doc.id, revision: body.revision, hash: body.hash, json: body.json }, fallbackPixels }, options.signal);
        fallbackPixels = [];
        if (reply.status === 'rendered') { this.stats[cls]++; return this.result(reply); }
        if (reply.status === 'empty') throw new EngineSnapshotError('empty', 'The layer draws nothing at that time.');
        if (reply.status === 'failed') throw failure(reply);
        this.stats.rounds++;
        // This path has no session to post compatibility pixels on.
        if (reply.compat.length) throw new EngineSnapshotError('notHeadlessRenderable', `The frame needs ${reply.compat.length} compatibility raster(s) this snapshot path does not draw.`);
        const runner = this.options.fallbackRunner();
        for (const request of reply.fallback) {
          if (!runner || uploaded.includes(request.key)) throw new EngineSnapshotError('unavailable', 'The requested ANGLE program is unavailable.');
          const pixels = await runner.render(request, fallbackTextures(reply, request));
          fallbackPixels.push({ key: pixels.key, width: pixels.width, height: pixels.height,
            rgba16f: new Uint8Array(pixels.rgba16f.buffer, pixels.rgba16f.byteOffset, pixels.rgba16f.byteLength) }); uploaded.push(request.key);
        }
      }
      throw new EngineSnapshotError('internal', `The snapshot still needed the renderer after ${MAX_ROUNDS} rounds.`);
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'cancelled') throw error;
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

  /** Playback depends on these pixels. Await over async IPC, but admit at
   * sync priority: ordinary async snapshots yield to a due viewer frame,
   * which itself cannot render until this compatibility answer arrives. */
  async rasterForCompat(layer: string, scale: number, time: number, crop: { x: number; y: number; width: number; height: number } | null = null, options: RasterOptions = {}): Promise<EngineSnapshot | null> {
    try {
      return await this.snapshotAsyncAt(this.rasterOptions(layer, scale, time, crop, options), 'sync');
    } catch (error) {
      if (error instanceof EngineSnapshotError && error.code === 'empty') return null;
      throw error;
    }
  }

  private rasterOptions(layer: string, scale: number, time: number, crop: { x: number; y: number; width: number; height: number } | null, options: RasterOptions): EngineSnapshotOptions {
    scale = layerRasterScale(scale);
    const edge = options.maxEdge ?? MAX_RASTER_EDGE;
    return { time, width: edge, height: edge, pixels: 'premultiplied', composition: options.composition, layer: { id: layer, scale, crop, content: options.content ?? null } };
  }

  /* ── today's render API shapes (apiVersion 1 to 3 and the async siblings) ── */

  /** A snapshot as a canvas (today's capture shapes build on it). */
  toCanvas(snapshot: EngineSnapshot): HTMLCanvasElement {
    return this.canvas(snapshot);
  }

  private canvas(snapshot: EngineSnapshot): HTMLCanvasElement {
    if (snapshot.videoPlanes) throw new EngineSnapshotError('invalidMessage', 'Native source video planes are presented through their video track.');
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

  snapshotDataUrlSync(time: number, maxWidth: number, size: { w: number; h: number }, options: { captionsHidden?: boolean } = {}): string {
    try {
      return this.canvas(this.snapshotSync({ ...this.snapshotOptions(time, maxWidth, size), captionsHidden: options.captionsHidden })).toDataURL('image/jpeg', 0.74);
    } catch (error) {
      // Today's capture of a Blender composition with no prepared frame
      // throws (`compositor.ts`); it does not come back empty.
      if (error instanceof EngineSnapshotError && error.code === 'blenderFailed') throw new Error(error.message);
      return '';
    }
  }

  /** The same, asynchronously. `prepareBlender`: today's `snapshotAsync`,
   *  which has Blender render the frame first; without it, today's
   *  synchronous capture (a Blender frame nobody prepared fails). */
  async snapshotDataUrl(time: number, maxWidth: number, size: { w: number; h: number }, options: { prepareBlender?: boolean; captionsHidden?: boolean } = {}): Promise<string> {
    const snapshot = { ...this.snapshotOptions(time, maxWidth, size), prepareBlender: options.prepareBlender === true, captionsHidden: options.captionsHidden };
    return this.canvas(await this.snapshot(snapshot)).toDataURL('image/jpeg', 0.74);
  }

  /** The window's document revision, after flushing pending edits. */
  currentRevision(): number | null {
    return this.options.revision();
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

  /** A local hidden host asks this same producer without an Electron port.
   *  `layer`: the export snapshot's layer (JSON), when the ask carries it. */
  async compatibility(request: CompatFrameRequest, layer?: string): Promise<EngineCompatAnswer> {
    const no = (message: string): EngineCompatAnswer => ({ ask: 0, status: 'unavailable', message });
    // An ask carrying the export snapshot's layer is drawn from it in any
    // window with the producer (today's render queue ran its jobs in the
    // window, whatever it had open); otherwise only the document's window.
    if (layer === undefined && request.document !== this.options.document()) return no(OTHER_DOCUMENT_OPEN);
    const producer = this.producers.get(request.source);
    const answerer = this.answerers.get(request.source);
    if (!producer && answerer) {
      // An export frame's layer (the job engine has no store of uploaded
      // rasters): the producer's answer, with fresh pixels.
      this.compatStats.exportAsks++;
      let result: CompatAnswerResult = null;
      this.answering++;
      try {
        result = answerer({ composition: request.composition, layer: request.layer, time: request.time, source: request.source, scale: request.scale }, Number(request.revision), { fresh: true, ...(layer !== undefined ? { layer } : {}) });
      } catch (error) {
        return no(error instanceof Error ? error.message : String(error));
      } finally {
        this.answering--;
      }
      if (!result) return no(`Layer ${request.layer} has no compatibility pixels at that frame.`);
      if (result.kind === 'native') return { ask: 0, status: 'native' };
      if (result.kind === 'reuse') return no(`Layer ${request.layer}'s raster was not drawn again.`);
      return { ask: 0, status: 'pixels', key: result.key, width: result.width, height: result.height, format: result.format, origin: result.anchor, scale: request.scale, data: result.data, size: result.size, ...(result.uv ? { uv: result.uv } : {}) };
    }
    if (!producer) return no(`No ${request.source === 'rasterOverride' ? 'raster override' : 'compatibility context'} draws layer ${request.layer} in this window.`);
    try {
      if (request.source === 'compatContext') this.compatStats.exportAsks++;
      const pixels = await producer(request);
      if (pixels === 'native') return { ask: 0, status: 'native' };
      if (!pixels) return no(`Layer ${request.layer} has no compatibility pixels at that frame.`);
      return { ask: 0, status: 'pixels', ...pixels };
    } catch (error) {
      return no(error instanceof Error ? error.message : String(error));
    }
  }

  private async answerAsk(ask: EngineCompatAsk): Promise<void> {
    const answer = this.options.native.answerCompat;
    if (answer) answer({ ...await this.compatibility(ask.request, ask.layer), ask: ask.ask });
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
  async request(request: ThumbnailRequest, revision: () => number | null = () => this.transport.currentRevision()): Promise<EngineSnapshot | null> {
    const ticket = ++this.next;
    this.latest.set(request.key, ticket);
    const required = request.minRevision ?? revision() ?? 0;
    const viewSeq = request.viewer?.viewSeq;
    try {
      const result = await this.transport.snapshot(request, 'idle');
      if (this.latest.get(request.key) !== ticket || result.revision < required
        || (viewSeq !== undefined && result.viewSeq !== viewSeq)) {
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
