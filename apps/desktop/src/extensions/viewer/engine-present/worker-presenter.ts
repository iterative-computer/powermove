/*
 * The page's side of off-main-thread presentation (present-worker.ts).
 *
 * `PresentWorker` owns the worker and the `OffscreenCanvas` the viewer's
 * canvas handed it, for as long as that canvas exists. `WorkerPresenter`
 * is one viewer session on it and stands in for `EnginePresenter`: what
 * needs the page's main thread stays here (the preload's imports, parking a
 * descriptor until its import arrives, retiring a replaced pool's imports,
 * releases to the engine), and each timed (playback) descriptor goes to
 * the worker with its `VideoFrame` the moment it arrives, so a frame the
 * engine rendered ahead is already in the worker when the page's main
 * thread stalls.
 *
 * A frame shown on arrival (paused, scrub, gesture: `duration` null) goes
 * to the worker at the page's next animation frame instead, and is
 * announced as presented there (`onPresented`). The worker draws it as it
 * arrives, so its canvas commits in the refresh whose page update carries
 * what the page draws for it (the viewer's selection box and handles, the
 * `overlay` extensions draw in), as today's frame loop draws a frame and its
 * overlay in one animation frame. Sent at once and announced on the
 * worker's report, the overlay landed one refresh after the pixels at best,
 * and as many as the page's main thread was busy for at worst (measured
 * 90 to 400 ms while scrubbing a 258-layer project).
 */
import type { FromWorker, ToWorker, WorkerMemoryStats } from './present-worker';
import { presentedFrame, type FrameLink, type FrameSource, type Presenter, type PresenterMemory, type PresentedFrame, type PresenterStats, type TraceRecord } from './presenter';
import type { EngineFrameMessage, FrameDescriptor, FrameRelease, ViewerState, VsyncHint } from './protocol';

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
  /** The worker's device was lost (the GPU process restarted): replace the worker. */
  onLost?: (message: string) => void;
  timers?: Timers;
};

export class PresentWorker {
  private sessions = new Map<number, WorkerPresenter>();
  private nextSession = 1;
  private disposed = false;
  /** Terminated after an uncaught error: nothing more is posted to it. */
  private dead = false;
  private failure: string | null = null;
  /** The worker's device was lost: it draws nothing more. */
  private lostReason: string | null = null;
  private readonly timers: Timers;
  private readonly memoryRequests = new Map<number, { resolve: (stats: WorkerMemoryStats | null) => void; timer: number }>();
  private nextMemoryRequest = 1;

  constructor(private readonly options: PresentWorkerOptions) {
    this.timers = options.timers ?? { set: (fn, ms) => setTimeout(fn, ms) as unknown as number, clear: (id) => clearTimeout(id) };
    options.worker.addEventListener('message', (event) => this.onMessage(event.data));
    options.worker.addEventListener('error', (event) => this.crashed(`presentation worker error: ${(event as ErrorEvent).message ?? 'unknown'}`));
    options.worker.postMessage({ type: 'init', canvas: options.canvas, pageTimeOrigin: options.timeOrigin ?? performance.timeOrigin }, [options.canvas]);
  }

  /** Why the worker cannot present, once it said so. */
  get failed(): string | null {
    return this.failure;
  }

  /** Whether the worker's device was lost (frames sent now are handed back undrawn). */
  get lost(): boolean {
    return this.lostReason !== null;
  }

  post(message: ToWorker, transfer?: Transferable[]): void {
    if (this.disposed || this.dead) return;
    this.options.worker.postMessage(message, transfer);
  }

  /** Ask the retained worker for painter allocations only when diagnostics
   * are requested. A failed worker answers null without leaving a timer. */
  memoryStats(): Promise<WorkerMemoryStats | null> {
    if (this.disposed || this.dead) return Promise.resolve(null);
    const id = this.nextMemoryRequest++;
    return new Promise(resolve => {
      const timer = this.timers.set(() => {
        this.memoryRequests.delete(id);
        resolve(null);
      }, 1000);
      this.memoryRequests.set(id, { resolve, timer });
      this.post({ type: 'memoryStats', id });
    });
  }

  trim(): void {
    this.post({ type: 'trim' });
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
      case 'lost':
        if (this.lostReason !== null) return;
        this.lostReason = message.message;
        if (!this.disposed && !this.dead) this.options.onLost?.(message.message);
        return;
      case 'disposed':
        this.terminate();
        return;
      case 'memoryStats': {
        const request = this.memoryRequests.get(message.id);
        if (!request) return;
        this.memoryRequests.delete(message.id);
        this.timers.clear(request.timer);
        request.resolve(message.stats);
        return;
      }
      case 'closed':
        // Its releases came before this: what it still held (frames it drew
        // whose GPU work had not finished when it closed, or every frame of
        // a lost device) goes back to the engine too, or those slots stay
        // held for a session that continues on a replaced drawing.
        this.sessions.get(message.session)?.releaseHeld();
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

  /**
   * An uncaught error in the worker: it may be in any state, so it is
   * terminated (its realm closes the VideoFrames it held) and the frames
   * sent to it that it had not handed back go back to the engine from here,
   * so their pool slots do not wait for the engine to reclaim them. Then
   * the viewer replaces the drawing (`onFailed`).
   */
  private crashed(message: string): void {
    if (this.dead || this.disposed) return;
    this.dead = true;
    this.options.worker.terminate();
    for (const presenter of this.sessions.values()) presenter.abandon();
    this.sessions.clear();
    this.fail(message);
  }

  /** Ends every session and the worker (its frames go back to the engine first). */
  dispose(): void {
    if (this.disposed) return;
    for (const presenter of this.sessions.values()) presenter.dispose();
    this.disposed = true;
    if (this.dead) return;
    this.options.worker.postMessage({ type: 'dispose' });
    // The worker answers `disposed` and is terminated then; a stuck one is
    // terminated anyway.
    this.timers.set(() => this.terminate(), DISPOSE_GRACE_MS);
  }

  private terminate(): void {
    this.options.worker.terminate();
    for (const request of this.memoryRequests.values()) {
      this.timers.clear(request.timer);
      request.resolve(null);
    }
    this.memoryRequests.clear();
    // As on a worker crash, termination ends its GPU context. The viewer may
    // keep its engine session on a replacement drawing, so return ownership.
    for (const presenter of this.sessions.values()) presenter.releaseHeld();
    this.sessions.clear();
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
  /** The worker could not read a frame of pool `generation` (`PresenterOptions.onUnusable`). */
  onUnusable?: (generation: number, message: string) => void;
  /** The page's animation frames (the canvas's window): frames shown on
   *  arrival go to the worker and are announced at them. Without them, each
   *  goes at once and is announced when the worker reports it. */
  requestFrame?: (cb: (t: number) => void) => number;
  cancelFrame?: (id: number) => void;
  parkTimeoutMs?: number;
  now?: () => number;
  timers?: Timers;
};

export class WorkerPresenter implements Presenter {
  private generation: number;
  /** The depth and object-id pool's generation (`role: depthId`). */
  private depthGeneration = 0;
  private parked: { desc: FrameDescriptor; since: number }[] = [];
  private parkTimer = 0;
  private readonly listeners = new Set<(frame: PresentedFrame) => void>();
  private readonly traces = new Map<number, (trace: TraceRecord | null) => void>();
  private nextTrace = 1;
  private disposed = false;
  private readonly unsubscribe: () => void;
  private readonly now: () => number;
  private readonly timers: Timers;
  /**
   * Counted here: descriptors, parking, stale pools, and releases (every
   * release reaches the engine through this page, while the worker's own
   * count is only as new as its last stats, sent before the GPU finished
   * with the frame it just drew). Drawing and dropping: the worker's last
   * stats.
   */
  private readonly local = { received: 0, parked: 0, parkTimeouts: 0, staleGeneration: 0, noImport: 0, released: 0, superseded: 0 };
  /** The newest frame shown on arrival, waiting for the page's animation frame. */
  private arrival: { desc: FrameDescriptor; frame: VideoFrame; arrivedMs: number } | null = null;
  private arrivalFrame = 0;
  /** Frames announced at the page's animation frame (`showArrival`): the
   *  worker's report of each is not announced again. */
  private readonly announced = new Set<number>();
  /**
   * Frames in the worker that it has not handed back yet, by
   * `poolIndex:seq`, with the page's own reference to each.
   *
   * The worker gets a clone; the page keeps the frame it imported and
   * closes it only after the worker handed its clone back. Electron runs a
   * shared texture's release callback (a function of the preload's realm)
   * from the GPU sync-token signal of the thread that drops the last
   * reference: dropped in the worker, that callback ran on the worker's
   * thread outside its realm and V8 aborted the renderer ("Cannot create a
   * handle without a HandleScope"; measured 2 crashes in 6 playback runs
   * before this). Keeping the last reference here keeps it on the page's
   * thread.
   */
  private readonly inWorker = new Map<string, { release: FrameRelease; frame: VideoFrame }>();
  private remote: PresenterStats | null = null;
  private resolveDrain: () => void = () => {};
  private readonly drain = new Promise<void>((resolve) => { this.resolveDrain = resolve; });

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
    if (this.local.superseded) dropped['superseded'] = (dropped['superseded'] ?? 0) + this.local.superseded;
    return {
      received: this.local.received,
      drawn: r?.drawn ?? 0,
      dropped,
      released: this.local.released,
      parked: this.local.parked,
      parkTimeouts: this.local.parkTimeouts,
      staleGeneration: this.local.staleGeneration,
      maxQueued: r?.maxQueued ?? 0,
      generation: this.generation
    };
  }

  memoryStats(): PresenterMemory {
    const held = this.arrival ? 1 : 0;
    return { localVideoFrames: this.inWorker.size + held, workerCloneUpperBound: this.inWorker.size, queuedFrames: held, parkedDescriptors: this.parked.length, submittedFences: 0 };
  }

  setViewerState(state: ViewerState): void {
    this.options.link.sendViewerState(state);
  }

  noteResize(): void {
    if (this.disposed) return;
    this.options.host.post({ type: 'resize', session: this.options.session });
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

  private readonly doneListeners = new Set<(seq: number) => void>();

  onDone(listener: (seq: number) => void): () => void {
    this.doneListeners.add(listener);
    return () => this.doneListeners.delete(listener);
  }

  receive(message: EngineFrameMessage, arrivedMs = this.now()): void {
    if (this.disposed) return;
    const session = this.options.session;
    if (message.type === 'clock') {
      this.options.host.post({ type: 'clock', session, sample: message.sample, arrivedMs });
      if (this.options.hostOffsetUs) this.options.host.post({ type: 'hostOffset', session, us: this.options.hostOffsetUs() });
    } else if (message.type === 'surfacePool') {
      const pool = message.pool;
      if (pool.role === 'depthId') {
        // The depth and object-id pool: the worker's presenter tags frames
        // with it; the imports are read on this side (`depthFrame`).
        if (pool.generation <= this.depthGeneration) return;
        const previous = this.depthGeneration;
        this.depthGeneration = pool.generation;
        this.options.host.post({ type: 'pool', session, pool });
        if (previous) this.options.source.retire(previous);
        return;
      }
      if (pool.generation <= this.generation) return;
      const previous = this.generation;
      this.generation = pool.generation;
      this.parked = this.parked.filter((p) => {
        if (p.desc.poolGeneration === pool.generation) return true;
        this.options.link.release({ poolIndex: p.desc.poolIndex, seq: p.desc.seq });
        return false;
      });
      // The worker drops a frame of a replaced pool: it is never announced.
      if (this.arrival && this.arrival.desc.poolGeneration < pool.generation) {
        this.supersede(this.arrival.desc, this.arrival.frame);
        this.arrival = null;
      }
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
    if (this.options.host.lost) {
      // The worker can no longer draw: the slot goes straight back.
      this.local.released++;
      this.options.link.release({ poolIndex: desc.poolIndex, seq: desc.seq });
      return;
    }
    const frame = this.options.source.videoFrame(desc.poolGeneration, desc.poolIndex);
    if (!frame) {
      this.local.parked++;
      this.parked.push({ desc, since: arrivedMs });
      this.armParkTimer();
      return;
    }
    this.deliver(desc, frame, arrivedMs);
  }

  /** A frame for the worker: a timed one at once, one shown on arrival at the page's next animation frame. */
  private deliver(desc: FrameDescriptor, frame: VideoFrame, arrivedMs: number): void {
    const requestFrame = this.options.requestFrame;
    if (desc.duration !== null || !requestFrame) {
      this.send(desc, frame, arrivedMs);
      return;
    }
    // The newest wins, as in the worker's own buffer.
    const previous = this.arrival;
    if (previous && previous.desc.seq > desc.seq) {
      this.supersede(desc, frame);
      return;
    }
    if (previous) this.supersede(previous.desc, previous.frame);
    this.arrival = { desc, frame, arrivedMs };
    if (!this.arrivalFrame) this.arrivalFrame = requestFrame((t) => this.showArrival(t));
  }

  /** The page's animation frame: the frame shown on arrival goes to the worker, and is on screen with this update. */
  private showArrival(rafMs: number): void {
    this.arrivalFrame = 0;
    const held = this.arrival;
    this.arrival = null;
    if (!held || this.disposed) return;
    const { desc, frame, arrivedMs } = held;
    if (this.options.host.lost) {
      // The worker can no longer draw: the slot goes straight back.
      this.supersede(desc, frame);
      return;
    }
    const presented = presentedFrame(desc, { width: frame.codedWidth, height: frame.codedHeight }, rafMs, null, this.depthGeneration);
    if (!this.send(desc, frame, arrivedMs)) return;
    this.announced.add(desc.seq);
    for (const listener of this.listeners) listener(presented);
  }

  /** A frame shown on arrival that a newer one replaced before it was sent: nothing reads it. */
  private supersede(desc: FrameDescriptor, frame: VideoFrame): void {
    frame.close();
    this.local.superseded++;
    this.local.released++;
    this.options.link.release({ poolIndex: desc.poolIndex, seq: desc.seq });
  }

  /** Whether the frame went to the worker. */
  private send(desc: FrameDescriptor, frame: VideoFrame, arrivedMs: number): boolean {
    const k = `${desc.poolIndex}:${desc.seq}`;
    this.inWorker.get(k)?.frame.close();
    let clone: VideoFrame;
    try {
      clone = frame.clone();
    } catch {
      // A frame that cannot be cloned (closed under us): nothing reads it.
      frame.close();
      this.releaseUndrawn(desc);
      return false;
    }
    this.inWorker.set(k, { release: { poolIndex: desc.poolIndex, seq: desc.seq }, frame });
    this.options.host.post({ type: 'frame', session: this.options.session, desc, frame: clone, arrivedMs }, [clone as unknown as Transferable]);
    return true;
  }

  private unpark(generation: number, poolIndex: number): void {
    const waiting = this.parked.filter((p) => p.desc.poolGeneration === generation && p.desc.poolIndex === poolIndex);
    if (!waiting.length) return;
    this.parked = this.parked.filter((p) => !waiting.includes(p));
    for (const { desc, since } of waiting) {
      const frame = this.options.source.videoFrame(generation, poolIndex);
      if (frame) this.deliver(desc, frame, since);
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
    this.local.released++;
    this.options.link.release({ poolIndex: desc.poolIndex, seq: desc.seq });
  }

  /** A message from the worker for this session. */
  fromWorker(message: Exclude<FromWorker, { type: 'ready' | 'failed' | 'lost' | 'disposed' | 'closed' }>): void {
    switch (message.type) {
      case 'release':
        // Releases keep flowing after the page ended the session: frames the
        // worker held when it closed go back to the engine (as the main
        // thread presenter releases its queue on dispose). A frame already
        // handed back for a crashed worker (`abandon`) is not released twice.
        {
          const k = `${message.release.poolIndex}:${message.release.seq}`;
          const held = this.inWorker.get(k);
          if (!held) return;
          this.inWorker.delete(k);
          // The worker closed its clone; this is the last reference.
          held.frame.close();
        }
        this.local.released++;
        this.options.link.release(message.release);
        return;
      case 'presented': {
        this.remote = message.stats;
        if (this.disposed) return;
        const seq = message.frame.seq;
        const early = this.announced.has(seq);
        // Frames announced before this one that the worker never drew are forgotten too.
        for (const announced of this.announced) if (announced <= seq) this.announced.delete(announced);
        if (early) return;
        for (const listener of this.listeners) listener(message.frame);
        return;
      }
      case 'done':
        if (this.disposed) return;
        for (const listener of this.doneListeners) listener(message.seq);
        return;
      case 'vsync':
        if (!this.disposed) this.options.onVsync?.(message.hint);
        return;
      case 'stats':
        this.remote = message.stats;
        return;
      case 'unusable':
        if (!this.disposed) this.options.onUnusable?.(message.generation, message.message);
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

  /**
   * The worker crashed: every frame it held goes back to the engine (it
   * will never release them), and the session ends.
   */
  abandon(): void {
    this.releaseHeld();
    this.dispose();
  }

  /** The worker closed this session: frames it never handed back go back to the engine. */
  releaseHeld(): void {
    for (const { release, frame } of this.inWorker.values()) {
      frame.close();
      this.local.released++;
      this.options.link.release(release);
    }
    this.inWorker.clear();
    // A worker sends `closed` only after its GPU fence. On a crash, the
    // worker is terminated before this release instead.
    this.resolveDrain();
  }

  drained(): Promise<void> {
    return this.drain;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.parkTimer) this.timers.clear(this.parkTimer);
    this.parkTimer = 0;
    if (this.arrivalFrame) this.options.cancelFrame?.(this.arrivalFrame);
    this.arrivalFrame = 0;
    if (this.arrival) {
      this.arrival.frame.close();
      this.local.released++;
      this.options.link.release({ poolIndex: this.arrival.desc.poolIndex, seq: this.arrival.desc.seq });
      this.arrival = null;
    }
    this.announced.clear();
    for (const { desc } of this.parked) this.releaseUndrawn(desc);
    this.parked = [];
    this.unsubscribe();
    this.listeners.clear();
    for (const resolve of this.traces.values()) resolve(null);
    this.traces.clear();
    this.options.host.closed(this.options.session);
  }
}
