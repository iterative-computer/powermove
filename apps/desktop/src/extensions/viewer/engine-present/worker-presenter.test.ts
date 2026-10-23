import { describe, expect, it } from 'vitest';

import type { FramePainter } from './gpu';
import { servePresenter, type FromWorker, type ToWorker, type WorkerScope } from './present-worker';
import type { FrameLink, FrameSource, PresentedFrame } from './presenter';
import { TICKS_PER_SECOND, type ClockSample, type FrameDescriptor, type FrameRelease, type ViewerState, type VsyncHint } from './protocol';
import { PresentWorker, type WorkerLike } from './worker-presenter';

const F = TICKS_PER_SECOND / 60;

class FakeFrame {
  closed = false;
  /** Clones made of this frame (what the page transfers to the worker). */
  clones: FakeFrame[] = [];
  /** Whether every clone was closed when this frame closed. */
  closedLast: boolean | null = null;
  constructor(readonly id: string, readonly codedWidth = 640, readonly codedHeight = 360) {}
  clone(): FakeFrame {
    const clone = new FakeFrame(this.id, this.codedWidth, this.codedHeight);
    this.clones.push(clone);
    return clone;
  }
  close(): void {
    if (!this.closed) this.closedLast = this.clones.every((c) => c.closed);
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
  private loseDevice: (reason: string) => void = () => {};
  readonly lost = new Promise<string>((resolve) => (this.loseDevice = resolve));
  /** The device goes away (the GPU process restarted). */
  lose(reason: string): void {
    this.loseDevice(reason);
    // A real lost queue settles all outstanding onSubmittedWorkDone calls.
    this.finishGpu();
  }
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
  private nextTimer = 1;
  private timers = new Map<number, { at: number; run: () => void }>();
  private toPage: ((event: MessageEvent<FromWorker>) => void)[] = [];
  private errors: ((event: Event) => void)[] = [];
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
      timers: {
        set: (run, ms) => { const id = pair.nextTimer++; pair.timers.set(id, { at: pair.workerNow + ms, run }); return id; },
        clear: id => { pair.timers.delete(id); },
      },
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
      if (type === 'error') this.errors.push(listener as unknown as (event: Event) => void);
    }) as WorkerLike['addEventListener'],
    terminate: () => {
      this.terminated = true;
    }
  };

  /** An uncaught exception in the worker script. */
  crash(message: string): void {
    for (const l of this.errors) l(Object.assign(new Event('error'), { message }));
  }

  /** A message from the worker that reaches the page whatever state it is in. */
  late(message: FromWorker): void {
    for (const l of this.toPage) l({ data: message } as MessageEvent<FromWorker>);
  }

  /** Runs the worker's animation frames at worker time `t`. */
  frame(t: number): void {
    this.workerNow = t;
    const due = [...this.rafs.values()];
    this.rafs.clear();
    for (const cb of due) cb(t);
    // Commit-phase tasks fire between this BeginFrame and the next one.
    for (const [id, task] of [...this.timers]) {
      this.timers.delete(id);
      this.workerNow = task.at;
      task.run();
    }
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
  const losses: string[] = [];
  const host = new PresentWorker({ worker: pair.worker, canvas: {} as OffscreenCanvas, timeOrigin: 0, onFailed: (m) => failures.push(m), onLost: (m) => losses.push(m) });
  const source = new Source();
  const link = new Link();
  const hints: VsyncHint[] = [];
  const presenter = host.open({ link, source, trace: options.trace, displayHz: 120, onVsync: (h) => hints.push(h) });
  const presented: PresentedFrame[] = [];
  presenter.onPresented((f) => presented.push(f));
  return { pair, host, source, link, presenter, presented, failures, hints, losses };
}

describe('WorkerPresenter and the presentation worker', () => {
  it('sends each frame with its VideoFrame to the worker, which draws a next-refresh frame on arrival and releases it after the GPU', async () => {
    const { pair, source, link, presenter, presented } = setup();
    source.imports.add('1:1');
    presenter.receive({ type: 'surfacePool', pool: { generation: 1, size: { width: 640, height: 360 }, format: 'bgra8Unorm', colorSpace: 'srgb', count: 8 } } as never, 0);
    pair.workerNow = 10;
    presenter.receive({ type: 'frame', frame: desc(1) }, 5);
    await settle();
    // The frame crossed as a transfer, with the page's arrival time.
    const sent = pair.posted.find((m) => m.type === 'frame') as Extract<ToWorker, { type: 'frame' }>;
    expect(sent.arrivedMs).toBe(5);
    // A clone crossed; the page keeps its own reference until the worker is done.
    expect(sent.frame).toBe(source.frames[0]!.clones[0]);
    expect(source.frames[0]!.closed).toBe(false);
    // A worker canvas commits on its own: drawn without waiting for an animation frame.
    expect(pair.painter.draws).toHaveLength(1);
    // Worker times are on the page's clock: worker 10 ms is page 1010 ms.
    expect(presented.map((f) => [f.seq, f.rafMs])).toEqual([[1, 1010]]);
    expect(link.releases).toEqual([]);
    expect(presenter.stats.released).toBe(0);
    pair.painter.finishGpu();
    await settle();
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(source.frames[0]!.closedLast).toBe(true);
    expect(presenter.stats.drawn).toBe(1);
    expect(presenter.stats.received).toBe(1);
    // The worker's last stats went out with the draw, before the GPU let
    // go of the frame; the release itself still counts (the integration
    // test's "every drawn frame released" after a pause).
    expect(pair.fromWorker.filter((m) => m.type === 'stats' || m.type === 'presented').at(-1)).toMatchObject({ stats: { released: 0 } });
    expect(presenter.stats.released).toBe(1);
  });

  it('with a release port, the worker hands frames back to the engine itself and the page closes a slot\'s frame when the slot comes back', async () => {
    const pair = new Pair();
    const host = new PresentWorker({ worker: pair.worker, canvas: {} as OffscreenCanvas, timeOrigin: 0 });
    const source = new Source();
    const releasedOnPort: Uint8Array[] = [];
    const port = { postMessage: (bytes: Uint8Array) => releasedOnPort.push(bytes), close: () => {} };
    class DirectLink extends Link {
      releasePort() {
        return { port: port as unknown as MessagePort, session: 77 };
      }
    }
    const link = new DirectLink();
    const presenter = host.open({ link, source, displayHz: 120 });
    source.imports.add('1:1');
    presenter.receive({ type: 'surfacePool', pool: { generation: 1, size: { width: 640, height: 360 }, format: 'bgra8Unorm', colorSpace: 'srgb', count: 8 } } as never, 0);
    presenter.receive({ type: 'frame', frame: desc(1) }, 5);
    await settle();
    expect(pair.painter.draws).toHaveLength(1);
    pair.painter.finishGpu();
    await settle();
    // Straight to the engine, as the session's own event; nothing through the page.
    const { decodeEnvelope } = await import('../../../main/engine/protocol');
    const sent = releasedOnPort.map((bytes) => decodeEnvelope(bytes));
    expect(sent).toHaveLength(1);
    expect(sent[0]).toMatchObject({ session: 77, message: { kind: 'event', body: { type: 'releaseFrame', poolIndex: 1, seq: 1 } } });
    expect(link.releases).toEqual([]);
    expect(pair.fromWorker.some((m) => m.type === 'release')).toBe(false);
    // The page keeps its reference (the last one) until the slot's next frame.
    expect(source.frames[0]!.closed).toBe(false);
    presenter.receive({ type: 'frame', frame: desc(7) }, 6);
    expect(source.frames[0]!.closed).toBe(true);
    expect(source.frames[0]!.closedLast).toBe(true);
    await settle();
    // The session ends: once the GPU let go of them the worker hands back
    // what it held; the page only closes.
    presenter.dispose();
    await settle();
    pair.painter.finishGpu();
    await settle();
    expect(source.frames[1]!.closed).toBe(true);
    expect(link.releases).toEqual([]);
    expect(releasedOnPort.map((bytes) => decodeEnvelope(bytes).message)).toContainEqual({ kind: 'event', body: { type: 'releaseFrame', poolIndex: 1, seq: 7 } });
  });

  it('sends a frame\'s GPU completion only while the page waits on it, and the newest one when it starts waiting', async () => {
    const { pair, source, presenter } = setup();
    source.imports.add('1:1');
    source.imports.add('1:2');
    presenter.receive({ type: 'surfacePool', pool: { generation: 1, size: { width: 640, height: 360 }, format: 'bgra8Unorm', colorSpace: 'srgb', count: 8 } } as never, 0);
    presenter.receive({ type: 'frame', frame: desc(1) }, 5);
    await settle();
    pair.painter.finishGpu();
    await settle();
    // Nobody waits: no page task per frame.
    expect(pair.fromWorker.filter((m) => m.type === 'done')).toEqual([]);
    const done: number[] = [];
    const off = presenter.onDone!((seq) => done.push(seq));
    await settle();
    // It finished before the page asked: the newest completion comes at once.
    expect(done).toEqual([1]);
    presenter.receive({ type: 'frame', frame: desc(2) }, 6);
    await settle();
    pair.painter.finishGpu();
    await settle();
    expect(done).toEqual([1, 2]);
    off();
    await settle();
    const before = pair.fromWorker.filter((m) => m.type === 'done').length;
    presenter.receive({ type: 'frame', frame: desc(8) }, 7);
    await settle();
    pair.painter.finishGpu();
    await settle();
    expect(pair.fromWorker.filter((m) => m.type === 'done')).toHaveLength(before);
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
    // Picked for the instant a worker canvas's frame reaches the panel
    // (2 refreshes and 3.2 ms after its animation frame, rounded to the
    // nearest refresh; 60 Hz until the refresh grid is fitted): at the
    // first animation frame that is media 36.5 ms, so frames 10 (0 to
    // 16.7 ms) and 11 (16.7 to 33.3 ms) are dropped as late.
    expect(presented.map((f) => f.seq)).toEqual([12, 13]);
    expect(presenter.stats.dropped['late']).toBe(2);
    pair.painter.finishGpu();
    await settle();
    expect(link.releases.map((r) => r.seq)).toEqual([10, 11, 12, 13]);
    const trace = await presenter.takeTrace();
    expect(trace!.arrivals.map((a) => a[2])).toEqual([1000, 1000, 1000, 1000]);
    expect(trace!.rafs[0]).toBe(1000);
    expect(trace!.draws).toHaveLength(2);
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
    // The worker got a clone and closed it first: the page's reference is
    // the last, so Electron's release callback runs on the page's thread.
    expect(source.frames[0]!.clones).toHaveLength(1);
    expect(source.frames[0]!.closedLast).toBe(true);
    expect(pair.fromWorker.some((m) => m.type === 'closed')).toBe(true);
    host.dispose();
    await settle();
    expect(pair.painter.disposed).toBe(true);
    expect(pair.terminated).toBe(true);
  });

  it('keeps submitted surfaces until their GPU fence before acknowledging close or destroying the painter', async () => {
    const { pair, host, source, link, presenter } = setup();
    source.imports.add('1:1');
    presenter.receive({ type: 'frame', frame: desc(1) }, 0);
    await settle();
    expect(pair.painter.draws).toHaveLength(1);
    host.dispose();
    await settle();
    expect(link.releases).toEqual([]);
    expect(source.frames[0]!.closed).toBe(false);
    expect(pair.fromWorker.some(m => m.type === 'closed' || m.type === 'disposed')).toBe(false);
    expect(pair.painter.disposed).toBe(false);
    pair.painter.finishGpu();
    await settle();
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(source.frames[0]!.closedLast).toBe(true);
    expect(pair.fromWorker.filter(m => m.type === 'closed' || m.type === 'disposed').map(m => m.type)).toEqual(['closed', 'disposed']);
    expect(pair.painter.disposed).toBe(true);
    expect(pair.terminated).toBe(true);
  });

  it('returns held slots when a disposed worker must be terminated', () => {
    const pair = new Pair(), source = new Source(), link = new Link();
    // A stuck realm receives transfers but never responds to Close/Dispose.
    pair.worker.postMessage = () => {};
    let terminate!: () => void;
    const host = new PresentWorker({ worker: pair.worker, canvas: {} as OffscreenCanvas,
      timers: { set: fn => { terminate = fn; return 1; }, clear: () => {} } });
    const presenter = host.open({ source, link });
    source.imports.add('1:1');
    presenter.receive({ type: 'frame', frame: desc(1) }, 0);
    host.dispose();
    expect(link.releases).toEqual([]);
    terminate();
    expect(pair.terminated).toBe(true);
    expect(source.frames[0]!.closed).toBe(true);
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
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

  it('terminates a worker that threw, hands every frame it held back to the engine once, and posts nothing more to it', async () => {
    const { pair, host, source, link, presenter, failures } = setup();
    source.imports.add('1:1');
    source.imports.add('1:2');
    source.imports.add('1:3');
    presenter.receive({ type: 'clock', sample: clock({}) }, 0);
    // Frame 1 is drawn and handed back; 2 and 3 wait a minute ahead in the worker.
    presenter.receive({ type: 'frame', frame: desc(1) }, 0);
    presenter.receive({ type: 'frame', frame: desc(2, { pts: 3600 * F, duration: F }) }, 0);
    presenter.receive({ type: 'frame', frame: desc(3, { pts: 3601 * F, duration: F }) }, 0);
    await settle();
    pair.painter.finishGpu();
    await settle();
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }]);

    pair.crash('boom');
    expect(pair.terminated).toBe(true);
    expect(failures).toEqual(['presentation worker error: boom']);
    expect(link.releases).toEqual([{ poolIndex: 1, seq: 1 }, { poolIndex: 2, seq: 2 }, { poolIndex: 3, seq: 3 }]);
    expect(presenter.isDisposed).toBe(true);
    expect(presenter.stats.released).toBe(3);
    // A release the worker posted before it died is not sent twice.
    pair.late({ type: 'release', session: 1, release: { poolIndex: 2, seq: 2 } });
    expect(link.releases).toHaveLength(3);
    // Nothing more goes to the dead worker, dispose included.
    const posted = pair.posted.length;
    presenter.receive({ type: 'frame', frame: desc(4) }, 0);
    host.dispose();
    expect(pair.posted).toHaveLength(posted);
  });

  it('a lost device ends the worker presenting: every frame it held goes back to the engine, later ones too, and the page is told', async () => {
    const { pair, source, link, presenter, losses } = setup();
    for (const k of ['1:1', '1:2', '1:3']) source.imports.add(k);
    presenter.receive({ type: 'clock', sample: clock({}) }, 0);
    // Frame 1 is drawn (its GPU work never finishes: the device is gone);
    // frame 2 waits a minute ahead in the jitter buffer.
    presenter.receive({ type: 'frame', frame: desc(1) }, 0);
    presenter.receive({ type: 'frame', frame: desc(2, { pts: 3600 * F, duration: F }) }, 0);
    await settle();
    expect(pair.painter.draws).toHaveLength(1);
    pair.painter.lose('A valid external Instance reference no longer exists.');
    await settle();
    expect(losses).toEqual(['A valid external Instance reference no longer exists.']);
    // Both came back: the queued one immediately, the submitted one when
    // its lost queue settled (before the worker acknowledged closure).
    expect(link.releases.map((r) => r.seq).sort()).toEqual([1, 2]);
    expect(source.frames.every((f) => f.closed)).toBe(true);
    // A frame that arrives afterwards is handed back undrawn.
    presenter.receive({ type: 'frame', frame: desc(3) }, 0);
    await settle();
    expect(pair.painter.draws).toHaveLength(1);
    expect(link.releases.map((r) => r.seq).sort()).toEqual([1, 2, 3]);
  });
});
