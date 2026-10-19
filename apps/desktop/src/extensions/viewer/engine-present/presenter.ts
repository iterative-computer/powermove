/*
 * Engine presentation in the viewer (plan section 18; amendments 4 and 5).
 *
 * The engine renders viewer frames into a pool of shared surfaces. Main
 * imports each surface once and sends it once to this window, where the
 * preload keeps it (`FrameSource`). Per-frame descriptors come from the
 * engine over this window's port (`FrameLink`), never through main. For
 * each descriptor the presenter asks the preload for a `VideoFrame` of the
 * named surface, queues it in the jitter buffer, and at the refresh that
 * should show it draws it, closes the `VideoFrame`, and releases the
 * surface to the engine once `queue.onSubmittedWorkDone()` resolves (the
 * per-use release callback runs before the GPU finished reading; S1).
 *
 * Overlays. Each drawn frame's revision, view sequence, and overlay
 * geometry go to `onPresented` listeners, so handles drawn from them match
 * the pixels on screen and never lead them during gestures.
 *
 * Until the kernel's engine connection hands the viewer a link and a frame
 * source (behind "Use New Engine"), nothing constructs a presenter in the
 * app; the bridge's integration test and the S1 harness do.
 */
import { decisionOffsetUs, JitterBuffer, type Queued } from './jitter-buffer';
import { HINT_INTERVAL_MS, hintChanged, nextVsync, VsyncEstimator, type VsyncFit } from './display-timing';
import { DeviceLostError, type FramePainter } from './gpu';
import type {
  ClockSample,
  EngineFrameMessage,
  FrameDescriptor,
  FrameRelease,
  OverlayGeometry,
  SurfacePool,
  ViewerState,
  VsyncHint
} from './protocol';

/** The engine connection as the presenter uses it. */
export interface FrameLink {
  sendViewerState(state: ViewerState): void;
  release(release: FrameRelease): void;
}

/** The preload's frame bridge: the imports it holds for this window. */
export interface FrameSource {
  /** A `VideoFrame` of the surface, or null when its import has not arrived. */
  videoFrame(generation: number, poolIndex: number): VideoFrame | null;
  /** Called when an import arrives. */
  onSurface(listener: (generation: number, poolIndex: number) => void): () => void;
  /** Releases the imports of a generation the engine replaced. */
  retire(generation: number): number;
}

export type PresentedFrame = {
  seq: number;
  revision: number;
  viewSeq: number;
  pts: number;
  overlay: OverlayGeometry | null;
  rafMs: number;
  timed: boolean;
  /** A scrub frame with stand-ins for video still decoding (the exact
   *  frame follows; docs/engine/scrub.md). */
  standIn: boolean;
  /** Host µs at which this refresh is predicted to reach the display (the
   *  instant the frame was picked for: the A/V sync measurement compares
   *  it with the audio clock); null for frames drawn on arrival or before
   *  the host clock is known. */
  displayUs: number | null;
  /** The frame's depth and object-id surface (`FrameDescriptor.depthId`),
   *  when the viewer asked for it and its pool arrived. */
  depthId?: { poolIndex: number; seq: number; poolGeneration: number } | null;
  /** The frame's size in pixels (the canvas takes it when the frame lands). */
  width?: number;
  height?: number;
};

export type PresenterOptions = {
  link: FrameLink;
  source: FrameSource;
  painter: FramePainter;
  /** The display's refresh rate when the host knows it. */
  displayHz?: number | null;
  /** Predicted rAF to display time, in refreshes (S1: one or two at 120 Hz). */
  latencyRefreshes?: number;
  /**
   * Added to `latencyRefreshes` while the window is being resized and for
   * `RESIZE_HOLD_MS` after its last step (`noteResize`): a live resize
   * holds the panel's presentation one to three refreshes longer
   * (docs/engine/perf.md, "A/V sync under load").
   */
  resizeLatencyRefreshes?: number;
  /** Added to `latencyRefreshes`: where in its refresh a frame's
   *  presentation lands after the BeginFrame (ms; measured, see
   *  present-worker.ts). */
  presentOffsetMs?: number;
  /**
   * Shows the frame whose interval contains the display instant rounded to
   * the nearest refresh (decides half a refresh later), so a frame reaches
   * the panel within half a refresh of its time on the audio clock's line
   * either way; otherwise frames show from the first refresh at or after
   * their start (up to a refresh late).
   */
  nearestRefresh?: boolean;
  /** How long a descriptor may wait for its surface's import. */
  parkTimeoutMs?: number;
  /** performance.now() */
  now?: () => number;
  /** Records per-frame timings (the S1 harness). */
  trace?: boolean;
  /**
   * performance.now() ms to host µs, when the host knows it (a ping to
   * main). Without it the offset is bounded from clock samples, which the
   * presenter's video clock stamps when they are sent.
   */
  hostOffsetUs?: () => number | null;
  /**
   * The display's refresh grid on the host clock changed (amendment 18):
   * the presentation re-sends its viewer state with the hint, so the
   * engine starts timed frames just after the compositor's GPU work.
   */
  onVsync?: (hint: VsyncHint) => void;
  /**
   * The pool generation the session already uses (a presenter that takes
   * over a running session), so the next pool retires it.
   */
  generation?: number;
  /**
   * Draw frames shown at the next refresh (paused, scrub, gesture) as soon
   * as they arrive instead of at the next animation frame. Only for a
   * canvas whose frames go to the compositor on their own (a worker's
   * `OffscreenCanvas`): a page canvas only commits with the page's next
   * frame anyway.
   */
  drawOnArrival?: boolean;
};

type Payload = { frame: VideoFrame };

export type PresenterStats = {
  received: number;
  drawn: number;
  dropped: Record<string, number>;
  released: number;
  parked: number;
  parkTimeouts: number;
  staleGeneration: number;
  maxQueued: number;
  generation: number;
};

export type TraceRecord = {
  arrivals: [seq: number, pts: number, arrivedMs: number, viewSeq: number, timed: number][];
  draws: [rafMs: number, seq: number, revision: number, pts: number, arrivedMs: number, submitMs: number, viewSeq: number, timed: number][];
  drops: [seq: number, reason: string, ms: number][];
  releases: [seq: number, submitMs: number, doneMs: number][];
  rafs: number[];
};

const PARK_TIMEOUT_MS = 1000;
/** How long after a live resize step its presentation delay is assumed. */
export const RESIZE_HOLD_MS = 150;

/**
 * What the viewer's presentation needs from a presenter: `EnginePresenter`
 * on the page's main thread, or `WorkerPresenter` (worker-presenter.ts),
 * which runs one in a worker.
 */
export interface Presenter {
  readonly stats: PresenterStats;
  setViewerState(state: ViewerState): void;
  receive(message: EngineFrameMessage, arrivedMs?: number): void;
  onPresented(listener: (frame: PresentedFrame) => void): () => void;
  /** A drawn frame's GPU work finished (its canvas frame is ready to show). */
  onDone?(listener: (seq: number) => void): () => void;
  /** Moves the loop to another window's animation frames (main thread only). */
  retarget(requestFrame: (cb: (t: number) => void) => number, cancelFrame: (id: number) => void, displayHz?: number | null): void;
  start(): void;
  /** The per-frame trace recorded since the last call (S1 harness), emptied. */
  takeTrace(): Promise<TraceRecord | null>;
  /** The viewer's window is being resized now (a live resize step). */
  noteResize?(): void;
  dispose(): void;
}

export class EnginePresenter implements Presenter {
  private readonly buffer = new JitterBuffer<Payload>();
  private vsync: VsyncEstimator;
  private cancelFrame: (id: number) => void = (id) => cancelAnimationFrame(id);
  /** Bumped when the loop moves, so a stale loop's callbacks stop. */
  private loop = 0;
  private clock: ClockSample | null = null;
  /** performance.now() ms → host µs: host = ms × 1000 + offset. */
  private hostOffsetUs: number | null = null;
  /** The estimate came from a run's first sample only (see `onClock`). */
  private provisionalOffset = false;
  private generation = 0;
  private parked: { desc: FrameDescriptor; since: number }[] = [];
  private offsetUs = 0;
  private offsetKey = '';
  /** performance.now() of the last live resize step (`noteResize`). */
  private resizedAtMs: number | null = null;
  /** The last timed frame's duration (ticks), for the decision phase. */
  private lastFrameTicks: number | null = null;
  private listeners = new Set<(frame: PresentedFrame) => void>();
  private hint: VsyncHint | null = null;
  private hintSentMs = -Infinity;
  private raf = 0;
  private disposed = false;
  private unsubscribe: () => void;
  private readonly now: () => number;
  readonly stats: PresenterStats = {
    received: 0,
    drawn: 0,
    dropped: {},
    released: 0,
    parked: 0,
    parkTimeouts: 0,
    staleGeneration: 0,
    maxQueued: 0,
    generation: 0
  };
  readonly trace: TraceRecord | null;

  constructor(private readonly options: PresenterOptions) {
    this.vsync = new VsyncEstimator(options.displayHz ?? null);
    this.now = options.now ?? (() => performance.now());
    this.trace = options.trace ? { arrivals: [], draws: [], drops: [], releases: [], rafs: [] } : null;
    this.generation = options.generation ?? 0;
    this.stats.generation = this.generation;
    this.unsubscribe = options.source.onSurface((generation, poolIndex) => this.unpark(generation, poolIndex));
  }

  /** Starts presenting at every animation frame. */
  start(
    requestFrame: (cb: (t: number) => void) => number = (cb) => requestAnimationFrame(cb),
    cancelFrame: (id: number) => void = (id) => cancelAnimationFrame(id)
  ): void {
    if (this.raf) this.cancelFrame(this.raf);
    this.raf = 0;
    this.cancelFrame = cancelFrame;
    const token = ++this.loop;
    const loop = (t: number): void => {
      if (this.disposed || token !== this.loop) return;
      this.raf = requestFrame(loop);
      this.refresh(t);
    };
    this.raf = requestFrame(loop);
  }

  /**
   * Moves the loop to another window's animation frames (the viewer moved
   * to a popout, or back): a different display has its own refresh grid,
   * so the timing fit starts over. Queued frames stay queued.
   */
  retarget(
    requestFrame: (cb: (t: number) => void) => number,
    cancelFrame: (id: number) => void,
    displayHz: number | null = null
  ): void {
    if (this.disposed) return;
    this.vsync = new VsyncEstimator(displayHz);
    this.offsetKey = '';
    this.offsetUs = 0;
    // Another display: its grid replaces the old one as soon as it is fitted.
    this.hint = null;
    this.hintSentMs = -Infinity;
    this.start(requestFrame, cancelFrame);
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

  private done(seq: number): void {
    if (this.disposed) return;
    for (const listener of this.doneListeners) listener(seq);
  }

  /** Sends a viewer state (the caller numbers it, see `ViewerSequencer`). */
  setViewerState(state: ViewerState): void {
    this.options.link.sendViewerState(state);
  }

  /** One message from the engine connection. */
  receive(message: EngineFrameMessage, arrivedMs = this.now()): void {
    if (this.disposed) return;
    if (message.type === 'clock') this.onClock(message.sample, arrivedMs);
    else if (message.type === 'surfacePool') this.onPool(message.pool);
    else this.onFrame(message.frame, arrivedMs);
  }

  private onClock(sample: ClockSample, arrivedMs: number): void {
    if (this.clock && sample.seq <= this.clock.seq && sample.epoch === this.clock.epoch) return;
    // A run's first sample may name a host time still ahead (where the run
    // starts: the pre-roll or the audio output's latency), so it is no
    // bound: it serves only until any other sample arrives.
    const runStart = sample.state !== 'stopped' && sample.epoch !== this.clock?.epoch;
    this.clock = sample;
    // The sample was made at its host time and arrived here no earlier, so
    // host = ms × 1000 + offset gives offset ≥ hostTime − arrivedMs × 1000:
    // every sample is a lower bound and the largest is the tightest (the
    // one-way latency of the engine → renderer port, about 0.1 to 0.5 ms,
    // when the main thread was free). A sample that waited behind a long
    // task bounds it loosely and must not move it: taking the smallest
    // difference instead (as before) let one stalled sample put the
    // presenter's clock that far behind the engine's for the rest of the
    // session (measured in the real app: about 110 ms, so the jitter buffer
    // held twice its render-ahead and the engine ran out of free slots).
    const offset = sample.hostTime - arrivedMs * 1000;
    if (runStart) {
      if (this.hostOffsetUs === null) {
        this.hostOffsetUs = offset;
        this.provisionalOffset = true;
      }
      return;
    }
    if (this.hostOffsetUs === null || this.provisionalOffset || offset > this.hostOffsetUs) this.hostOffsetUs = offset;
    this.provisionalOffset = false;
  }

  /** Host µs (the engine's clock) for a performance.now() time. */
  hostUs(ms: number): number | null {
    const offset = this.options.hostOffsetUs?.() ?? this.hostOffsetUs;
    return offset === null ? null : ms * 1000 + offset;
  }

  /** The depth and object-id pool's generation (`role: depthId`). */
  private depthGeneration = 0;

  private onPool(pool: SurfacePool): void {
    if (pool.role === 'depthId') {
      // A depth pool never replaces the frames' pool: its surfaces are read
      // by the 3D chrome for the frame on screen.
      if (pool.generation <= this.depthGeneration) return;
      const previous = this.depthGeneration;
      this.depthGeneration = pool.generation;
      if (previous) this.options.source.retire(previous);
      return;
    }
    if (pool.generation <= this.generation) return;
    const previous = this.generation;
    this.generation = pool.generation;
    this.stats.generation = pool.generation;
    for (const item of this.buffer.clear((i) => i.desc.poolGeneration !== pool.generation)) this.dropItem(item, 'generation');
    this.parked = this.parked.filter((p) => p.desc.poolGeneration === pool.generation);
    if (previous) this.options.source.retire(previous);
  }

  private onFrame(desc: FrameDescriptor, arrivedMs: number): void {
    if (!this.accept(desc, arrivedMs)) return;
    const frame = this.options.source.videoFrame(desc.poolGeneration, desc.poolIndex);
    if (!frame) {
      // The import is still on its way from main.
      this.stats.parked++;
      this.parked.push({ desc, since: arrivedMs });
      return;
    }
    this.enqueue(desc, frame, arrivedMs);
  }

  /** Counts and traces an arriving descriptor; false when its pool is gone. */
  private accept(desc: FrameDescriptor, arrivedMs: number): boolean {
    this.stats.received++;
    this.trace?.arrivals.push([desc.seq, desc.pts, arrivedMs, desc.viewSeq, desc.duration === null ? 0 : 1]);
    if (desc.poolGeneration < this.generation) {
      // The engine already replaced that pool: its ledger is gone.
      this.stats.staleGeneration++;
      return false;
    }
    return true;
  }

  /**
   * A descriptor whose `VideoFrame` the caller already made (the
   * off-main-thread presenter: the page's main thread asks the preload for
   * it and transfers it here). The presenter owns `frame` from now on.
   */
  present(desc: FrameDescriptor, frame: VideoFrame, arrivedMs = this.now()): void {
    if (this.disposed) {
      frame.close();
      return;
    }
    if (!this.accept(desc, arrivedMs)) {
      frame.close();
      return;
    }
    this.enqueue(desc, frame, arrivedMs);
  }

  private unpark(generation: number, poolIndex: number): void {
    const waiting = this.parked.filter((p) => p.desc.poolGeneration === generation && p.desc.poolIndex === poolIndex);
    if (!waiting.length) return;
    this.parked = this.parked.filter((p) => !waiting.includes(p));
    for (const { desc, since } of waiting) {
      const frame = this.options.source.videoFrame(generation, poolIndex);
      if (frame) this.enqueue(desc, frame, since);
      else this.releaseDesc(desc, 'no-import');
    }
  }

  private enqueue(desc: FrameDescriptor, frame: VideoFrame, arrivedMs: number): void {
    if (desc.duration !== null) this.lastFrameTicks = desc.duration;
    const item: Queued<Payload> = { desc, payload: { frame }, arrivedMs };
    // A timed frame comes back from the buffer when a newer revision of the
    // same frame replaced it (an edit during playback), not as a drop.
    for (const old of this.buffer.push(item)) this.dropItem(old, old.desc.duration === null ? 'superseded' : 'replaced');
    this.stats.maxQueued = Math.max(this.stats.maxQueued, this.buffer.size);
    if (desc.duration === null && this.options.drawOnArrival && this.raf) {
      const show = this.buffer.takeImmediate();
      if (show) this.draw(show, this.now());
    }
  }

  /** One refresh: called at requestAnimationFrame with its timestamp. */
  refresh(rafMs: number): void {
    if (this.disposed) return;
    this.trace?.rafs.push(rafMs);
    const fit = this.vsync.sample(rafMs);
    this.updateHint(rafMs, fit);
    this.expireParked(rafMs);
    const { decisionMs, displayMs } = this.decision(rafMs, fit);
    const atUs = this.hostUs(decisionMs);
    if (atUs === null && this.clock && this.clock.state !== 'stopped') return;
    const { show, drop } = this.buffer.pick(atUs ?? 0, this.clock);
    for (const { item, reason } of drop) this.dropItem(item, reason);
    // The display instant itself, without the shift that puts frame
    // boundaries halfway between decisions.
    if (show) this.draw(show, rafMs, this.hostUs(displayMs));
  }

  /** A live resize step of the viewer's window (`resizeLatencyRefreshes`). */
  noteResize(): void {
    this.resizedAtMs = this.now();
  }

  takeTrace(): Promise<TraceRecord | null> {
    const t = this.trace;
    return Promise.resolve(t && { arrivals: t.arrivals.splice(0), draws: t.draws.splice(0), drops: t.drops.splice(0), releases: t.releases.splice(0), rafs: t.rafs.splice(0) });
  }

  /** The refresh grid on the host clock, for the engine (amendment 18). */
  get vsyncHint(): VsyncHint | null {
    return this.hint;
  }

  private updateHint(rafMs: number, fit: VsyncFit | null): void {
    if (!fit || !this.options.onVsync) return;
    const host = this.hostUs(fit.vsyncMs);
    if (host === null) return;
    const next: VsyncHint = { hostTime: Math.round(host), periodUs: fit.periodMs * 1000 };
    if (!hintChanged(this.hint, next)) return;
    if (this.hint && !fit.jumped && rafMs - this.hintSentMs < HINT_INTERVAL_MS) return;
    this.hint = next;
    this.hintSentMs = rafMs;
    this.options.onVsync(next);
  }

  /** The predicted display time of this refresh's frame (`displayMs`), and
   *  the instant the frame is picked for (`decisionMs`): half a refresh
   *  later with `nearestRefresh`, then moved so frame boundaries fall
   *  halfway between decisions. */
  private decision(rafMs: number, fit: VsyncFit | null): { decisionMs: number; displayMs: number } {
    const periodMs = fit?.periodMs ?? 1000 / 60;
    const resizing = this.resizedAtMs !== null && rafMs - this.resizedAtMs < RESIZE_HOLD_MS ? (this.options.resizeLatencyRefreshes ?? 0) : 0;
    const latency = ((this.options.latencyRefreshes ?? 1) + resizing) * periodMs + (this.options.presentOffsetMs ?? 0);
    const vsync = fit ? nextVsync(fit, rafMs - periodMs / 2) : rafMs;
    const displayMs = vsync + latency;
    const decision = displayMs + (this.options.nearestRefresh ? periodMs / 2 : 0);
    const clock = this.clock;
    const head = this.buffer.frameTicks() ?? this.lastFrameTicks;
    if (fit && clock && head) {
      const key = `${clock.epoch}:${fit.periodMs.toFixed(4)}:${fit.vsyncMs.toFixed(1)}:${fit.jumped}:${resizing}`;
      const host = this.hostUs(decision);
      if (key !== this.offsetKey && host !== null) {
        this.offsetKey = key;
        this.offsetUs = decisionOffsetUs(periodMs * 1000, host, clock, head);
      }
    }
    return { decisionMs: decision + this.offsetUs / 1000, displayMs };
  }

  private draw(item: Queued<Payload>, rafMs: number, displayUs: number | null = null): void {
    const { desc } = item;
    const { frame } = item.payload;
    try {
      this.options.painter.configure(desc.format, desc.colorSpace, frame.codedWidth, frame.codedHeight);
      this.options.painter.draw(frame);
    } catch (error) {
      frame.close();
      // A lost device draws nothing more: the frame goes back to the engine
      // and the presentation replaces the drawing (`FramePainter.lost`).
      if (error instanceof DeviceLostError) {
        this.releaseDesc(desc, 'device-lost');
        return;
      }
      this.releaseDesc(desc, 'draw-failed');
      throw error;
    }
    const submitMs = this.now();
    const width = frame.codedWidth, height = frame.codedHeight;
    frame.close();
    this.stats.drawn++;
    this.trace?.draws.push([rafMs, desc.seq, desc.revision, desc.pts, item.arrivedMs, submitMs, desc.viewSeq, desc.duration === null ? 0 : 1]);
    this.options.painter.workDone().then(
      () => {
        this.trace?.releases.push([desc.seq, submitMs, this.now()]);
        this.release(desc);
        this.done(desc.seq);
      },
      () => {
        this.release(desc);
        this.done(desc.seq);
      }
    );
    const presented: PresentedFrame = {
      seq: desc.seq,
      revision: desc.revision,
      viewSeq: desc.viewSeq,
      pts: desc.pts,
      overlay: desc.overlay,
      rafMs,
      timed: desc.duration !== null,
      standIn: desc.quality.standIn === true,
      displayUs,
      depthId: desc.depthId && desc.depthId.poolGeneration === this.depthGeneration ? { ...desc.depthId } : null,
      width,
      height
    };
    for (const listener of this.listeners) listener(presented);
  }

  private dropItem(item: Queued<Payload>, reason: string): void {
    item.payload.frame.close();
    this.releaseDesc(item.desc, reason);
  }

  /** Releases a frame that was never drawn: nothing on the GPU reads it. */
  private releaseDesc(desc: FrameDescriptor, reason: string): void {
    this.stats.dropped[reason] = (this.stats.dropped[reason] ?? 0) + 1;
    this.trace?.drops.push([desc.seq, reason, this.now()]);
    this.release(desc);
  }

  private release(desc: FrameDescriptor): void {
    if (this.disposed || desc.poolGeneration < this.generation) return;
    this.stats.released++;
    this.options.link.release({ poolIndex: desc.poolIndex, seq: desc.seq });
  }

  private expireParked(nowMs: number): void {
    const timeout = this.options.parkTimeoutMs ?? PARK_TIMEOUT_MS;
    const expired = this.parked.filter((p) => nowMs - p.since > timeout);
    if (!expired.length) return;
    this.parked = this.parked.filter((p) => !expired.includes(p));
    for (const { desc } of expired) {
      this.stats.parkTimeouts++;
      this.releaseDesc(desc, 'no-import');
    }
  }

  dispose(cancelFrame: (id: number) => void = this.cancelFrame): void {
    if (this.disposed) return;
    for (const item of this.buffer.clear()) this.dropItem(item, 'disposed');
    for (const { desc } of this.parked) this.releaseDesc(desc, 'disposed');
    this.parked = [];
    this.disposed = true;
    if (this.raf) cancelFrame(this.raf);
    this.unsubscribe();
    this.listeners.clear();
  }
}
