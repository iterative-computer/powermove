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
  /** Messages from and to the worker (the page's tasks for it). */
  messagesIn = 0;
  messagesOut = 0;
  /** The same, by message type (diagnostics). */
  readonly byType: Record<string, number> = {};

  constructor(private readonly options: PresentWorkerOptions) {
    this.timers = options.timers ?? { set: (fn, ms) => setTimeout(fn, ms) as unknown as number, clear: (id) => clearTimeout(id) };
    options.worker.addEventListener('message', (event) => {
      this.messagesIn++;
      const type = `in:${String((event.data as { type?: unknown })?.type)}`;
      this.byType[type] = (this.byType[type] ?? 0) + 1;
      this.onMessage(event.data);
    });
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
    this.messagesOut++;
    const type = `out:${message.type}`;
    this.byType[type] = (this.byType[type] ?? 0) + 1;
    this.options.worker.postMessage(message, transfer);
  }

  /** Opens a session for `link`; the worker's presenter starts at once. */
  open(options: Omit<WorkerPresenterOptions, 'host' | 'session'>): WorkerPresenter {
    const session = this.nextSession++;
    // Opened before the presenter offers its release port (which names the session).
    this.post({ type: 'open', session, displayHz: options.displayHz ?? null, trace: !!options.trace, generation: options.generation ?? 0 });
    const presenter = new WorkerPresenter({ ...options, host: this, session });
    this.sessions.set(session, presenter);
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
  private readonly local = { received: 0, parked: 0, parkTimeouts: 0, staleGeneration: 0, noImport: 0, released: 0 };
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
  private readonly inWorker = new Map<string, { release: FrameRelease; frame: VideoFrame; paired?: boolean }>();
  private remote: PresenterStats | null = null;
  /**
   * The worker hands frames back to the engine itself (`releasePort`): this
   * thread hears of no release, so it keeps one reference per pool slot (the
   * frame the slot last showed) and closes it when the slot's next frame
   * arrives (the engine reuses a slot only after the worker closed its clone
   * and released it) or when the session ends.
   */
  private direct = false;

  constructor(private readonly options: WorkerPresenterOptions) {
    this.generation = options.generation ?? 0;
    this.now = options.now ?? (() => performance.now());
    this.timers = options.timers ?? { set: (fn, ms) => setTimeout(fn, ms) as unknown as number, clear: (id) => clearTimeout(id) };
    this.unsubscribe = options.source.onSurface((generation, poolIndex) => this.unpark(generation, poolIndex));
    this.offerReleasePort();
  }

  /**
   * Gives the worker a port to hand frames back on, once the link can make
   * one: a session opened before the engine numbered it gets it with its
   * first frame (frames before it went back through this thread).
   */
  private offerReleasePort(): void {
    if (this.direct || this.disposed) return;
    let offered: { port: MessagePort; session: number } | null = null;
    try {
      offered = this.options.link.releasePort?.() ?? null;
    } catch {
      offered = null;
    }
    if (!offered) return;
    this.direct = true;
    this.options.host.post({ type: 'releasePort', session: this.options.session, port: offered.port, engineSession: offered.session }, [offered.port as unknown as Transferable]);
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
      released: this.direct ? (r?.released ?? 0) : this.local.released,
      parked: this.local.parked,
      parkTimeouts: this.local.parkTimeouts,
      staleGeneration: this.local.staleGeneration,
      maxQueued: r?.maxQueued ?? 0,
      generation: this.generation,
      shownLate: r?.shownLate ?? 0,
      workerMessagesIn: this.options.host.messagesIn,
      workerMessagesOut: this.options.host.messagesOut,
      workerMessages: { ...this.options.host.byType },
      directRelease: this.direct
    };
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

  /** Whether the worker sends `done` (`watchDone`). */
  private watchingDone = false;

  /** The worker sends `done` only while someone listens (a reveal). It is
   *  told to stop at the first `done` nobody listened to, so a listener that
   *  comes and goes every frame costs no message of its own. */
  onDone(listener: (seq: number) => void): () => void {
    this.doneListeners.add(listener);
    if (!this.watchingDone && !this.disposed) {
      this.watchingDone = true;
      this.options.host.post({ type: 'watchDone', session: this.options.session, on: true });
    }
    return () => {
      this.doneListeners.delete(listener);
    };
  }

  receive(message: EngineFrameMessage, arrivedMs = this.now()): void {
    if (this.disposed) return;
    const session = this.options.session;
    if (message.type === 'clock') {
      // The host clock offset goes with the sample (one message).
      const hostOffsetUs = this.options.hostOffsetUs ? this.options.hostOffsetUs() : undefined;
      this.options.host.post({ type: 'clock', session, sample: message.sample, arrivedMs, ...(hostOffsetUs !== undefined ? { hostOffsetUs } : {}) });
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
      this.options.host.post({ type: 'pool', session, pool });
      if (previous) this.options.source.retire(previous);
    } else {
      this.onFrame(message.frame, arrivedMs);
    }
  }

  private onFrame(desc: FrameDescriptor, arrivedMs: number): void {
    this.local.received++;
    this.offerReleasePort();
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
    this.send(desc, frame, arrivedMs);
  }

  private send(desc: FrameDescriptor, frame: VideoFrame, arrivedMs: number): void {
    const k = this.direct ? `${desc.poolGeneration}:${desc.poolIndex}` : `${desc.poolIndex}:${desc.seq}`;
    this.inWorker.get(k)?.frame.close();
    if (this.direct) {
      // A frame of this slot sent before the worker had its release port:
      // the slot came back, so the worker handed it back.
      for (const [key, entry] of this.inWorker) {
        if (key !== k && entry.release.poolIndex === desc.poolIndex) {
          entry.frame.close();
          this.inWorker.delete(key);
        }
      }
    }
    let clone: VideoFrame;
    try {
      clone = frame.clone();
    } catch {
      // A frame that cannot be cloned (closed under us): nothing reads it.
      frame.close();
      this.releaseUndrawn(desc);
      return;
    }
    this.inWorker.set(k, { release: { poolIndex: desc.poolIndex, seq: desc.seq }, frame, paired: desc.depthId?.encoding === 'chromePlanesV1' });
    this.options.host.post({ type: 'frame', session: this.options.session, desc, frame: clone, arrivedMs }, [clone as unknown as Transferable]);
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
          let k = `${message.release.poolIndex}:${message.release.seq}`;
          if (this.direct) {
            // A frame for a session the worker no longer had (its release
            // port went with it): this thread hands it back.
            for (const [key, entry] of this.inWorker) if (entry.release.poolIndex === message.release.poolIndex && entry.release.seq === message.release.seq) k = key;
          }
          const held = this.inWorker.get(k);
          if (!held) return;
          this.inWorker.delete(k);
          // The worker closed its clone; this is the last reference.
          held.frame.close();
        }
        this.local.released++;
        this.options.link.release(message.release);
        return;
      case 'presented':
        this.remote = message.stats;
        if (this.disposed) return;
        for (const listener of this.listeners) listener(message.frame);
        return;
      case 'done':
        if (this.disposed) return;
        if (!this.doneListeners.size && this.watchingDone) {
          this.watchingDone = false;
          this.options.host.post({ type: 'watchDone', session: this.options.session, on: false });
          return;
        }
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
    // With a release port the worker may have handed some of these back
    // already: the engine ignores a release of a slot that moved on.
    for (const { release, frame } of this.inWorker.values()) {
      frame.close();
      this.local.released++;
      this.options.link.release(release);
    }
    this.inWorker.clear();
    this.dispose();
  }

  /** The worker closed this session: frames it never handed back go back to
   *  the engine (with a release port it handed every one back itself; the
   *  page's references close). */
  releaseHeld(): void {
    for (const { release, frame, paired } of this.inWorker.values()) {
      frame.close();
      // With a release port the worker handed back every frame but those
      // paired with chrome planes.
      if (this.direct && !paired) continue;
      this.local.released++;
      this.options.link.release(release);
    }
    this.inWorker.clear();
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
