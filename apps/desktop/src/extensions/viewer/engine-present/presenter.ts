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
import type { FramePainter } from './gpu';
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
};

export type PresenterOptions = {
  link: FrameLink;
  source: FrameSource;
  painter: FramePainter;
  /** The display's refresh rate when the host knows it. */
  displayHz?: number | null;
  /** Predicted rAF to display time, in refreshes (S1: one or two at 120 Hz). */
  latencyRefreshes?: number;
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

export class EnginePresenter {
  private readonly buffer = new JitterBuffer<Payload>();
  private vsync: VsyncEstimator;
  private cancelFrame: (id: number) => void = (id) => cancelAnimationFrame(id);
  /** Bumped when the loop moves, so a stale loop's callbacks stop. */
  private loop = 0;
  private clock: ClockSample | null = null;
  /** performance.now() ms → host µs: host = ms × 1000 + offset. */
  private hostOffsetUs: number | null = null;
  private generation = 0;
  private parked: { desc: FrameDescriptor; since: number }[] = [];
  private offsetUs = 0;
  private offsetKey = '';
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

  /** Sends a viewer state (the caller numbers it, see `ViewSequencer`). */
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
    this.clock = sample;
    // The sample was made at its host time and arrived here no earlier:
    // the smallest difference bounds the clock offset (one-way latency of
    // the engine → renderer port, about 0.1 to 0.5 ms).
    const offset = sample.hostTime - arrivedMs * 1000;
    if (this.hostOffsetUs === null || offset < this.hostOffsetUs) this.hostOffsetUs = offset;
  }

  /** Host µs (the engine's clock) for a performance.now() time. */
  hostUs(ms: number): number | null {
    const offset = this.options.hostOffsetUs?.() ?? this.hostOffsetUs;
    return offset === null ? null : ms * 1000 + offset;
  }

  private onPool(pool: SurfacePool): void {
    if (pool.generation <= this.generation) return;
    const previous = this.generation;
    this.generation = pool.generation;
    this.stats.generation = pool.generation;
    for (const item of this.buffer.clear((i) => i.desc.poolGeneration !== pool.generation)) this.dropItem(item, 'generation');
    this.parked = this.parked.filter((p) => p.desc.poolGeneration === pool.generation);
    if (previous) this.options.source.retire(previous);
  }

  private onFrame(desc: FrameDescriptor, arrivedMs: number): void {
    this.stats.received++;
    this.trace?.arrivals.push([desc.seq, desc.pts, arrivedMs, desc.viewSeq, desc.duration === null ? 0 : 1]);
    if (desc.poolGeneration < this.generation) {
      // The engine already replaced that pool: its ledger is gone.
      this.stats.staleGeneration++;
      return;
    }
    const frame = this.options.source.videoFrame(desc.poolGeneration, desc.poolIndex);
    if (!frame) {
      // The import is still on its way from main.
      this.stats.parked++;
      this.parked.push({ desc, since: arrivedMs });
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
    for (const old of this.buffer.push(item)) this.dropItem(old, 'superseded');
    this.stats.maxQueued = Math.max(this.stats.maxQueued, this.buffer.size);
  }

  /** One refresh: called at requestAnimationFrame with its timestamp. */
  refresh(rafMs: number): void {
    if (this.disposed) return;
    this.trace?.rafs.push(rafMs);
    const fit = this.vsync.sample(rafMs);
    this.updateHint(rafMs, fit);
    this.expireParked(rafMs);
    const decisionMs = this.decisionMs(rafMs, fit);
    const atUs = this.hostUs(decisionMs);
    if (atUs === null && this.clock && this.clock.state !== 'stopped') return;
    const { show, drop } = this.buffer.pick(atUs ?? 0, this.clock);
    for (const { item, reason } of drop) this.dropItem(item, reason);
    if (show) this.draw(show, rafMs);
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

  /** The predicted display time of this refresh's frame, moved so frame
   *  boundaries fall halfway between decisions. */
  private decisionMs(rafMs: number, fit: VsyncFit | null): number {
    const periodMs = fit?.periodMs ?? 1000 / 60;
    const latency = (this.options.latencyRefreshes ?? 1) * periodMs;
    const vsync = fit ? nextVsync(fit, rafMs - periodMs / 2) : rafMs;
    const decision = vsync + latency;
    const clock = this.clock;
    const head = this.buffer.frameTicks() ?? this.lastFrameTicks;
    if (fit && clock && head) {
      const key = `${clock.epoch}:${fit.periodMs.toFixed(4)}:${fit.vsyncMs.toFixed(1)}:${fit.jumped}`;
      const host = this.hostUs(decision);
      if (key !== this.offsetKey && host !== null) {
        this.offsetKey = key;
        this.offsetUs = decisionOffsetUs(periodMs * 1000, host, clock, head);
      }
    }
    return decision + this.offsetUs / 1000;
  }

  private draw(item: Queued<Payload>, rafMs: number): void {
    const { desc } = item;
    const { frame } = item.payload;
    try {
      this.options.painter.configure(desc.format, desc.colorSpace, frame.codedWidth, frame.codedHeight);
      this.options.painter.draw(frame);
    } catch (error) {
      frame.close();
      this.releaseDesc(desc, 'draw-failed');
      throw error;
    }
    const submitMs = this.now();
    frame.close();
    this.stats.drawn++;
    this.trace?.draws.push([rafMs, desc.seq, desc.revision, desc.pts, item.arrivedMs, submitMs, desc.viewSeq, desc.duration === null ? 0 : 1]);
    this.options.painter.workDone().then(
      () => {
        this.trace?.releases.push([desc.seq, submitMs, this.now()]);
        this.release(desc);
      },
      () => this.release(desc)
    );
    const presented: PresentedFrame = {
      seq: desc.seq,
      revision: desc.revision,
      viewSeq: desc.viewSeq,
      pts: desc.pts,
      overlay: desc.overlay,
      rafMs,
      timed: desc.duration !== null
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
