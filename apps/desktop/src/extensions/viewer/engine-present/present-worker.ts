/*
 * Engine presentation off the page's main thread (docs/engine/bridge.md,
 * "Presenting from a worker").
 *
 * The viewer's canvas hands its drawing to an `OffscreenCanvas` in a
 * dedicated worker, and the worker runs the same `EnginePresenter` (jitter
 * buffer, F16 painter, release on GPU completion) on its own
 * animation frames. Chromium drives a worker's animation frames and
 * commits its canvas without the page's main thread, so a frame that is
 * already queued here reaches the display on time while the editor's main
 * thread is busy (a live window resize lays out every panel; S1's long
 * tasks). On the main-thread presenter those stalls dropped engine frames
 * that had arrived 60 to 130 ms early.
 *
 * The page keeps everything that needs the main thread (the engine port,
 * the preload's imports, viewer state, overlays) and sends each descriptor
 * here with its `VideoFrame` as soon as it arrives (`WorkerPresenter` in
 * worker-presenter.ts). Every time in this worker is on the page's clock
 * (`performance.now()` of the window that made the worker), so arrivals,
 * animation frames, and traces compare with the page's.
 *
 * The painter (the canvas's F16 graphics context) lives as long as the worker; a
 * presenter lives for one viewer session (`open` to `close`), as on the
 * main thread. Messages carry the session, so a release or a presented
 * frame of an ended session never reaches the next one.
 *
 * This module is the worker's side; `present-worker-entry.ts` is the
 * worker script.
 */
import type { FramePainter, PaintTarget } from './gpu';
import { createFramePainter } from './painter';
import { EnginePresenter, type PresentedFrame, type PresenterStats, type TraceRecord } from './presenter';
import type { ClockSample, FrameDescriptor, FrameRelease, SurfacePool, VsyncHint } from './protocol';

/** Page → worker. */
export type ToWorker =
  | {
      type: 'init';
      canvas: OffscreenCanvas;
      /** The page's `performance.timeOrigin`, to put worker times on its clock. */
      pageTimeOrigin: number;
    }
  | { type: 'open'; session: number; displayHz: number | null; trace: boolean; generation: number }
  | { type: 'close'; session: number }
  | { type: 'clock'; session: number; sample: ClockSample; arrivedMs: number }
  | { type: 'pool'; session: number; pool: SurfacePool }
  | { type: 'frame'; session: number; desc: FrameDescriptor; frame: VideoFrame; arrivedMs: number }
  | { type: 'hostOffset'; session: number; us: number | null }
  | { type: 'display'; session: number; displayHz: number | null }
  | { type: 'resize'; session: number }
  | { type: 'takeTrace'; session: number; id: number }
  | { type: 'dispose' };

/** Worker → page. */
export type FromWorker =
  | { type: 'ready' }
  | { type: 'failed'; message: string }
  /** The painter's device was lost: this worker draws nothing more (its
   *  session's frames went back to the engine); the page replaces it. */
  | { type: 'lost'; message: string }
  | { type: 'release'; session: number; release: FrameRelease }
  /** A frame drawn, with the session's stats after it. */
  | { type: 'presented'; session: number; frame: PresentedFrame; stats: PresenterStats }
  /** A drawn frame's GPU work finished (its canvas frame is ready to show). */
  | { type: 'done'; session: number; seq: number }
  | { type: 'vsync'; session: number; hint: VsyncHint }
  | { type: 'stats'; session: number; stats: PresenterStats }
  /** A frame of pool `generation` could not be read (`onUnusable`). */
  | { type: 'unusable'; session: number; generation: number; message: string }
  | { type: 'trace'; session: number; id: number; trace: TraceRecord | null }
  /** The session's presenter is gone: nothing more comes for it. */
  | { type: 'closed'; session: number }
  | { type: 'disposed' };

/** The slice of `DedicatedWorkerGlobalScope` the worker uses. */
export interface WorkerScope {
  postMessage(message: FromWorker): void;
  addEventListener(type: 'message', listener: (event: MessageEvent<ToWorker>) => void): void;
  requestAnimationFrame(callback: (t: number) => void): number;
  cancelAnimationFrame(id: number): void;
  readonly performance: { now(): number; readonly timeOrigin: number };
}

export type ServeOptions = {
  timers?: { set(fn: () => void, ms: number): number; clear(id: number): void };
  createPainter?: (canvas: PaintTarget) => Promise<FramePainter>;
  /** Wraps the painter (the integration test reads drawn frames back). */
  wrapPainter?: (painter: FramePainter) => Promise<FramePainter> | FramePainter;
  /** Every frame drawn, in the worker (the integration test checks its pixels). */
  onPresented?: (frame: PresentedFrame) => void;
};

/**
 * Timed canvas commits use a fixed phase after BeginFrame, targeting the
 * following compositor refresh rather than racing the page's frame for
 * the current one. The prediction is two refreshes to the panel, including
 * the following draw's presentation, plus its measured 3.2 ms offset
 * (docs/engine/bridge.md, "A/V sync"). Paused/gesture frames still submit
 * on arrival. The common-clock performance gate must validate the phase
 * and A/V prediction on an awake display (docs/engine/perf.md).
 */
export const WORKER_LATENCY_REFRESHES = 2;
/** Submit after this BeginFrame's compositor deadline, before the next one. */
export const WORKER_COMMIT_PHASE_REFRESHES = 0.75;
/**
 * Added while the window is being resized (docs/engine/perf.md, "A/V sync
 * under load"): a live resize holds the panel's presentation of a worker
 * canvas one to three refreshes longer, which left frames a refresh or two
 * late outside ±10 ms (0.75: the best of 0.5, 0.75, and 1.0 on the
 * measured offsets of six loaded runs). Idle prediction is unchanged: a
 * later one there puts the runs where draw and presentation take one
 * refresh outside ±10 ms (measured at 1.75: 208 of 1,433 frames at
 * 23.976 fps).
 */
export const WORKER_RESIZE_LATENCY_REFRESHES = 0.75;
export const WORKER_PRESENT_OFFSET_MS = 3.2;

/** How often the worker sends a session's stats while frames flow. */
export const STATS_INTERVAL_MS = 250;

/** A frame source for a presenter that only gets `present()`ed frames. */
const NO_SOURCE = { videoFrame: () => null, onSurface: () => () => {}, retire: () => 0 };

type Session = { id: number; presenter: EnginePresenter; hostOffsetUs: number | null; statsSentMs: number };

export function servePresenter(scope: WorkerScope, options: ServeOptions = {}): void {
  let painter: FramePainter | null = null;
  let session: Session | null = null;
  let shift = 0;
  let disposed = false;
  let failed = false;
  const closing = new Set<Promise<void>>();
  /** Messages that arrived before the painter was ready, in order. */
  let pending: ToWorker[] | null = [];
  const now = (): number => scope.performance.now() + shift;
  const post = (message: FromWorker): void => scope.postMessage(message);
  const requestFrame = (cb: (t: number) => void): number => scope.requestAnimationFrame((t) => cb(t + shift));
  const cancelFrame = (id: number): void => scope.cancelAnimationFrame(id);

  /** A frame this worker will never draw: closed, and its surface handed back to the engine. */
  const giveBack = (message: Extract<ToWorker, { type: 'frame' }>): void => {
    message.frame.close();
    post({ type: 'release', session: message.session, release: { poolIndex: message.desc.poolIndex, seq: message.desc.seq } });
  };

  const sendStats = (s: Session, force = false): void => {
    const t = now();
    if (!force && t - s.statsSentMs < STATS_INTERVAL_MS) return;
    s.statsSentMs = t;
    const stats = s.presenter.stats;
    post({ type: 'stats', session: s.id, stats: { ...stats, dropped: { ...stats.dropped } } });
  };

  const close = (): void => {
    if (!session) return;
    // Its queued frames go back to the engine first; the final stats tell
    // the page the session is done.
    const ended = session;
    ended.presenter.dispose(cancelFrame);
    session = null;
    // The page retains the last VideoFrame reference. Acknowledging before
    // the queue fence lets it return an IOSurface still read by the GPU.
    const drained = ended.presenter.drained().then(() => {
      sendStats(ended, true);
      post({ type: 'closed', session: ended.id });
    });
    closing.add(drained);
    void drained.then(() => closing.delete(drained));
  };

  const open = (message: Extract<ToWorker, { type: 'open' }>): void => {
    close();
    const id = message.session;
    const s: Session = {
      id,
      hostOffsetUs: null,
      statsSentMs: -Infinity,
      presenter: new EnginePresenter({
        link: {
          // The page sends viewer states itself; the worker only releases.
          sendViewerState: () => {},
          release: (release) => post({ type: 'release', session: id, release })
        },
        source: NO_SOURCE,
        painter: painter!,
        displayHz: message.displayHz,
        generation: message.generation,
        // The worker's canvas commits on its own: a gesture frame drawn
        // now can reach the display frame being composed, instead of
        // waiting for the worker's next animation frame.
        drawOnArrival: true,
        commitPhaseRefreshes: WORKER_COMMIT_PHASE_REFRESHES,
        timers: options.timers,
        // Timed frames are picked for the instant a worker canvas's frame
        // reaches the panel (WORKER_LATENCY_REFRESHES), rounded to the
        // nearest refresh, so they stay on the audio clock's line.
        latencyRefreshes: WORKER_LATENCY_REFRESHES,
        resizeLatencyRefreshes: WORKER_RESIZE_LATENCY_REFRESHES,
        presentOffsetMs: WORKER_PRESENT_OFFSET_MS,
        nearestRefresh: true,
        now,
        trace: message.trace,
        hostOffsetUs: () => s.hostOffsetUs,
        onVsync: (hint) => post({ type: 'vsync', session: id, hint }),
        onUnusable: (generation, message) => post({ type: 'unusable', session: id, generation, message })
      })
    };
    s.presenter.onPresented((frame) => {
      options.onPresented?.(frame);
      const stats = s.presenter.stats;
      s.statsSentMs = now();
      post({ type: 'presented', session: id, frame, stats: { ...stats, dropped: { ...stats.dropped } } });
    });
    s.presenter.onDone((seq) => post({ type: 'done', session: id, seq }));
    s.presenter.start(requestFrame, cancelFrame);
    session = s;
  };

  const handle = (message: ToWorker): void => {
    // A worker that failed still answers `dispose`, so the page can end it at once.
    if (disposed || (failed && message.type !== 'dispose')) {
      if (message.type === 'frame') giveBack(message);
      return;
    }
    if (message.type === 'init') {
      void init(message);
      return;
    }
    if (message.type === 'dispose') {
      disposed = true;
      for (const queued of pending ?? []) if (queued.type === 'frame') giveBack(queued);
      pending = null;
      close();
      void Promise.all(closing).then(() => {
        painter?.dispose();
        painter = null;
        post({ type: 'disposed' });
      });
      return;
    }
    if (pending) {
      pending.push(message);
      return;
    }
    if (message.type === 'open') {
      open(message);
      return;
    }
    // Everything else belongs to one session; an ended one's is dropped.
    const s = session && session.id === message.session ? session : null;
    switch (message.type) {
      case 'close':
        if (s) close();
        return;
      case 'clock':
        s?.presenter.receive({ type: 'clock', sample: message.sample }, message.arrivedMs);
        return;
      case 'pool':
        s?.presenter.receive({ type: 'surfacePool', pool: message.pool });
        return;
      case 'frame':
        if (!s) {
          giveBack(message);
          return;
        }
        s.presenter.present(message.desc, message.frame, message.arrivedMs);
        sendStats(s);
        return;
      case 'hostOffset':
        if (s) s.hostOffsetUs = message.us;
        return;
      case 'display':
        s?.presenter.retarget(requestFrame, cancelFrame, message.displayHz);
        return;
      case 'resize':
        s?.presenter.noteResize();
        return;
      case 'takeTrace': {
        const trace = s?.presenter.trace ?? null;
        if (s) sendStats(s, true);
        post({
          type: 'trace',
          session: message.session,
          id: message.id,
          trace: trace && { arrivals: trace.arrivals.splice(0), draws: trace.draws.splice(0), drops: trace.drops.splice(0), releases: trace.releases.splice(0), rafs: trace.rafs.splice(0) }
        });
        return;
      }
    }
  };

  const init = async (message: Extract<ToWorker, { type: 'init' }>): Promise<void> => {
    shift = scope.performance.timeOrigin - message.pageTimeOrigin;
    try {
      const made = await (options.createPainter ?? createFramePainter)(message.canvas);
      painter = options.wrapPainter ? await options.wrapPainter(made) : made;
    } catch (error) {
      failed = true;
      for (const queued of pending ?? []) if (queued.type === 'frame') giveBack(queued);
      pending = null;
      post({ type: 'failed', message: error instanceof Error ? error.message : String(error) });
      return;
    }
    if (disposed) {
      painter.dispose();
      painter = null;
      return;
    }
    void painter.lost?.then((message) => {
      if (disposed || failed) return;
      // Nothing may touch the lost device again: the session's frames go
      // back to the engine, later ones are handed back as they come.
      failed = true;
      post({ type: 'lost', message });
      close();
    });
    const queued = pending ?? [];
    pending = null;
    post({ type: 'ready' });
    for (const m of queued) handle(m);
  };

  scope.addEventListener('message', (event) => handle(event.data));
}
