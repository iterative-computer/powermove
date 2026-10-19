import { describe, expect, it } from 'vitest';

import { DeviceLostError, type FramePainter } from './gpu';
import { EnginePresenter, type FrameLink, type FrameSource, type PresentedFrame } from './presenter';
import { TICKS_PER_SECOND, type ClockSample, type FrameDescriptor, type FrameRelease, type ViewerState, type VsyncHint } from './protocol';

const F = TICKS_PER_SECOND / 60;

class FakeFrame {
  closed = false;
  constructor(readonly id: string, readonly codedWidth = 640, readonly codedHeight = 360) {}
  close(): void {
    this.closed = true;
  }
}

class Source implements FrameSource {
  imports = new Set<string>();
  frames: FakeFrame[] = [];
  retired: number[] = [];
  private listeners = new Set<(g: number, i: number) => void>();
  videoFrame(generation: number, poolIndex: number): VideoFrame | null {
    if (!this.imports.has(`${generation}:${poolIndex}`)) return null;
    const frame = new FakeFrame(`${generation}:${poolIndex}`);
    this.frames.push(frame);
    return frame as unknown as VideoFrame;
  }
  onSurface(listener: (g: number, i: number) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  retire(generation: number): number {
    this.retired.push(generation);
    return 1;
  }
  arrive(generation: number, poolIndex: number): void {
    this.imports.add(`${generation}:${poolIndex}`);
    for (const l of this.listeners) l(generation, poolIndex);
  }
}

class Painter implements FramePainter {
  draws: FakeFrame[] = [];
  configured: string[] = [];
  lostReason: string | null = null;
  private pending: (() => void)[] = [];
  configure(format: string, colorSpace: string, width: number, height: number): void {
    this.configured.push(`${format}:${colorSpace}:${width}x${height}`);
  }
  draw(frame: VideoFrame): void {
    if (this.lostReason !== null) throw new DeviceLostError(this.lostReason);
    const fake = frame as unknown as FakeFrame;
    expect(fake.closed).toBe(false);
    this.draws.push(fake);
  }
  workDone(): Promise<void> {
    return new Promise((resolve) => this.pending.push(resolve));
  }
  finishGpu(): void {
    for (const resolve of this.pending.splice(0)) resolve();
  }
  dispose(): void {}
}

class Link implements FrameLink {
  releases: FrameRelease[] = [];
  states: ViewerState[] = [];
  sendViewerState(state: ViewerState): void {
    this.states.push(state);
  }
  release(release: FrameRelease): void {
    this.releases.push(release);
  }
}

function desc(seq: number, partial: Partial<FrameDescriptor> = {}): FrameDescriptor {
  return {
    poolIndex: seq % 6,
    poolGeneration: 1,
    seq,
    pts: 0,
    revision: 4,
    colorSpace: 'srgb',
    format: 'bgra8Unorm',
    quality: { resolution: 'full', representation: 'original', draft3d: false, cached: false },
    overlay: { viewport: { x: 0, y: 0, width: 640, height: 360 }, layers: [], caret: null },
    depthId: null,
    viewSeq: seq,
    duration: null,
    ...partial
  };
}

function setup() {
  const source = new Source();
  const painter = new Painter();
  const link = new Link();
  let now = 0;
  const presenter = new EnginePresenter({ link, source, painter, displayHz: 60, now: () => now });
  presenter.receive({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } });
  return { source, painter, link, presenter, setNow: (t: number) => (now = t) };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

describe('EnginePresenter', () => {
  it('draws a paused frame at the next refresh and releases it after the GPU finished', async () => {
    const { source, painter, link, presenter } = setup();
    const presented: PresentedFrame[] = [];
    presenter.onPresented((p) => presented.push(p));
    source.arrive(1, 1);
    presenter.receive({ type: 'frame', frame: desc(1) });
    expect(painter.draws).toHaveLength(0);
    presenter.refresh(16.7);
    expect(painter.draws).toHaveLength(1);
    expect(painter.configured).toEqual(['bgra8Unorm:srgb:640x360']);
    expect(painter.draws[0]!.closed).toBe(true);
    await flush();
    expect(link.releases).toEqual([]);
    painter.finishGpu();
    await flush();
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(presented).toEqual([
      { seq: 1, revision: 4, viewSeq: 1, pts: 0, overlay: desc(1).overlay, rafMs: 16.7, timed: false, standIn: false, displayUs: null, depthId: null, width: 640, height: 360 }
    ]);
  });

  it('parks a frame until its import arrives and releases it if the import never comes', () => {
    const { source, painter, link, presenter } = setup();
    presenter.receive({ type: 'frame', frame: desc(1) }, 0);
    presenter.refresh(16);
    expect(painter.draws).toHaveLength(0);
    source.arrive(1, 1);
    presenter.refresh(32);
    expect(painter.draws).toHaveLength(1);
    presenter.receive({ type: 'frame', frame: desc(2) }, 40);
    presenter.refresh(1100);
    expect(link.releases).toEqual([{ poolIndex: 2, seq: 2 }]);
    expect(presenter.stats.parkTimeouts).toBe(1);
  });

  it('a newer paused frame supersedes one not yet drawn', () => {
    const { source, painter, link, presenter } = setup();
    source.arrive(1, 1);
    source.arrive(1, 2);
    presenter.receive({ type: 'frame', frame: desc(1) });
    presenter.receive({ type: 'frame', frame: desc(2) });
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    presenter.refresh(16);
    expect(painter.draws.map((f) => f.id)).toEqual(['1:2']);
  });

  it('presents timed frames by the clock and drops late ones', () => {
    const { source, painter, link, presenter, setNow } = setup();
    for (let i = 0; i < 6; i++) source.arrive(1, i);
    // The clock sample arrives 0.3 ms after its host time: performance.now()
    // 100 ms is host 1_000_000 µs (offset 900_000 µs, bounded by the arrival).
    const sample: ClockSample = {
      seq: 1,
      epoch: 2,
      state: 'playing',
      hostTime: 1_000_000,
      mediaTime: 0,
      rate: { num: 1, den: 1 },
      outputLatencyUs: 0,
      sampleRate: 48_000,
      loopRange: null
    };
    setNow(100.3);
    presenter.receive({ type: 'clock', sample }, 100.3);
    for (let k = 0; k < 5; k++) presenter.receive({ type: 'frame', frame: desc(k + 1, { pts: k * F, duration: F }) }, 101);
    // Refreshes at 60 Hz; the frame drawn at a refresh is predicted to show
    // one refresh later.
    const draws: string[] = [];
    const shown: PresentedFrame[] = [];
    presenter.onPresented((p) => shown.push(p));
    for (let i = 0; i < 4; i++) {
      presenter.refresh(100 + i * (1000 / 60));
      draws.push(painter.draws.map((f) => f.id).join(','));
    }
    // Every frame is drawn once, in order, one per refresh.
    expect(painter.draws.length).toBeGreaterThanOrEqual(3);
    const seqs = painter.draws.map((f) => Number(f.id.split(':')[1]));
    expect(seqs).toEqual([...seqs].sort((a, b) => a - b));
    expect(new Set(seqs).size).toBe(seqs.length);
    expect(link.releases).toEqual([]);
    // Each timed frame carries the display instant it was picked for: one
    // refresh after its animation frame, on the host clock (the offset the
    // sample bounds: 899.7 ms); the audio
    // clock's line at that instant falls inside the frame's interval.
    for (const p of shown) {
      expect(p.timed).toBe(true);
      expect(p.displayUs).toBeCloseTo((p.rafMs + 1000 / 60) * 1000 + 899_700, 3);
      const heard = ((p.displayUs! - sample.hostTime) / 1e6) * TICKS_PER_SECOND;
      expect(heard - p.pts).toBeGreaterThanOrEqual(-1);
      expect(heard - p.pts).toBeLessThan(F + 1);
    }
    // A long stall: everything left is late.
    presenter.refresh(100 + 30 * (1000 / 60));
    expect(link.releases.length).toBeGreaterThan(0);
    expect(presenter.stats.dropped['late']).toBeGreaterThan(0);
  });

  it('maps to host time with the promptest clock sample, never a stalled one', () => {
    const { presenter } = setup();
    // True mapping: performance.now() ms × 1000 + 900_000 = host µs.
    const sample = (seq: number, hostTime: number): ClockSample => ({
      seq,
      epoch: 2,
      state: 'playing',
      hostTime,
      mediaTime: 0,
      rate: { num: 1, den: 1 },
      outputLatencyUs: 0,
      sampleRate: 48_000,
      loopRange: null
    });
    // A run's first sample names where it starts, 160 ms ahead (the audio
    // output's latency): it serves only until another sample arrives.
    presenter.receive({ type: 'clock', sample: sample(1, 1_160_000) }, 100.3);
    expect(presenter.hostUs(200)).toBe(1_259_700);
    // Made at 100 ms, delivered 0.3 ms later.
    presenter.receive({ type: 'clock', sample: sample(2, 1_000_000) }, 100.3);
    expect(presenter.hostUs(200)).toBe(1_099_700);
    // Made at 350 ms, delivered after a 110 ms long task: it says nothing new.
    presenter.receive({ type: 'clock', sample: sample(3, 1_250_000) }, 460);
    expect(presenter.hostUs(200)).toBe(1_099_700);
    // Made at 600 ms, delivered 0.1 ms later: a tighter bound.
    presenter.receive({ type: 'clock', sample: sample(4, 1_500_000) }, 600.1);
    expect(presenter.hostUs(200)).toBe(1_099_900);
  });

  it('a new pool generation drops the old frames and retires the old imports', () => {
    const { source, link, presenter } = setup();
    source.arrive(1, 1);
    presenter.receive({ type: 'frame', frame: desc(1) });
    presenter.receive({ type: 'surfacePool', pool: { generation: 2, surfaces: [] } });
    expect(link.releases).toEqual([]);
    expect(source.retired).toEqual([1]);
    expect(source.frames[0]!.closed).toBe(true);
    // A descriptor of the retired generation still in flight is ignored.
    presenter.receive({ type: 'frame', frame: desc(2) });
    expect(presenter.stats.staleGeneration).toBe(1);
    expect(link.releases).toEqual([]);
  });

  it('a depth and object-id pool never replaces the frames, and tags the frames that name it', () => {
    const { source, link, presenter, painter } = setup();
    const presented: PresentedFrame[] = [];
    presenter.onPresented((p) => presented.push(p));
    presenter.receive({ type: 'surfacePool', pool: { generation: 2, surfaces: [], role: 'depthId' } });
    expect(source.retired).toEqual([]);
    source.arrive(1, 1);
    presenter.receive({ type: 'frame', frame: desc(1, { depthId: { poolIndex: 1, seq: 1, poolGeneration: 2 } }) });
    presenter.refresh(16.7);
    expect(painter.draws).toHaveLength(1);
    expect(presented[0]!.depthId).toEqual({ poolIndex: 1, seq: 1, poolGeneration: 2 });
    // A newer depth pool retires the older one's imports, not the frames'.
    presenter.receive({ type: 'surfacePool', pool: { generation: 3, surfaces: [], role: 'depthId' } });
    expect(source.retired).toEqual([2]);
    // A frame naming a depth pool that is gone carries no depth.
    source.arrive(1, 2);
    presenter.receive({ type: 'frame', frame: desc(2, { depthId: { poolIndex: 2, seq: 2, poolGeneration: 2 } }) });
    presenter.refresh(33.4);
    expect(presented[1]!.depthId).toBeNull();
    expect(link.releases).toEqual([]);
  });

  it('dispose releases everything it holds', () => {
    const { source, link, presenter } = setup();
    source.arrive(1, 1);
    presenter.receive({ type: 'frame', frame: desc(1) });
    presenter.receive({ type: 'frame', frame: desc(3) });
    presenter.dispose(() => {});
    expect(link.releases).toEqual([
      { poolIndex: 1, seq: 1 },
      { poolIndex: 3, seq: 3 }
    ]);
    presenter.receive({ type: 'frame', frame: desc(4) });
    expect(link.releases).toHaveLength(2);
  });

  it('tells the engine the refresh grid on the host clock once it is fitted, and again only when it moves', () => {
    const hints: VsyncHint[] = [];
    // host µs = performance.now() ms × 1000 + 5,000,000
    const presenter = new EnginePresenter({
      link: new Link(),
      source: new Source(),
      painter: new Painter(),
      displayHz: 120,
      hostOffsetUs: () => 5_000_000,
      onVsync: (hint) => hints.push(hint)
    });
    const period = 1000 / 120;
    let t = 1000.4;
    for (let i = 0; i < 30; i++, t += period) presenter.refresh(t);
    expect(hints).toHaveLength(1);
    expect(hints[0]!.periodUs).toBeCloseTo(8333.33, 1);
    // On the grid: hostTime is a refresh, 1000.4 ms + n periods.
    const phase = (((hints[0]!.hostTime - 5_000_000 - 1_000_400) % hints[0]!.periodUs) + hints[0]!.periodUs) % hints[0]!.periodUs;
    expect(Math.min(phase, hints[0]!.periodUs - phase)).toBeLessThan(1);
    // The same grid two seconds later: nothing new.
    for (let i = 0; i < 240; i++, t += period) presenter.refresh(t);
    expect(hints).toHaveLength(1);
    // The display link's phase jumps by half a refresh: a new hint at once.
    t += period / 2;
    for (let i = 0; i < 24; i++, t += period) presenter.refresh(t);
    expect(hints).toHaveLength(2);
    const moved = (((hints[1]!.hostTime - hints[0]!.hostTime) % hints[0]!.periodUs) + hints[0]!.periodUs) % hints[0]!.periodUs;
    expect(moved).toBeCloseTo(hints[0]!.periodUs / 2, -1);
    expect(presenter.vsyncHint).toEqual(hints[1]);
  });

  it('sends no hint without a host clock offset', () => {
    const hints: VsyncHint[] = [];
    const presenter = new EnginePresenter({ link: new Link(), source: new Source(), painter: new Painter(), displayHz: 120, onVsync: (hint) => hints.push(hint) });
    for (let i = 0, t = 0; i < 60; i++, t += 1000 / 120) presenter.refresh(t);
    expect(hints).toEqual([]);
  });
  it('draws next-refresh frames on arrival when asked (a worker canvas), but timed frames only at animation frames', () => {
    const source = new Source();
    const painter = new Painter();
    const link = new Link();
    let now = 100;
    const rafs: ((t: number) => void)[] = [];
    const presenter = new EnginePresenter({ link, source, painter, drawOnArrival: true, now: () => now });
    presenter.start((cb) => rafs.push(cb), () => {});
    for (let i = 0; i < 6; i++) source.imports.add(`1:${i}`);
    presenter.receive({ type: 'frame', frame: desc(1) }, 100);
    expect(painter.draws).toHaveLength(1);
    presenter.receive({ type: 'frame', frame: desc(2) }, 101);
    expect(painter.draws.map((f) => f.id)).toEqual(['1:1', '1:2']);
    // A timed frame waits for its refresh.
    const clock = { epoch: 1, seq: 1, state: 'playing', hostTime: 0, mediaTime: 0, rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48_000, loopRange: null } as ClockSample;
    presenter.receive({ type: 'clock', sample: clock }, 0);
    now = 102;
    presenter.receive({ type: 'frame', frame: desc(3, { pts: 0, duration: F }) }, 102);
    expect(painter.draws).toHaveLength(2);
    presenter.dispose();
  });

  it('releases a frame its lost device cannot draw, without throwing', () => {
    const { source, painter, link, presenter } = setup();
    source.arrive(1, 1);
    painter.lostReason = 'GPU process restarted';
    presenter.receive({ type: 'frame', frame: desc(1) });
    expect(() => presenter.refresh(16)).not.toThrow();
    expect(painter.draws).toHaveLength(0);
    expect(source.frames[0]!.closed).toBe(true);
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(presenter.stats.dropped['device-lost']).toBe(1);
  });

  it('picks the frame nearest the calibrated display instant (nearestRefresh, presentOffsetMs)', () => {
    const source = new Source();
    const painter = new Painter();
    const link = new Link();
    let now = 0;
    const presenter = new EnginePresenter({ link, source, painter, displayHz: 120, now: () => now, latencyRefreshes: 1.5, presentOffsetMs: 3.2, nearestRefresh: true });
    presenter.receive({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } });
    for (let i = 0; i < 6; i++) source.arrive(1, i);
    const P = 1000 / 120;
    const raf = (i: number): void => {
      now = i * P + 0.3;
      presenter.refresh(i * P);
      painter.finishGpu();
    };
    for (let i = 0; i < 30; i++) raf(i);
    // Host = ms × 1000 + 900_000; media 0 at page 300 ms; 25 fps frames
    // (40 ms: 4.8 refreshes, no decision shift).
    const sample: ClockSample = { seq: 1, epoch: 2, state: 'playing', hostTime: 1_200_000, mediaTime: 0, rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48_000, loopRange: null };
    presenter.receive({ type: 'clock', sample }, 300);
    const F25 = TICKS_PER_SECOND / 25;
    const shown: PresentedFrame[] = [];
    presenter.onPresented((f) => shown.push(f));
    let next = 0;
    for (let i = 30; i < 90; i++) {
      // Keep four frames queued ahead.
      while (next < 12 && presenter.stats.received - presenter.stats.drawn - Object.values(presenter.stats.dropped).reduce((a, b) => a + b, 0) < 4) {
        presenter.receive({ type: 'frame', frame: desc(next + 1, { poolIndex: next % 6, pts: next * F25, duration: F25 }) }, i * P);
        next++;
      }
      raf(i);
    }
    expect(shown.length).toBeGreaterThanOrEqual(5);
    for (const f of shown) {
      // Shown from the refresh nearest its start: the display instant is
      // within half a refresh of the frame's time on the line.
      const displayMs = (f.displayUs! - 900_000) / 1000;
      expect(Math.abs(displayMs - (300 + (f.pts / F25) * 40))).toBeLessThanOrEqual(P / 2 + 1e-6);
      // The instant is 1.5 refreshes and 3.2 ms after the animation frame's vsync.
      expect(displayMs - f.rafMs).toBeCloseTo(1.5 * P + 3.2, 6);
    }
  });
});
