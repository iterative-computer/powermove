import { describe, expect, it } from 'vitest';

import type { FramePainter } from './gpu';
import { servePresenter, type FromWorker, type ToWorker, type WorkerScope } from './present-worker';
import type { FrameLink, FrameSource, PresentedFrame } from './presenter';
import { TICKS_PER_SECOND, type ClockSample, type FrameDescriptor, type FrameRelease, type ViewerState, type VsyncHint } from './protocol';
import { PresentWorker, type WorkerLike } from './worker-presenter';

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
  disposed = false;
  private pending: (() => void)[] = [];
  configure(): void {}
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
  dispose(): void {
    this.disposed = true;
  }
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
    overlay: null,
    depthId: null,
    viewSeq: seq,
    duration: null,
    ...partial
  };
}

/**
 * A worker and its scope in one thread: messages cross on the macrotask
 * queue as they do between threads; the worker's animation frames and
 * clock are the test's. The worker's time origin is 1000 ms after the
 * page's, so worker times must be moved onto the page clock.
 */
class Pair {
  readonly painter = new Painter();
  readonly posted: ToWorker[] = [];
  readonly fromWorker: FromWorker[] = [];
  terminated = false;
  workerNow = 0;
  private readonly rafs = new Map<number, (t: number) => void>();
  private nextRaf = 1;
  private toPage: ((event: MessageEvent<FromWorker>) => void)[] = [];
  private toWorker: ((event: MessageEvent<ToWorker>) => void)[] = [];

  constructor(failPainter = false) {
    const pair = this;
    const scope: WorkerScope = {
      postMessage(message) {
        pair.fromWorker.push(message);
        setTimeout(() => {
          for (const l of pair.toPage) l({ data: message } as MessageEvent<FromWorker>);
        }, 0);
      },
      addEventListener(_type, listener) {
        pair.toWorker.push(listener);
      },
      requestAnimationFrame(cb) {
        pair.rafs.set(pair.nextRaf, cb);
        return pair.nextRaf++;
      },
      cancelAnimationFrame(id) {
        pair.rafs.delete(id);
      },
      performance: { now: () => pair.workerNow, timeOrigin: 1000 }
    };
    servePresenter(scope, {
      createPainter: async () => {
        if (failPainter) throw new Error('no WebGPU in workers');
        return pair.painter;
      }
    });
  }

  readonly worker: WorkerLike = {
    postMessage: (message: ToWorker) => {
      this.posted.push(message);
      setTimeout(() => {
        for (const l of this.toWorker) l({ data: message } as MessageEvent<ToWorker>);
      }, 0);
    },
    addEventListener: ((type: string, listener: (event: MessageEvent<FromWorker>) => void) => {
      if (type === 'message') this.toPage.push(listener);
    }) as WorkerLike['addEventListener'],
    terminate: () => {
      this.terminated = true;
    }
  };

  /** Runs the worker's animation frames at worker time `t`. */
  frame(t: number): void {
    this.workerNow = t;
    const due = [...this.rafs.values()];
    this.rafs.clear();
    for (const cb of due) cb(t);
  }
}

function clock(partial: Partial<ClockSample>): ClockSample {
  return { epoch: 1, seq: 1, state: 'playing', hostTime: 0, mediaTime: 0, rate: { num: 1, den: 1 }, outputLatencyUs: 0, sampleRate: 48_000, loopRange: null, ...partial };
}

const settle = async (): Promise<void> => {
  for (let i = 0; i < 4; i++) await new Promise((resolve) => setTimeout(resolve, 0));
};

function setup(options: { failPainter?: boolean; trace?: boolean } = {}) {
  const pair = new Pair(options.failPainter);
  const failures: string[] = [];
  const host = new PresentWorker({ worker: pair.worker, canvas: {} as OffscreenCanvas, timeOrigin: 0, onFailed: (m) => failures.push(m) });
  const source = new Source();
  const link = new Link();
  const hints: VsyncHint[] = [];
  const presenter = host.open({ link, source, trace: options.trace, displayHz: 120, onVsync: (h) => hints.push(h) });
  const presented: PresentedFrame[] = [];
  presenter.onPresented((f) => presented.push(f));
  return { pair, host, source, link, presenter, presented, failures, hints };
}

describe('WorkerPresenter and the presentation worker', () => {
  it('sends each frame with its VideoFrame to the worker, which draws it at its next animation frame and releases it after the GPU', async () => {
    const { pair, source, link, presenter, presented } = setup();
    source.imports.add('1:1');
    presenter.receive({ type: 'surfacePool', pool: { generation: 1, size: { width: 640, height: 360 }, format: 'bgra8Unorm', colorSpace: 'srgb', count: 8 } } as never, 0);
    presenter.receive({ type: 'frame', frame: desc(1) }, 5);
    await settle();
    // The frame crossed as a transfer, with the page's arrival time.
    const sent = pair.posted.find((m) => m.type === 'frame') as Extract<ToWorker, { type: 'frame' }>;
    expect(sent.arrivedMs).toBe(5);
    expect(sent.frame).toBe(source.frames[0]);
    pair.frame(10);
    expect(pair.painter.draws).toHaveLength(1);
    await settle();
    // Worker times are on the page's clock: worker 10 ms is page 1010 ms.
    expect(presented.map((f) => [f.seq, f.rafMs])).toEqual([[1, 1010]]);
    expect(link.releases).toEqual([]);
    pair.painter.finishGpu();
    await settle();
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(presenter.stats.drawn).toBe(1);
    expect(presenter.stats.received).toBe(1);
  });

  it('parks a descriptor on the page until its import arrives, and releases it undrawn after the park timeout', async () => {
    const pair = new Pair();
    const timers: (() => void)[] = [];
    const host = new PresentWorker({ worker: pair.worker, canvas: {} as OffscreenCanvas, timeOrigin: 0 });
    const source = new Source();
    const link = new Link();
    let now = 0;
    const presenter = host.open({ link, source, displayHz: 120, now: () => now, timers: { set: (fn) => timers.push(fn), clear: () => {} } });
    presenter.receive({ type: 'frame', frame: desc(2) }, 0);
    presenter.receive({ type: 'frame', frame: desc(3) }, 0);
    expect(presenter.stats.parked).toBe(2);
    source.arrive(1, 2);
    await settle();
    expect(pair.posted.filter((m) => m.type === 'frame').map((m) => (m as { desc: FrameDescriptor }).desc.seq)).toEqual([2]);
    now = 1500;
    timers.shift()!();
    expect(link.releases).toEqual([{ poolIndex: 3, seq: 3 }]);
    expect(presenter.stats.parkTimeouts).toBe(1);
    expect(presenter.stats.dropped['no-import']).toBe(1);
  });

  it('retires a replaced pool on the page, counts stale descriptors, and tells the worker', async () => {
    const { pair, source, presenter } = setup();
    const pool = (generation: number) => ({ type: 'surfacePool', pool: { generation, size: { width: 640, height: 360 }, format: 'bgra8Unorm', colorSpace: 'srgb', count: 8 } }) as never;
    presenter.receive(pool(1));
    presenter.receive(pool(2));
    presenter.receive(pool(2));
    expect(source.retired).toEqual([1]);
    presenter.receive({ type: 'frame', frame: desc(4, { poolGeneration: 1 }) });
    expect(presenter.stats.staleGeneration).toBe(1);
    expect(presenter.stats.generation).toBe(2);
    await settle();
    expect(pair.posted.filter((m) => m.type === 'pool')).toHaveLength(2);
  });

  it('plays timed frames on the worker clock mapped from the page arrivals of clock samples', async () => {
    const { pair, source, link, presenter, presented } = setup({ trace: true });
    for (let i = 0; i < 6; i++) source.imports.add(`1:${i}`);
    // Host µs = page ms × 1000 (offset 0): the sample arrived at page 1000 ms.
    const sample = clock({ hostTime: 1_000_000 });
    presenter.receive({ type: 'clock', sample }, 1000);
    for (let k = 0; k < 4; k++) presenter.receive({ type: 'frame', frame: desc(10 + k, { pts: k * F, duration: F }) }, 1000);
    await settle();
    // Worker frames every 8.33 ms from page 1000 ms (worker 0 ms).
    for (let i = 0; i < 8; i++) pair.frame(i * (1000 / 120));
    await settle();
    expect(presented.map((f) => f.seq)).toEqual([10, 11, 12, 13]);
    pair.painter.finishGpu();
    await settle();
    expect(link.releases.map((r) => r.seq)).toEqual([10, 11, 12, 13]);
    const trace = await presenter.takeTrace();
    expect(trace!.arrivals.map((a) => a[2])).toEqual([1000, 1000, 1000, 1000]);
    expect(trace!.rafs[0]).toBe(1000);
    expect(trace!.draws).toHaveLength(4);
  });

  it('hands queued frames back to the engine when the session ends, and forgets the session once the worker closed it', async () => {
    const { pair, host, source, link, presenter } = setup();
    source.imports.add('1:1');
    const sample = clock({});
    presenter.receive({ type: 'clock', sample }, 0);
    // A timed frame a minute ahead: it waits in the worker's jitter buffer.
    presenter.receive({ type: 'frame', frame: desc(1, { pts: 3600 * F, duration: F }) }, 0);
    await settle();
    pair.frame(1);
    expect(pair.painter.draws).toHaveLength(0);
    presenter.dispose();
    await settle();
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(source.frames[0]!.closed).toBe(true);
    expect(pair.fromWorker.some((m) => m.type === 'closed')).toBe(true);
    host.dispose();
    await settle();
    expect(pair.painter.disposed).toBe(true);
    expect(pair.terminated).toBe(true);
  });

  it('forwards the vsync hint fitted on the worker animation frames', async () => {
    const { pair, presenter, hints } = setup();
    const sample = clock({ state: 'stopped', rate: { num: 0, den: 1 } });
    presenter.receive({ type: 'clock', sample }, 0);
    await settle();
    for (let i = 0; i < 40; i++) pair.frame(i * (1000 / 120));
    await settle();
    expect(hints.length).toBeGreaterThan(0);
    expect(hints[0]!.periodUs).toBeCloseTo(8333.3, 0);
  });

  it('reports a worker that cannot draw, and hands back the frames sent to it', async () => {
    const { source, presenter, failures, pair, link } = setup({ failPainter: true });
    source.imports.add('1:1');
    presenter.receive({ type: 'frame', frame: desc(1) }, 0);
    await settle();
    expect(failures).toEqual(['no WebGPU in workers']);
    expect(source.frames[0]!.closed).toBe(true);
    expect(pair.painter.draws).toHaveLength(0);
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
  });
});
