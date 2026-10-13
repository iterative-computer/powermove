import { describe, expect, it } from 'vitest';

import type { FramePainter } from './gpu';
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
  private pending: (() => void)[] = [];
  configure(format: string, colorSpace: string, width: number, height: number): void {
    this.configured.push(`${format}:${colorSpace}:${width}x${height}`);
  }
  draw(frame: VideoFrame): void {
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
      { seq: 1, revision: 4, viewSeq: 1, pts: 0, overlay: desc(1).overlay, rafMs: 16.7, timed: false }
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
    // A long stall: everything left is late.
    presenter.refresh(100 + 30 * (1000 / 60));
    expect(link.releases.length).toBeGreaterThan(0);
    expect(presenter.stats.dropped['late']).toBeGreaterThan(0);
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
});
