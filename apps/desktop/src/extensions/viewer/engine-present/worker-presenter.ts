/*
 * The page's side of off-main-thread presentation (present-worker.ts).
 *
 * `PresentWorker` owns the worker and the `OffscreenCanvas` the viewer's
 * canvas handed it, for as long as that canvas exists. `WorkerPresenter`
 * is one viewer session on it and stands in for `EnginePresenter`: what
 * needs the page's main thread stays here (the preload's imports, parking a
 * descriptor until its import arrives, retiring a replaced pool's imports,
 * releases to the engine), and each descriptor goes to the worker with its
 * `VideoFrame` the moment it arrives, so a frame the engine rendered ahead
 * is already in the worker when the page's main thread stalls.
 */
import type { FromWorker, ToWorker } from './present-worker';
import type { FrameLink, FrameSource, Presenter, PresentedFrame, PresenterStats, TraceRecord } from './presenter';
import type { EngineFrameMessage, FrameDescriptor, ViewerState, VsyncHint } from './protocol';

/** The slice of `Worker` used here (a fake in tests). */
export interface WorkerLike {
  postMessage(message: ToWorker, transfer?: Transferable[]): void;
  addEventListener(type: 'message', listener: (event: MessageEvent<FromWorker>) => void): void;
  addEventListener(type: 'error', listener: (event: Event) => void): void;
  terminate(): void;
}

const PARK_TIMEOUT_MS = 1000;
/** How long a disposed worker may take to hand its frames back before it is terminated. */
const DISPOSE_GRACE_MS = 1000;

type Timers = { set(fn: () => void, ms: number): number; clear(id: number): void };

export type PresentWorkerOptions = {
  worker: WorkerLike;
  canvas: OffscreenCanvas;
  /** The page's `performance.timeOrigin`. */
  timeOrigin?: number;
  /** The worker could not draw (no WebGPU in workers): present on the main thread instead. */
  onFailed?: (message: string) => void;
  timers?: Timers;
};

export class PresentWorker {
  private sessions = new Map<number, WorkerPresenter>();
  private nextSession = 1;
  private disposed = false;
  private failure: string | null = null;
  private readonly timers: Timers;

  constructor(private readonly options: PresentWorkerOptions) {
    this.timers = options.timers ?? { set: (fn, ms) => setTimeout(fn, ms) as unknown as number, clear: (id) => clearTimeout(id) };
    options.worker.addEventListener('message', (event) => this.onMessage(event.data));
    options.worker.addEventListener('error', (event) => this.fail(`presentation worker error: ${(event as ErrorEvent).message ?? 'unknown'}`));
    options.worker.postMessage({ type: 'init', canvas: options.canvas, pageTimeOrigin: options.timeOrigin ?? performance.timeOrigin }, [options.canvas]);
  }

  /** Why the worker cannot present, once it said so. */
  get failed(): string | null {
    return this.failure;
  }

  post(message: ToWorker, transfer?: Transferable[]): void {
    if (this.disposed) return;
    this.options.worker.postMessage(message, transfer);
  }

  /** Opens a session for `link`; the worker's presenter starts at once. */
  open(options: Omit<WorkerPresenterOptions, 'host' | 'session'>): WorkerPresenter {
    const session = this.nextSession++;
    const presenter = new WorkerPresenter({ ...options, host: this, session });
    this.sessions.set(session, presenter);
    this.post({ type: 'open', session, displayHz: options.displayHz ?? null, trace: !!options.trace, generation: options.generation ?? 0 });
    return presenter;
  }

  /** The session ended on the page: the worker's presenter hands its frames back, then it is forgotten. */
  closed(session: number): void {
    this.post({ type: 'close', session });
  }

  private onMessage(message: FromWorker): void {
    switch (message.type) {
      case 'ready':
        return;
      case 'failed':
        this.fail(message.message);
        return;
      case 'disposed':
        this.options.worker.terminate();
        return;
      case 'closed':
        this.sessions.delete(message.session);
        return;
      default:
        this.sessions.get(message.session)?.fromWorker(message);
    }
  }

  private fail(message: string): void {
    if (this.failure) return;
    this.failure = message;
    this.options.onFailed?.(message);
  }

  /** Ends every session and the worker (its frames go back to the engine first). */
  dispose(): void {
    if (this.disposed) return;
    for (const presenter of this.sessions.values()) presenter.dispose();
    this.options.worker.postMessage({ type: 'dispose' });
    this.disposed = true;
    // The worker answers `disposed` and is terminated then; a stuck one is
    // terminated anyway.
    this.timers.set(() => this.options.worker.terminate(), DISPOSE_GRACE_MS);
  }
}

export type WorkerPresenterOptions = {
  host: PresentWorker;
  session: number;
  link: FrameLink;
  source: FrameSource;
  displayHz?: number | null;
  trace?: boolean;
  generation?: number;
  hostOffsetUs?: () => number | null;
  onVsync?: (hint: VsyncHint) => void;
  parkTimeoutMs?: number;
  now?: () => number;
  timers?: Timers;
};

export class WorkerPresenter implements Presenter {
  private generation: number;
  private parked: { desc: FrameDescriptor; since: number }[] = [];
  private parkTimer = 0;
  private readonly listeners = new Set<(frame: PresentedFrame) => void>();
  private readonly traces = new Map<number, (trace: TraceRecord | null) => void>();
  private nextTrace = 1;
  private disposed = false;
  private readonly unsubscribe: () => void;
  private readonly now: () => number;
  private readonly timers: Timers;
  /** Counted here: descriptors, parking, stale pools. Drawing and dropping: the worker's last stats. */
  private readonly local = { received: 0, parked: 0, parkTimeouts: 0, staleGeneration: 0, noImport: 0 };
  private remote: PresenterStats | null = null;

  constructor(private readonly options: WorkerPresenterOptions) {
    this.generation = options.generation ?? 0;
    this.now = options.now ?? (() => performance.now());
    this.timers = options.timers ?? { set: (fn, ms) => setTimeout(fn, ms) as unknown as number, clear: (id) => clearTimeout(id) };
    this.unsubscribe = options.source.onSurface((generation, poolIndex) => this.unpark(generation, poolIndex));
  }

  get isDisposed(): boolean {
    return this.disposed;
  }

  get stats(): PresenterStats {
    const r = this.remote;
    const dropped = { ...(r?.dropped ?? {}) };
    if (this.local.noImport) dropped['no-import'] = (dropped['no-import'] ?? 0) + this.local.noImport;
    return {
      received: this.local.received,
      drawn: r?.drawn ?? 0,
      dropped,
      released: (r?.released ?? 0) + this.local.noImport,
      parked: this.local.parked,
      parkTimeouts: this.local.parkTimeouts,
      staleGeneration: this.local.staleGeneration,
      maxQueued: r?.maxQueued ?? 0,
      generation: this.generation
    };
  }

  setViewerState(state: ViewerState): void {
    this.options.link.sendViewerState(state);
  }

  start(): void {}

  /** The worker's animation frames follow its canvas; only the display's rate changes here. */
  retarget(_requestFrame: (cb: (t: number) => void) => number, _cancelFrame: (id: number) => void, displayHz: number | null = null): void {
    if (this.disposed) return;
    this.options.host.post({ type: 'display', session: this.options.session, displayHz });
  }

  onPresented(listener: (frame: PresentedFrame) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  receive(message: EngineFrameMessage, arrivedMs = this.now()): void {
    if (this.disposed) return;
    const session = this.options.session;
    if (message.type === 'clock') {
      this.options.host.post({ type: 'clock', session, sample: message.sample, arrivedMs });
      if (this.options.hostOffsetUs) this.options.host.post({ type: 'hostOffset', session, us: this.options.hostOffsetUs() });
    } else if (message.type === 'surfacePool') {
      const pool = message.pool;
      if (pool.generation <= this.generation) return;
      const previous = this.generation;
      this.generation = pool.generation;
      this.parked = this.parked.filter((p) => p.desc.poolGeneration === pool.generation);
      this.options.host.post({ type: 'pool', session, pool });
      if (previous) this.options.source.retire(previous);
    } else {
      this.onFrame(message.frame, arrivedMs);
    }
  }

  private onFrame(desc: FrameDescriptor, arrivedMs: number): void {
    this.local.received++;
    if (desc.poolGeneration < this.generation) {
      this.local.staleGeneration++;
      return;
    }
    const frame = this.options.source.videoFrame(desc.poolGeneration, desc.poolIndex);
    if (!frame) {
      this.local.parked++;
      this.parked.push({ desc, since: arrivedMs });
      this.armParkTimer();
      return;
    }
    this.send(desc, frame, arrivedMs);
  }

  private send(desc: FrameDescriptor, frame: VideoFrame, arrivedMs: number): void {
    this.options.host.post({ type: 'frame', session: this.options.session, desc, frame, arrivedMs }, [frame as unknown as Transferable]);
  }

  private unpark(generation: number, poolIndex: number): void {
    const waiting = this.parked.filter((p) => p.desc.poolGeneration === generation && p.desc.poolIndex === poolIndex);
    if (!waiting.length) return;
    this.parked = this.parked.filter((p) => !waiting.includes(p));
    for (const { desc, since } of waiting) {
      const frame = this.options.source.videoFrame(generation, poolIndex);
      if (frame) this.send(desc, frame, since);
      else this.releaseUndrawn(desc);
    }
  }

  private armParkTimer(): void {
    if (this.parkTimer || this.disposed) return;
    const timeout = this.options.parkTimeoutMs ?? PARK_TIMEOUT_MS;
    this.parkTimer = this.timers.set(() => {
      this.parkTimer = 0;
      const nowMs = this.now();
      const expired = this.parked.filter((p) => nowMs - p.since > timeout);
      this.parked = this.parked.filter((p) => !expired.includes(p));
      for (const { desc } of expired) {
        this.local.parkTimeouts++;
        this.releaseUndrawn(desc);
      }
      if (this.parked.length) this.armParkTimer();
    }, Math.max(50, timeout / 4));
  }

  /** A descriptor whose import never came: nothing reads its surface. */
  private releaseUndrawn(desc: FrameDescriptor): void {
    this.local.noImport++;
    if (desc.poolGeneration < this.generation) return;
    this.options.link.release({ poolIndex: desc.poolIndex, seq: desc.seq });
  }

  /** A message from the worker for this session. */
  fromWorker(message: Exclude<FromWorker, { type: 'ready' | 'failed' | 'disposed' | 'closed' }>): void {
    switch (message.type) {
      case 'release':
        // Releases keep flowing after the page ended the session: frames the
        // worker held when it closed go back to the engine (as the main
        // thread presenter releases its queue on dispose).
        this.options.link.release(message.release);
        return;
      case 'presented':
        this.remote = message.stats;
        if (this.disposed) return;
        for (const listener of this.listeners) listener(message.frame);
        return;
      case 'vsync':
        if (!this.disposed) this.options.onVsync?.(message.hint);
        return;
      case 'stats':
        this.remote = message.stats;
        return;
      case 'trace': {
        const resolve = this.traces.get(message.id);
        this.traces.delete(message.id);
        resolve?.(message.trace);
        return;
      }
    }
  }

  takeTrace(): Promise<TraceRecord | null> {
    if (this.disposed) return Promise.resolve(null);
    const id = this.nextTrace++;
    return new Promise((resolve) => {
      this.traces.set(id, resolve);
      this.options.host.post({ type: 'takeTrace', session: this.options.session, id });
    });
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.parkTimer) this.timers.clear(this.parkTimer);
    this.parkTimer = 0;
    for (const { desc } of this.parked) this.releaseUndrawn(desc);
    this.parked = [];
    this.unsubscribe();
    this.listeners.clear();
    for (const resolve of this.traces.values()) resolve(null);
    this.traces.clear();
    this.options.host.closed(this.options.session);
  }
}
