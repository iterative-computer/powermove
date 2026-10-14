/*
 * Engine presentation off the page's main thread (docs/engine/bridge.md,
 * "Presenting from a worker").
 *
 * The viewer's canvas hands its drawing to an `OffscreenCanvas` in a
 * dedicated worker, and the worker runs the same `EnginePresenter` (jitter
 * buffer, WebGPU painter, release on `onSubmittedWorkDone`) on its own
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
 * The painter (the canvas's WebGPU context) lives as long as the worker; a
 * presenter lives for one viewer session (`open` to `close`), as on the
 * main thread. Messages carry the session, so a release or a presented
 * frame of an ended session never reaches the next one.
 *
 * This module is the worker's side; `present-worker-entry.ts` is the
 * worker script.
 */
import { createWebGpuPainter, type FramePainter, type PaintTarget } from './gpu';
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
  | { type: 'takeTrace'; session: number; id: number }
  | { type: 'dispose' };

/** Worker → page. */
export type FromWorker =
  | { type: 'ready' }
  | { type: 'failed'; message: string }
  | { type: 'release'; session: number; release: FrameRelease }
  /** A frame drawn, with the session's stats after it. */
  | { type: 'presented'; session: number; frame: PresentedFrame; stats: PresenterStats }
  | { type: 'vsync'; session: number; hint: VsyncHint }
  | { type: 'stats'; session: number; stats: PresenterStats }
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
  createPainter?: (canvas: PaintTarget) => Promise<FramePainter>;
  /** Wraps the painter (the integration test reads drawn frames back). */
  wrapPainter?: (painter: FramePainter) => Promise<FramePainter> | FramePainter;
};

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
    session.presenter.dispose(cancelFrame);
    sendStats(session, true);
    post({ type: 'closed', session: session.id });
    session = null;
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
        now,
        trace: message.trace,
        hostOffsetUs: () => s.hostOffsetUs,
        onVsync: (hint) => post({ type: 'vsync', session: id, hint })
      })
    };
    s.presenter.onPresented((frame) => {
      const stats = s.presenter.stats;
      s.statsSentMs = now();
      post({ type: 'presented', session: id, frame, stats: { ...stats, dropped: { ...stats.dropped } } });
    });
    s.presenter.start(requestFrame, cancelFrame);
    session = s;
  };

  const handle = (message: ToWorker): void => {
    if (disposed || failed) {
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
      painter?.dispose();
      painter = null;
      post({ type: 'disposed' });
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
      const made = await (options.createPainter ?? createWebGpuPainter)(message.canvas);
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
    const queued = pending ?? [];
    pending = null;
    post({ type: 'ready' });
    for (const m of queued) handle(m);
  };

  scope.addEventListener('message', (event) => handle(event.data));
}
