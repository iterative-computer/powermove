// @vitest-environment happy-dom
import { Window as HappyWindow } from 'happy-dom';
import { describe, expect, it } from 'vitest';

import { FrameUnusableError, type FramePainter } from './gpu';
import { servePresenter, type FromWorker, type ToWorker } from './present-worker';
import type { WorkerLike } from './worker-presenter';
import type { FrameSource } from './presenter';
import { MAX_SURFACE_RENEWALS, overlayToCss, PRESENTATION_STATUS, ViewerPresentation, type EngineViewerChannel, type EngineViewerHandlers, type EngineViewerService, type ViewParams } from './presentation';
import type { FrameDescriptor, FrameRelease, OverlayGeometry, ViewerState } from './protocol';

class FakeFrame {
  closed = false;
  readonly codedWidth = 1280;
  readonly codedHeight = 720;
  clone(): FakeFrame {
    return new FakeFrame();
  }
  close(): void {
    this.closed = true;
  }
}

class Source implements FrameSource {
  private listeners = new Set<(g: number, i: number) => void>();
  readonly retired: number[] = [];
  launch = 1;
  // Custom sources may have unrelated stats. Retirement policy belongs to
  // the built-in channel, not the generic FrameSource contract.
  stats(): { customImportCount: number } { return { customImportCount: 0 }; }
  videoFrame(): VideoFrame | null {
    return new FakeFrame() as unknown as VideoFrame;
  }
  onSurface(listener: (g: number, i: number) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  retire(generation: number): number {
    this.retired.push(generation);
    return 0;
  }
}

class Painter implements FramePainter {
  draws = 0;
  disposed = false;
  trims = 0;
  /** Pool generations whose frames cannot be read (imports from before a GPU restart). */
  unreadable = new Set<number>();
  private generation = 0;
  private loseDevice: (reason: string) => void = () => {};
  readonly lost = new Promise<string>((resolve) => (this.loseDevice = resolve));
  /** The device goes away (the GPU process restarted). */
  lose(reason: string): void {
    this.loseDevice(reason);
  }
  configure(): void {}
  /** The next frame drawn comes from pool `generation`. */
  from(generation: number): void {
    this.generation = generation;
  }
  draw(): void {
    if (this.unreadable.has(this.generation)) throw new FrameUnusableError('VideoFrame RGBA16F upload failed (GL 0x501)');
    this.draws++;
  }
  workDone(): Promise<void> {
    return Promise.resolve();
  }
  trim(): void { this.trims++; }
  dispose(): void {
    this.disposed = true;
  }
}

class Channel implements EngineViewerChannel {
  states: ViewerState[] = [];
  releases: FrameRelease[] = [];
  closed = false;
  private closedLaunch: number | null = null;
  retireClosedPool?: (generation: number) => number;
  constructor(readonly handlers: EngineViewerHandlers, private readonly source: Source) {
    this.retireClosedPool = (generation) => this.closed && this.closedLaunch === this.source.launch ? this.source.retire(generation) : 0;
  }
  sendViewerState(state: ViewerState): void {
    this.states.push(state);
  }
  release(release: FrameRelease): void {
    this.releases.push(release);
  }
  closeOptions: Array<{ keepsDrawing?: boolean } | undefined> = [];
  close(options?: { keepsDrawing?: boolean }): void {
    this.closed = true;
    this.closedLaunch = this.source.launch;
    this.closeOptions.push(options);
  }
}

class Service implements EngineViewerService {
  readonly source = new Source();
  /** Frames run by hand: a shown frame counts at once (the reveal is tested on its own). */
  revealFrames = 0;
  connected = true;
  channels: Channel[] = [];
  private ready = new Set<() => void>();
  composition(): string {
    return 'comp-1';
  }
  open(handlers: EngineViewerHandlers): EngineViewerChannel | null {
    if (!this.connected) return null;
    const channel = new Channel(handlers, this.source);
    this.channels.push(channel);
    return channel;
  }
  onReady(listener: () => void): () => void {
    this.ready.add(listener);
    return () => this.ready.delete(listener);
  }
  fireReady(): void {
    for (const l of this.ready) l();
  }
}

const PARAMS: ViewParams = { zoom: 0.5, pan: [0, 0], resolution: 'automatic', media: 'automatic', playEveryFrame: false };

const OVERLAY: OverlayGeometry = {
  viewport: { x: 100, y: 50, width: 1000, height: 600 },
  layers: [
    {
      layer: 'a',
      world2d: [2, 0, 0, 0, 2, 0, 100, 50, 1],
      world3d: null,
      bounds: [[200, 100], [300, 100], [300, 200], [200, 200]],
      paths: [[{ op: 'moveTo', x: 10, y: 20 }, { op: 'cubicTo', x1: 2, y1: 4, x2: 6, y2: 8, x: 10, y: 12 }, { op: 'close' }]]
    }
  ],
  caret: { top: [40, 40], bottom: [40, 80] }
};

function desc(seq: number, viewSeq: number, poolGeneration = 1): FrameDescriptor {
  return {
    poolIndex: seq % 4,
    poolGeneration,
    seq,
    pts: 0,
    revision: 7,
    colorSpace: 'srgb',
    format: 'bgra8Unorm',
    quality: { resolution: 'full', representation: 'original', draft3d: false, cached: false },
    overlay: OVERLAY,
    depthId: null,
    viewSeq,
    duration: null
  };
}

/** A stage in a window whose animation frames the test runs by hand. */
function stageIn(win: Window, width = 640, height = 360): { stage: HTMLElement; overlay: HTMLElement; rafs: Map<number, (t: number) => void> } {
  const doc = win.document;
  const stage = doc.createElement('div');
  const overlay = doc.createElement('canvas');
  overlay.id = 'overlay';
  stage.appendChild(overlay);
  doc.body.appendChild(stage);
  stage.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: width, bottom: height, width, height, toJSON: () => ({}) }) as DOMRect;
  const rafs = new Map<number, (t: number) => void>();
  let next = 1;
  Object.assign(win, {
    devicePixelRatio: 2,
    requestAnimationFrame: (cb: (t: number) => void) => {
      rafs.set(next, cb);
      return next++;
    },
    cancelAnimationFrame: (id: number) => rafs.delete(id),
    matchMedia: (query: string) => ({ matches: query === '(dynamic-range: high)', addEventListener() {}, removeEventListener() {} })
  });
  return { stage, overlay, rafs };
}

function runFrames(rafs: Map<number, (t: number) => void>, t: number): void {
  const due = [...rafs.entries()];
  rafs.clear();
  for (const [, cb] of due) cb(t);
}

const settle = (): Promise<void> => new Promise((resolve) => setTimeout(resolve, 0));


/** A presentation worker run in this thread (messages cross on the task queue), drawing with `painter`. */
function inProcessWorker(painter: FramePainter | null): WorkerLike & { rafs: Map<number, (t: number) => void>; terminated: boolean; canvases: unknown[]; origins: number[] } {
  const toPage: ((event: MessageEvent<FromWorker>) => void)[] = [];
  const toWorker: ((event: MessageEvent<ToWorker>) => void)[] = [];
  const rafs = new Map<number, (t: number) => void>();
  let next = 1;
  const worker = {
    rafs,
    terminated: false,
    canvases: [] as unknown[],
    origins: [] as number[],
    postMessage(message: ToWorker) {
      if (message.type === 'init') {
        worker.canvases.push(message.canvas);
        worker.origins.push(message.pageTimeOrigin);
      }
      setTimeout(() => toWorker.forEach((l) => l({ data: message } as MessageEvent<ToWorker>)), 0);
    },
    addEventListener(type: string, listener: (event: MessageEvent<FromWorker>) => void) {
      if (type === 'message') toPage.push(listener);
    },
    terminate() {
      worker.terminated = true;
    }
  };
  servePresenter(
    {
      postMessage: (message) => setTimeout(() => toPage.forEach((l) => l({ data: message } as MessageEvent<FromWorker>)), 0),
      addEventListener: (_type, listener) => toWorker.push(listener),
      requestAnimationFrame: (cb) => (rafs.set(next, cb), next++),
      cancelAnimationFrame: (id) => rafs.delete(id),
      performance: { now: () => performance.now(), timeOrigin: performance.timeOrigin }
    },
    {
      createPainter: async () => {
        if (!painter) throw new Error('no WebGPU in workers');
        return painter;
      }
    }
  );
  return worker as never;
}

/** Lets `canvas.transferControlToOffscreen()` work in happy-dom. */
function offscreenCanvases(win: Window): { transferred: HTMLCanvasElement[] } {
  const record = { transferred: [] as HTMLCanvasElement[] };
  const proto = (win as unknown as { HTMLCanvasElement: { prototype: { transferControlToOffscreen?: () => unknown } } }).HTMLCanvasElement.prototype;
  proto.transferControlToOffscreen = function (this: HTMLCanvasElement) {
    record.transferred.push(this);
    return { offscreenFor: this.id };
  };
  return record;
}

const settleLong = async (): Promise<void> => {
  for (let i = 0; i < 6; i++) await settle();
};

describe('ViewerPresentation', () => {
  it('leaves a prepared drawing for the next viewer except while suspended on the final-tab home screen', async () => {
    const warms: string[] = [];
    const warmup = { warm: () => { warms.push('warm'); }, take: () => null } as unknown as NonNullable<EngineViewerService['drawingWarmup']>;
    const { stage, overlay } = stageIn(window as unknown as Window);
    // Disposed while presenting (extension reload, service replacement):
    // the channel close asks the connection for a new drawing.
    const replaced = Object.assign(new Service(), { drawingWarmup: warmup });
    const first = new ViewerPresentation({ service: replaced, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    first.dispose();
    await settle();
    expect(replaced.channels[0]!.closeOptions).toEqual([undefined]);
    expect(warms).toEqual([]);
    // Suspended for the home screen: the channel closes keeping the
    // drawing, so no second drawing is prepared while home.
    const home = Object.assign(new Service(), { drawingWarmup: warmup });
    const second = new ViewerPresentation({ service: home, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    second.suspend();
    await settle();
    expect(home.channels[0]!.closeOptions).toEqual([{ keepsDrawing: true }]);
    expect(warms).toEqual([]);
    // Disposed while suspended: its kept drawing goes, so it prepares one.
    second.dispose();
    await settle();
    expect(warms).toEqual(['warm']);
  });

  it('suspends only the viewer channel, trims after draining, and reopens a fresh surface epoch on the same drawing', async () => {
    const service = new Service();
    const painter = new Painter();
    const { stage, overlay } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => painter });
    await settle();
    presentation.update(PARAMS);
    const originalCanvas = presentation.canvas;
    const first = service.channels[0]!;
    first.handlers.message({ type: 'surfacePool', pool: { generation: 8, surfaces: [{ poolIndex: 0, size: { width: 1280, height: 720 }, format: 'rgba16Float' }] } }, performance.now());
    expect((await presentation.memoryStats()).announcedNominalBytes).toBe(1280 * 720 * 8);
    presentation.suspend();
    await settle();
    expect(first.closed).toBe(true);
    expect(service.source.retired).toEqual([8]);
    expect(painter.trims).toBe(1);
    expect(painter.disposed).toBe(false);
    expect(await presentation.memoryStats()).toMatchObject({ suspended: true, channelOpen: false, announcedNominalBytes: 0, pendingDrains: 0 });
    service.fireReady();
    await settle();
    expect(service.channels).toHaveLength(1);
    presentation.resume();
    await settle();
    expect(presentation.canvas).toBe(originalCanvas);
    expect(service.channels).toHaveLength(2);
    expect(service.channels[1]!.states[0]).toMatchObject({ surfaceEpoch: 1, viewSeq: 2 });
    presentation.dispose();
  });

  it('keeps a worker alive and trims only after the worker GPU fence drains', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    offscreenCanvases(window as unknown as Window);
    const painter = new Painter();
    let finish: () => void = () => {};
    painter.workDone = () => new Promise<void>(resolve => { finish = resolve; });
    const worker = inProcessWorker(painter);
    try {
      const presentation = new ViewerPresentation({ service, stage, before: overlay, createWorker: () => worker, createPainter: async () => new Painter() });
      await settleLong();
      presentation.update(PARAMS);
      const first = service.channels[0]!;
      first.handlers.message({ type: 'surfacePool', pool: { generation: 7, surfaces: [] } }, performance.now());
      first.handlers.message({ type: 'frame', frame: desc(1, 1, 7) }, performance.now());
      // Sent to the worker, and announced, at the page's animation frame.
      runFrames(rafs, performance.now());
      await settleLong();
      expect(painter.draws).toBe(1);
      presentation.suspend();
      await settleLong();
      expect(painter.trims).toBe(0);
      expect(worker.terminated).toBe(false);
      finish();
      await settleLong();
      expect(painter.trims).toBe(1);
      expect(service.source.retired).toEqual([7]);
      expect((await presentation.memoryStats()).worker?.sessionOpen).toBe(false);
      presentation.resume();
      await settleLong();
      expect(worker.terminated).toBe(false);
      expect(service.channels[1]!.states[0]).toMatchObject({ surfaceEpoch: 1 });
      presentation.dispose();
      await settleLong();
    } finally {
      delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
    }
  });
  it('retires both active imports only when the final viewer is disposed', async () => {
    const service = new Service();
    const { stage, overlay } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 11, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 12, surfaces: [], role: 'depthId' } }, performance.now());
    expect(service.source.retired).toEqual([]);
    presentation.dispose();
    await settle();
    expect(channel.closed).toBe(true);
    expect(service.source.retired).toEqual([11, 12]);
  });

  it('keeps the last frame through a restart and retires a reused generation only after its new channel closes', async () => {
    const service = new Service();
    Object.assign(service, { exclusive: true });
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 21, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(1, 1, 21) }, performance.now());
    runFrames(rafs, performance.now());
    await settle();
    expect(presentation.latest?.seq).toBe(1);
    channel.handlers.closed('engine restart');
    await settle();
    expect(presentation.latest?.seq).toBe(1);
    expect(service.source.retired).toEqual([]);
    service.fireReady();
    await settle();
    const next = service.channels[1]!;
    next.handlers.message({ type: 'surfacePool', pool: { generation: 21, surfaces: [] } }, performance.now());
    presentation.dispose();
    await settle();
    expect(service.source.retired).toEqual([21]);
  });

  it('defers final color and depth retirement until a chrome read releases', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 31, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 32, surfaces: [], role: 'depthId' } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: { ...desc(1, 1, 31), depthId: { poolIndex: 1, poolGeneration: 32, seq: 1, encoding: 'chromePlanesV1' } } }, performance.now());
    runFrames(rafs, performance.now());
    await settle();
    const release = presentation.retainChromeFrame(presentation.latest!);
    expect(release).not.toBeNull();
    presentation.dispose();
    await settle();
    expect(service.source.retired).toEqual([]);
    release?.();
    await settle();
    expect(service.source.retired).toEqual([31, 32]);
  });

  it('does not retire a reused pool generation after the engine relaunches while chrome still reads it', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 31, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: { ...desc(1, 1, 31), depthId: { poolIndex: 1, poolGeneration: 31, seq: 1, encoding: 'chromePlanesV1' } } }, performance.now());
    runFrames(rafs, performance.now());
    await settle();
    const release = presentation.retainChromeFrame(presentation.latest!);
    expect(release).not.toBeNull();
    presentation.dispose();
    await settle();
    expect(service.source.retired).toEqual([]);
    service.source.launch = 2; // Preload's new launch cleared old imports and accepted generation 31 again.
    release?.();
    await settle();
    expect(service.source.retired).toEqual([]);
  });

  it('does not retire a pool handed to a replacement viewer before old disposal', async () => {
    class HandoffService extends Service {
      private lease = 0;
      override open(handlers: EngineViewerHandlers): EngineViewerChannel | null {
        const channel = super.open(handlers) as Channel;
        const mine = ++this.lease;
        const close = channel.close.bind(channel);
        channel.close = () => {
          if (mine === this.lease) close();
        };
        return channel;
      }
    }
    const service = new HandoffService();
    const firstStage = stageIn(window as unknown as Window);
    const first = new ViewerPresentation({ service, stage: firstStage.stage, before: firstStage.overlay, createPainter: async () => new Painter() });
    await settle();
    service.channels[0]!.handlers.message({ type: 'surfacePool', pool: { generation: 41, surfaces: [] } }, performance.now());
    const nextStage = stageIn(window as unknown as Window);
    const next = new ViewerPresentation({ service, stage: nextStage.stage, before: nextStage.overlay, createPainter: async () => new Painter() });
    await settle();
    service.channels[1]!.handlers.message({ type: 'surfacePool', pool: { generation: 41, surfaces: [] } }, performance.now());
    first.dispose();
    await settle();
    expect(service.source.retired).toEqual([]);
    next.dispose();
    await settle();
    expect(service.source.retired).toEqual([41]);
  });

  it('leaves import ownership to a custom viewer channel that does not authorize retirement', async () => {
    const service = new Service();
    const { stage, overlay } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 51, surfaces: [] } }, performance.now());
    channel.retireClosedPool = undefined;
    (channel as EngineViewerChannel).close = () => { channel.closed = true; return 'custom closed'; };
    presentation.dispose();
    await settle();
    expect(channel.closed).toBe(true);
    expect(service.source.retired).toEqual([]);
  });

  it('waits for a worker GPU fence before retiring its final imported pool', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    offscreenCanvases(window as unknown as Window);
    const painter = new Painter();
    let finish: () => void = () => {};
    painter.workDone = () => new Promise<void>(resolve => { finish = resolve; });
    const worker = inProcessWorker(painter);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createWorker: () => worker, createPainter: async () => new Painter() });
    await settleLong();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 13, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(1, 1, 13) }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame.
    runFrames(rafs, performance.now());
    await settleLong();
    expect(painter.draws).toBe(1);
    presentation.dispose();
    await settleLong();
    expect(service.source.retired).toEqual([]);
    finish();
    await settleLong();
    expect(service.source.retired).toEqual([13]);
    expect(worker.terminated).toBe(true);
    delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
  });

  it('opens a session, sends numbered viewer states, and reveals the canvas at the first frame', async () => {
    const service = new Service();
    const painter = new Painter();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => painter });
    expect(presentation.canvas.nextSibling).toBe(overlay);
    expect(presentation.presenting).toBe(false);
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    expect(channel.states).toHaveLength(1);
    const first = channel.states[0]!;
    expect(first).toMatchObject({ composition: 'comp-1', size: { width: 1280, height: 720 }, devicePixelRatio: 2, edr: true, zoom: 0.5, viewSeq: 1 });
    presentation.update(PARAMS);
    expect(channel.states).toHaveLength(1);
    presentation.update({ ...PARAMS, pan: [10, 0] });
    expect(channel.states.map((s) => s.viewSeq)).toEqual([1, 2]);

    const presented: number[] = [];
    presentation.onPresented((frame) => presented.push(frame.seq));
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(1, 2) }, performance.now());
    runFrames(rafs, performance.now());
    expect(painter.draws).toBe(1);
    expect(presented).toEqual([1]);
    expect(presentation.presenting).toBe(true);
    expect(presentation.latest).toMatchObject({ seq: 1, revision: 7, viewSeq: 2 });
    // Overlay geometry converts with the DPR of the state the frame shows.
    expect(presentation.overlayInCss()?.viewport).toEqual({ x: 50, y: 25, width: 500, height: 300 });
    await settle();
    expect(channel.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    presentation.dispose();
    expect(channel.closed).toBe(true);
    expect(painter.disposed).toBe(true);
    expect(stage.contains(presentation.canvas)).toBe(false);
  });

  it.each([true, false])('keeps configured surface colors through capability flips (initial high gamut/EDR: %s) and later geometry changes', async (initialHigh) => {
    const service = new Service();
    const painter = new Painter();
    const win = window as unknown as Window;
    const { stage, overlay, rafs } = stageIn(win);
    const lists = new Map<string, { matches: boolean; listeners: Set<() => void> }>();
    Object.assign(win, {
      matchMedia: (query: string) => {
        let list = lists.get(query);
        if (!list) {
          list = { matches: query === '(dynamic-range: high)' || query === '(color-gamut: p3)' ? initialHigh : true, listeners: new Set() };
          lists.set(query, list);
        }
        return {
          get matches() { return list.matches; },
          addEventListener: (_type: string, listener: () => void) => list.listeners.add(listener),
          removeEventListener: (_type: string, listener: () => void) => list.listeners.delete(listener)
        };
      }
    });
    const flip = (query: string, matches: boolean): void => {
      const list = lists.get(query)!;
      list.matches = matches;
      for (const listener of list.listeners) listener();
    };
    const logs: [string, string][] = [];
    const presentation = new ViewerPresentation({ service, stage, before: overlay, sizeSettleMs: 0, createPainter: async () => painter, log: (level, message) => logs.push([level, message]) });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    const colors = { colorSpace: initialHigh ? 'displayP3' : 'srgb', edr: initialHigh };
    expect(channel.states[0]).toMatchObject({ ...colors, viewSeq: 1 });
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 7, surfaces: [] } }, performance.now());
    const frame = (seq: number, viewSeq: number): void => {
      channel.handlers.message({ type: 'frame', frame: { ...desc(seq, viewSeq), poolGeneration: 7 } }, performance.now());
      runFrames(rafs, performance.now());
    };
    frame(1, 1);

    // Each capability changes independently, as on first show or a display
    // transition. Neither asks the engine to reconfigure a playing pool.
    flip('(color-gamut: p3)', !initialHigh);
    frame(2, 1);
    flip('(dynamic-range: high)', !initialHigh);
    frame(3, 1);
    expect(channel.states).toHaveLength(1);
    expect(presentation.stats()).toMatchObject({ generation: 7, drawn: 3, dropped: {}, staleGeneration: 0 });
    expect(presentation.latest).toMatchObject({ seq: 3, viewSeq: 1 });
    expect(logs).toHaveLength(2);
    expect(logs.every(([level, message]) => level === 'info' && message.includes(`keeping configured ${colors.colorSpace}, edr=${colors.edr}`))).toBe(true);
    expect(logs[0]![1]).toContain(`changed to ${initialHigh ? 'srgb' : 'displayP3'}, edr=${initialHigh}`);
    expect(logs[1]![1]).toContain(`edr=${!initialHigh}; keeping`);

    // A later real DPR/size/zoom update must not pick up the observed
    // capability tuple that was deliberately excluded above.
    Object.assign(win, { devicePixelRatio: 1 });
    flip('(resolution: 2dppx)', false);
    expect(channel.states.at(-1)).toMatchObject({ ...colors, devicePixelRatio: 1, size: { width: 640, height: 360 }, viewSeq: 2 });
    const rect = stage.getBoundingClientRect();
    stage.getBoundingClientRect = () => ({ ...rect, width: 800, height: 450, right: 800, bottom: 450 });
    presentation.update({ ...PARAMS, zoom: 0.75, pan: [4, 8] }, { width: 800, height: 450 });
    expect(channel.states.at(-1)).toMatchObject({ ...colors, size: { width: 800, height: 450 }, zoom: 0.75, pan: [4, 8], viewSeq: 3 });
    expect(logs).toHaveLength(2);

    flip('(color-gamut: p3)', initialHigh);
    flip('(dynamic-range: high)', initialHigh);
    expect(channel.states).toHaveLength(3);
    expect(presentation.stats()?.generation).toBe(7);
    expect(logs).toHaveLength(4);
    presentation.dispose();
    expect([...lists.values()].every((list) => list.listeners.size === 0)).toBe(true);
  });

  it('a canvas that was hidden counts its first frame as on screen only after the display shows it', async () => {
    const service = new Service();
    service.revealFrames = 2;
    const painter = new Painter();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => painter });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    const presented: number[] = [];
    presentation.onPresented((frame) => presented.push(frame.seq));
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now());
    // Drawn and shown, but not on screen yet: no events, no `latest`.
    expect(painter.draws).toBe(1);
    expect(presentation.canvas.style.visibility).toBe('visible');
    expect(presentation.presenting).toBe(false);
    expect(presentation.latest).toBeNull();
    expect(presented).toEqual([]);
    // The GPU finishes it; a newer frame drawn meanwhile is the one that
    // counts, two display frames after the finish.
    await settle();
    channel.handlers.message({ type: 'frame', frame: desc(2, 1) }, performance.now());
    runFrames(rafs, performance.now() + 16);
    expect(presented).toEqual([]);
    runFrames(rafs, performance.now() + 33);
    expect(presented).toEqual([]);
    runFrames(rafs, performance.now() + 50);
    expect(presented).toEqual([2]);
    expect(presentation.canvas.style.visibility).toBe('visible');
    expect(presentation.presenting).toBe(true);
    expect(presentation.latest).toMatchObject({ seq: 2 });
    // While shown, later frames count at once.
    channel.handlers.message({ type: 'frame', frame: desc(3, 1) }, performance.now());
    runFrames(rafs, performance.now() + 50);
    expect(presented).toEqual([2, 3]);
    presentation.dispose();
  });

  it('a worker’s first frame counts once its GPU work finished and the display took it', async () => {
    const service = new Service();
    service.revealFrames = 2;
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    offscreenCanvases(window as unknown as Window);
    const painter = new Painter();
    let finish: () => void = () => {};
    painter.workDone = () => new Promise<void>((resolve) => { finish = resolve; });
    const worker = inProcessWorker(painter);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter(), createWorker: () => worker });
    await settleLong();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    const presented: number[] = [];
    presentation.onPresented((frame) => presented.push(frame.seq));
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame.
    runFrames(rafs, performance.now());
    await settleLong();
    expect(painter.draws).toBe(1);
    // Animation frames pass while the GPU has not finished the frame.
    for (let i = 0; i < 4; i++) runFrames(rafs, 100 + i * 16);
    expect(presented).toEqual([]);
    expect(presentation.presenting).toBe(false);
    // It finishes: two display frames later it counts.
    finish();
    await settleLong();
    runFrames(rafs, 200);
    expect(presented).toEqual([]);
    runFrames(rafs, 216);
    runFrames(rafs, 233);
    expect(presented).toEqual([1]);
    expect(presentation.presenting).toBe(true);
    presentation.dispose();
    delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
  });

  it('re-sends the current view with the vsync hint (same viewSeq) and keeps the hint on later states', async () => {
    const service = new Service();
    Object.assign(service, { hostOffsetUs: () => 2_000_000, displayHz: () => 120 });
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    expect(channel.states).toHaveLength(1);
    expect(channel.states[0]!.vsync).toBeUndefined();
    for (let i = 0, t = 500; i < 30; i++, t += 1000 / 120) runFrames(rafs, t);
    expect(channel.states).toHaveLength(2);
    expect(channel.states[1]!.viewSeq).toBe(1);
    expect(channel.states[1]!.vsync?.periodUs).toBeCloseTo(8333.33, 1);
    presentation.update({ ...PARAMS, pan: [4, 0] });
    expect(channel.states[2]).toMatchObject({ viewSeq: 2, vsync: channel.states[1]!.vsync });
    presentation.dispose();
  });

  it('takes the stage size from the caller without reading layout (gesture steps)', async () => {
    const service = new Service();
    const { stage, overlay } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    presentation.update(PARAMS);
    let reads = 0;
    const read = stage.getBoundingClientRect.bind(stage);
    stage.getBoundingClientRect = () => {
      reads++;
      return read();
    };
    presentation.update({ ...PARAMS, pan: [12, 0] }, { width: 640, height: 360 });
    expect(reads).toBe(0);
    const channel = service.channels[0]!;
    expect(channel.states.map((s) => s.viewSeq)).toEqual([1, 2]);
    presentation.dispose();
  });

  it('hides on a closed session and reopens with the current state when the engine is ready again', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    // The viewer catches up on a resize it skipped while the engine presented.
    const hidden: boolean[] = [];
    presentation.onHidden(() => hidden.push(presentation.presenting));
    await settle();
    presentation.update(PARAMS);
    const first = service.channels[0]!;
    first.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(presentation.presenting).toBe(true);
    expect(hidden).toEqual([]);
    service.connected = false;
    first.handlers.closed('the interactive engine restarted');
    expect(presentation.presenting).toBe(false);
    expect(presentation.latest).toBeNull();
    // Once, after the canvas is hidden (so the viewer's resize is not held back).
    expect(hidden).toEqual([false]);
    service.fireReady();
    await settle();
    expect(service.channels).toHaveLength(1);
    service.connected = true;
    service.fireReady();
    await settle();
    const second = service.channels[1]!;
    expect(second.states).toEqual([first.states[0]]);
    expect(presentation.stats()?.sessions).toBe(2);
    presentation.dispose();
  });

  it('exclusive ("Use New Engine" on): shows its state until the first frame, keeps the last frame through a restart, never hands back', async () => {
    const service = new Service();
    const presented: Array<{ seq: number; quality: number }> = [];
    Object.assign(service, { exclusive: true, presented: (frame: { seq: number }, quality: number) => presented.push({ seq: frame.seq, quality }) });
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    const hidden: number[] = [];
    presentation.onHidden(() => hidden.push(1));
    expect(presentation.exclusive).toBe(true);
    expect(presentation.status).toBe(PRESENTATION_STATUS.starting);
    expect(stage.querySelector('[data-engine-status]')?.textContent).toBe(PRESENTATION_STATUS.starting);
    await settle();
    presentation.update({ ...PARAMS, resolution: 'half' });
    const first = service.channels[0]!;
    first.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(presentation.presenting).toBe(true);
    expect(presentation.status).toBeNull();
    // Each new frame once, with the quality its view asked for.
    expect(presented).toEqual([{ seq: 1, quality: 0.5 }]);
    runFrames(rafs, performance.now() + 20);
    expect(presented).toHaveLength(1);
    // The engine restarts: the last frame stays, with its tags, and says so.
    service.connected = false;
    first.handlers.closed('the interactive engine restarted');
    expect(presentation.presenting).toBe(true);
    expect(presentation.latest?.seq).toBe(1);
    expect(presentation.status).toBe(PRESENTATION_STATUS.reconnecting);
    expect(hidden).toEqual([]);
    service.connected = true;
    service.fireReady();
    await settle();
    const second = service.channels[1]!;
    second.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now() + 40);
    expect(presentation.status).toBeNull();
    // A composition the engine cannot draw (amendment 21): the stage says
    // why and is never handed back; a frame the engine produced before the
    // report, still in the presenter, stays off it; the next frame the
    // engine draws clears it.
    const why = 'The new engine cannot draw this composition yet: path masks (layer box in C).';
    second.handlers.message({ type: 'frame', frame: { ...desc(2, 1), revision: 9 } }, performance.now());
    second.handlers.message({ type: 'routed', route: 'legacy', reason: why, revision: 9 } as never, performance.now());
    expect(hidden).toEqual([]);
    expect(presentation.presenting).toBe(false);
    expect(presentation.latest).toBeNull();
    expect(presentation.status).toBe(why);
    expect(stage.querySelector('[data-engine-status]')?.getAttribute('role')).toBe('status');
    runFrames(rafs, performance.now() + 60);
    expect(presentation.presenting).toBe(false);
    expect(presentation.status).toBe(why);
    second.handlers.message({ type: 'routed', route: 'legacy', reason: '', revision: 9 } as never, performance.now());
    expect(presentation.status).toBe(PRESENTATION_STATUS.unsupported);
    second.handlers.message({ type: 'frame', frame: { ...desc(3, 1), revision: 10 } }, performance.now());
    runFrames(rafs, performance.now() + 80);
    runFrames(rafs, performance.now() + 100);
    expect(presentation.presenting).toBe(true);
    expect(presentation.status).toBeNull();
    expect(hidden).toEqual([]);
    // A composition switch, A (cannot draw) to B and back, twice: the
    // document batch reaches the engine before the viewer state, so it
    // reports A at the revision it then draws B at. B's frame at that very
    // revision shows and clears the line; A's report hides it again.
    let seq = 3;
    for (const revision of [11, 12]) {
      second.handlers.message({ type: 'routed', route: 'legacy', reason: why, revision } as never, performance.now());
      expect(presentation.status).toBe(why);
      expect(presentation.presenting).toBe(false);
      second.handlers.message({ type: 'frame', frame: { ...desc(++seq, 1), revision } }, performance.now());
      runFrames(rafs, performance.now() + 120 + seq * 20);
      runFrames(rafs, performance.now() + 130 + seq * 20);
      expect(presentation.presenting).toBe(true);
      expect(presentation.latest?.revision).toBe(revision);
      expect(presentation.status).toBeNull();
    }
    // The report names the newest seq the engine had given the view (7):
    // frames 6 and 7, of revision 12, were still on the GPU. Frame 6 arrives
    // after the report with a higher seq than any delivered: it is stale and
    // the line stays. Frame 8 is a later decision.
    second.handlers.message({ type: 'routed', route: 'legacy', reason: why, revision: 13, seq: 7 } as never, performance.now());
    expect(presentation.status).toBe(why);
    second.handlers.message({ type: 'frame', frame: { ...desc(6, 1), revision: 12 } }, performance.now());
    runFrames(rafs, performance.now() + 300);
    runFrames(rafs, performance.now() + 320);
    expect(presentation.presenting).toBe(false);
    expect(presentation.latest).toBeNull();
    expect(presentation.status).toBe(why);
    second.handlers.message({ type: 'frame', frame: { ...desc(8, 1), revision: 13 } }, performance.now());
    runFrames(rafs, performance.now() + 340);
    runFrames(rafs, performance.now() + 360);
    expect(presentation.presenting).toBe(true);
    expect(presentation.latest?.seq).toBe(8);
    expect(presentation.status).toBeNull();
    expect(hidden).toEqual([]);
    presentation.dispose();
    expect(stage.querySelector('[data-engine-status]')).toBeNull();
  });

  it('follows the stage into a popout: the loop runs on its frames and its display sets the state', async () => {
    const service = new Service();
    const main = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage: main.stage, before: main.overlay, createPainter: async () => new Painter(), sizeSettleMs: 0 });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    expect(main.rafs.size).toBe(1);

    const popout = new HappyWindow() as unknown as Window;
    const target = stageIn(popout, 800, 450);
    // The popout's clock starts 5 s after ours.
    Object.defineProperty(popout, 'performance', { value: { timeOrigin: performance.timeOrigin + 5000, now: () => 0 }, configurable: true });
    Object.assign(popout, {
      devicePixelRatio: 1,
      matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} })
    });
    // The stage (with the canvas) moves into the popout's document.
    popout.document.body.replaceChildren(popout.document.adoptNode(main.stage));
    main.stage.getBoundingClientRect = target.stage.getBoundingClientRect;
    expect(presentation.canvas.ownerDocument).toBe(popout.document);
    presentation.update(PARAMS);
    expect(main.rafs.size).toBe(0);
    expect(target.rafs.size).toBe(1);
    const moved = channel.states.at(-1)!;
    expect(moved).toMatchObject({ size: { width: 800, height: 450 }, devicePixelRatio: 1, edr: false, viewSeq: 2 });

    // Its rAF timestamps are shifted onto this realm's clock.
    let seen = -1;
    presentation.onPresented((frame) => (seen = frame.rafMs));
    channel.handlers.message({ type: 'frame', frame: desc(3, 2) }, performance.now());
    runFrames(target.rafs, 10);
    expect(presentation.latest?.seq).toBe(3);
    expect(target.rafs.size).toBe(1);
    expect(seen).toBe(5010);
    presentation.dispose();
    await (popout as unknown as HappyWindow).happyDOM.close();
  });

  it('sends a new size only once it holds still, keeping zoom and pan changes immediate', async () => {
    const service = new Service();
    const { stage, overlay } = stageIn(window as unknown as Window);
    const timers: { fn: () => void; ms: number; live: boolean }[] = [];
    const presentation = new ViewerPresentation({
      service,
      stage,
      before: overlay,
      createPainter: async () => new Painter(),
      timers: {
        set: (fn, ms) => timers.push({ fn, ms, live: true }),
        clear: (id) => (timers[id - 1]!.live = false)
      }
    });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    let width = 640;
    stage.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: width, bottom: 360, width, height: 360, toJSON: () => ({}) }) as DOMRect;
    // A live resize: three steps, then a pan while it is still moving.
    for (const w of [660, 700, 740]) {
      width = w;
      presentation.update(PARAMS);
    }
    presentation.update({ ...PARAMS, pan: [5, 0] });
    expect(channel.states.map((s) => s.size.width)).toEqual([1280, 1280]);
    expect(channel.states[1]!.pan).toEqual([5, 0]);
    expect(timers.filter((t) => t.live)).toHaveLength(1);
    expect(timers.at(-1)!.ms).toBe(150);
    timers.find((t) => t.live)!.fn();
    expect(channel.states.at(-1)!.size).toEqual({ width: 1480, height: 720 });
    presentation.dispose();
  });

  it('shows a named compatibility failure on the stage and clears it on a rendered frame', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'failure', reason: 'invalidMessage: compatibility raster scale 33 outside 0..=32' }, performance.now());
    expect(stage.querySelector('[data-engine-status]')?.textContent).toContain('compatibility raster scale 33');
    expect(presentation.presenting).toBe(false);
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(presentation.status).toBeNull();
    presentation.dispose();
  });

  it('settles a stable pending size while repeated playback updates continue', async () => {
    const service = new Service();
    const { stage } = stageIn(window as unknown as Window);
    const timers: { fn: () => void; live: boolean }[] = [];
    const presentation = new ViewerPresentation({ service, stage, createPainter: async () => new Painter(), timers: {
      set: (fn) => timers.push({ fn, live: true }), clear: (id) => { timers[id-1]!.live = false; }
    } });
    await settle();
    presentation.update(PARAMS, { width: 640, height: 360 });
    stage.getBoundingClientRect = () => ({ width: 960, height: 540 }) as DOMRect;
    presentation.update(PARAMS);
    // The same 150ms pending size must not be postponed every animation frame.
    for (let frame = 0; frame < 120; frame++) presentation.update(PARAMS);
    expect(timers).toHaveLength(1);
    timers[0]!.fn();
    const channel = service.channels[0]!;
    expect(channel.states.at(-1)!.size).toEqual({ width: 1920, height: 1080 });
    const stable = channel.states.at(-1)!.viewSeq;
    for (let frame = 0; frame < 120; frame++) presentation.update(PARAMS);
    expect(channel.states.at(-1)!.viewSeq).toBe(stable);
    expect(channel.states).toHaveLength(2);
    presentation.dispose();
  });

  it('starts a pending resize deadline again when its stage moves to another window', async () => {
    const service = new Service();
    const { stage } = stageIn(window as unknown as Window);
    const timers: { fn: () => void; live: boolean }[] = [];
    const presentation = new ViewerPresentation({ service, stage, createPainter: async () => new Painter(), timers: {
      set: (fn) => timers.push({ fn, live: true }), clear: (id) => { timers[id-1]!.live = false; }
    } });
    await settle();
    presentation.update(PARAMS, { width: 640, height: 360 });
    stage.getBoundingClientRect = () => ({width:960,height:540}) as DOMRect;
    presentation.update(PARAMS);
    const popout = new HappyWindow() as unknown as Window;
    stageIn(popout);
    popout.document.body.appendChild(stage);
    presentation.update(PARAMS);
    expect(timers).toHaveLength(2);
    expect(timers[0]!.live).toBe(false);
    expect(timers[1]!.live).toBe(true);
    timers[1]!.fn();
    expect(service.channels[0]!.states.at(-1)!.size).toEqual({width:1920,height:1080});
    presentation.dispose();
    await (popout as unknown as HappyWindow).happyDOM.close();
  });

  it('presents from a worker when the viewer can start one: frames, tags, releases, and teardown', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const canvases = offscreenCanvases(window as unknown as Window);
    const painter = new Painter();
    const worker = inProcessWorker(painter);
    const mainPainter = new Painter();
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createWorker: () => worker, createPainter: async () => mainPainter });
    await settle();
    expect(presentation.offMainThread).toBe(true);
    expect(canvases.transferred).toEqual([presentation.canvas]);
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    expect(channel.states).toHaveLength(1);
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame.
    runFrames(rafs, performance.now());
    await settleLong();
    // Drawn by the worker as it arrived (a next-refresh frame).
    expect(painter.draws).toBe(1);
    expect(mainPainter.draws).toBe(0);
    expect(presentation.presenting).toBe(true);
    expect(presentation.latest).toMatchObject({ seq: 1, viewSeq: 1 });
    expect(channel.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(presentation.stats()).toMatchObject({ drawn: 1, received: 1, offMainThread: true });
    presentation.dispose();
    await settleLong();
    expect(painter.disposed).toBe(true);
    expect(worker.terminated).toBe(true);
    delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
  });

  it('falls back to drawing on this thread in a new canvas when the worker cannot draw, keeping the session', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    offscreenCanvases(window as unknown as Window);
    const logs: string[] = [];
    const mainPainter = new Painter();
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createWorker: () => inProcessWorker(null), createPainter: async () => mainPainter, log: (_l, m) => logs.push(m) });
    await settle();
    const first = presentation.canvas;
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    await settleLong();
    expect(logs).toEqual(['engine presentation worker failed, presenting on the main thread: no WebGPU in workers']);
    expect(presentation.offMainThread).toBe(false);
    expect(presentation.canvas).not.toBe(first);
    expect(presentation.canvas.nextSibling).toBe(overlay);
    expect(stage.contains(first)).toBe(false);
    // The frame the worker never drew went back to the engine.
    expect(channel.releases).toEqual([{ poolIndex: 1, seq: 1 }]);
    expect(service.channels).toHaveLength(1);
    channel.handlers.message({ type: 'frame', frame: desc(2, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(mainPainter.draws).toBe(1);
    expect(presentation.presenting).toBe(true);
    presentation.dispose();
    delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
  });

  it('replaces a worker whose device was lost with a new canvas and worker, keeping the session', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    offscreenCanvases(window as unknown as Window);
    const logs: string[] = [];
    const painters = [new Painter(), new Painter()];
    const workers = painters.map((p) => inProcessWorker(p));
    let made = 0;
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createWorker: () => workers[made++]!, createPainter: async () => new Painter(), log: (_l, m) => logs.push(m) });
    await settle();
    const first = presentation.canvas;
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } }, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame.
    runFrames(rafs, performance.now());
    await settleLong();
    expect(painters[0]!.draws).toBe(1);
    painters[0]!.lose('A valid external Instance reference no longer exists.');
    await settleLong();
    expect(logs).toEqual([
      'engine presentation lost its graphics context (A valid external Instance reference no longer exists.); drawing again on a new one',
      'engine presentation asks for new surfaces: the graphics context was lost (A valid external Instance reference no longer exists.)'
    ]);
    // The imports from before the loss may not upload any more: the new
    // drawing asks for new surfaces (the next epoch, a new view).
    const renewal = channel.states.at(-1)!;
    expect(renewal.surfaceEpoch).toBe(1);
    expect(renewal.viewSeq).toBe(channel.states[0]!.viewSeq + 1);
    expect(presentation.state?.surfaceEpoch).toBe(1);
    // A new worker on a new canvas, the same session; still off the main thread.
    expect(made).toBe(2);
    expect(workers[0]!.terminated).toBe(true);
    expect(presentation.offMainThread).toBe(true);
    expect(presentation.canvas).not.toBe(first);
    expect(service.channels).toHaveLength(1);
    channel.handlers.message({ type: 'frame', frame: desc(2, 1) }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame.
    runFrames(rafs, performance.now());
    await settleLong();
    expect(painters[1]!.draws).toBe(1);
    expect(presentation.presenting).toBe(true);
    expect(channel.releases.map((r) => r.seq)).toEqual([1, 2]);
    presentation.dispose();
    delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
  });

  it('asks for new surfaces once per pool whose frames cannot be read, and draws the new pool without any view change', async () => {
    const service = new Service();
    Object.assign(service, { exclusive: true });
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const logs: string[] = [];
    const painter = new Painter();
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => painter, log: (_l, m) => logs.push(m) });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } }, performance.now());
    painter.from(1);
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(painter.draws).toBe(1);
    expect(presentation.presenting).toBe(true);
    // The GPU process restarted under a drawing that survived: frames of
    // the pool imported before it fail to upload.
    painter.unreadable.add(1);
    channel.handlers.message({ type: 'frame', frame: desc(2, 1) }, performance.now());
    runFrames(rafs, performance.now());
    channel.handlers.message({ type: 'frame', frame: desc(3, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(presentation.stats()!.dropped['unusable']).toBe(2);
    // One renewal for the pool, however many of its frames fail.
    expect(channel.states.map((s) => s.surfaceEpoch ?? 0)).toEqual([0, 1]);
    expect(channel.states[1]!.viewSeq).toBe(2);
    expect(logs).toEqual(['engine presentation asks for new surfaces: the presented frame could not be read: VideoFrame RGBA16F upload failed (GL 0x501)']);
    // The engine's new pool for the renewed view draws.
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 2, surfaces: [] } }, performance.now());
    painter.from(2);
    channel.handlers.message({ type: 'frame', frame: desc(4, 2, 2) }, performance.now());
    runFrames(rafs, performance.now());
    expect(painter.draws).toBe(2);
    expect(presentation.latest?.seq).toBe(4);
    expect(presentation.status).toBe(null);
    // Later views keep the epoch; they are not renewals.
    presentation.update({ ...PARAMS, zoom: 0.75 });
    expect(channel.states.at(-1)!.surfaceEpoch).toBe(1);
    expect(channel.states.at(-1)!.viewSeq).toBe(3);
    presentation.dispose();
  });

  it('asks for new surfaces when a presentation worker cannot read a pool', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    offscreenCanvases(window as unknown as Window);
    const logs: string[] = [];
    const painter = new Painter();
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createWorker: () => inProcessWorker(painter), createPainter: async () => new Painter(), log: (_l, m) => logs.push(m) });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 1, surfaces: [] } }, performance.now());
    painter.unreadable.add(1);
    painter.from(1);
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame.
    runFrames(rafs, performance.now());
    await settleLong();
    expect(presentation.offMainThread).toBe(true);
    expect(painter.draws).toBe(0);
    expect(channel.states.map((s) => s.surfaceEpoch ?? 0)).toEqual([0, 1]);
    expect(logs).toEqual(['engine presentation asks for new surfaces: the presented frame could not be read: VideoFrame RGBA16F upload failed (GL 0x501)']);
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 2, surfaces: [] } }, performance.now());
    painter.from(2);
    channel.handlers.message({ type: 'frame', frame: desc(2, 2, 2) }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame.
    runFrames(rafs, performance.now());
    await settleLong();
    expect(painter.draws).toBe(1);
    expect(presentation.presenting).toBe(true);
    expect(channel.releases.map((r) => r.seq)).toEqual([1, 2]);
    presentation.dispose();
    delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
  });

  it('stops renewing surfaces that keep failing and says so on the stage', async () => {
    const service = new Service();
    Object.assign(service, { exclusive: true });
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const logs: { level: string; message: string }[] = [];
    const painter = new Painter();
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => painter, log: (level, message) => logs.push({ level, message }) });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    let seq = 0;
    // Every pool's frames fail (an upload the context always rejects).
    for (let generation = 1; generation <= MAX_SURFACE_RENEWALS + 2; generation++) {
      painter.unreadable.add(generation);
      channel.handlers.message({ type: 'surfacePool', pool: { generation, surfaces: [] } }, performance.now());
      painter.from(generation);
      channel.handlers.message({ type: 'frame', frame: desc(++seq, channel.states.at(-1)!.viewSeq, generation) }, performance.now());
      runFrames(rafs, performance.now());
    }
    expect(painter.draws).toBe(0);
    expect(channel.states.filter((s) => s.surfaceEpoch).length).toBe(MAX_SURFACE_RENEWALS);
    expect(logs.filter((l) => l.level === 'error')).toEqual([
      { level: 'error', message: `engine presentation cannot read its surfaces: ${MAX_SURFACE_RENEWALS} renewals in 30 s (the presented frame could not be read: VideoFrame RGBA16F upload failed (GL 0x501))` },
      { level: 'error', message: `engine presentation cannot read its surfaces: ${MAX_SURFACE_RENEWALS} renewals in 30 s (the presented frame could not be read: VideoFrame RGBA16F upload failed (GL 0x501))` }
    ]);
    expect(presentation.status).toBe(PRESENTATION_STATUS.unreadable);
    presentation.dispose();
  });

  it('replaces a main-thread painter whose device was lost, and gives up after repeated losses', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const logs: string[] = [];
    const painters: Painter[] = [];
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => (painters.push(new Painter()), painters.at(-1)!), log: (_l, m) => logs.push(m) });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(painters[0]!.draws).toBe(1);
    painters[0]!.lose('gpu restarted');
    await settleLong();
    expect(painters).toHaveLength(2);
    expect(painters[0]!.disposed).toBe(true);
    channel.handlers.message({ type: 'frame', frame: desc(2, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(painters[1]!.draws).toBe(1);
    expect(presentation.presenting).toBe(true);
    // Losses that keep coming back: after the third replacement it stops,
    // and the current engine's canvas shows.
    for (let i = 1; i <= 3; i++) {
      painters[i]!.lose('gpu restarted');
      await settleLong();
    }
    expect(painters).toHaveLength(4);
    expect(logs.at(-1)).toMatch(/^engine presentation stopped: the graphics context was lost 4 times/);
    expect(presentation.presenting).toBe(false);
    channel.handlers.message({ type: 'frame', frame: desc(3, 1) }, performance.now());
    expect(channel.releases.at(-1)).toEqual({ poolIndex: 3, seq: 3 });
    presentation.dispose();
  });

  it('gives a popout a new canvas and worker, carrying the session and pool over', async () => {
    const service = new Service();
    const main = stageIn(window as unknown as Window);
    offscreenCanvases(window as unknown as Window);
    const workers: ReturnType<typeof inProcessWorker>[] = [];
    const presentation = new ViewerPresentation({
      service,
      stage: main.stage,
      before: main.overlay,
      sizeSettleMs: 0,
      createWorker: () => {
        const w = inProcessWorker(new Painter());
        workers.push(w);
        return w;
      }
    });
    await settle();
    presentation.update(PARAMS);
    const channel = service.channels[0]!;
    channel.handlers.message({ type: 'surfacePool', pool: { generation: 3, surfaces: [] } }, performance.now());
    await settleLong();

    const popout = new HappyWindow() as unknown as Window;
    const target = stageIn(popout, 800, 450);
    offscreenCanvases(popout);
    Object.defineProperty(popout, 'performance', { value: { timeOrigin: performance.timeOrigin + 5000, now: () => 0 }, configurable: true });
    Object.assign(popout, { devicePixelRatio: 1, matchMedia: () => ({ matches: false, addEventListener() {}, removeEventListener() {} }) });
    popout.document.body.replaceChildren(popout.document.adoptNode(main.stage));
    main.stage.getBoundingClientRect = target.stage.getBoundingClientRect;
    presentation.update(PARAMS);
    await settleLong();
    expect(workers).toHaveLength(2);
    expect(workers[0]!.terminated).toBe(true);
    // The popout's worker decides on this realm's clock, which stamps arrivals.
    expect(workers[1]!.origins).toEqual([performance.timeOrigin]);
    expect(presentation.canvas.ownerDocument).toBe(popout.document);
    expect(presentation.offMainThread).toBe(true);
    expect(service.channels).toHaveLength(1);
    expect(presentation.stats()?.generation).toBe(3);
    expect(channel.states.at(-1)).toMatchObject({ size: { width: 800, height: 450 }, devicePixelRatio: 1 });
    channel.handlers.message({ type: 'frame', frame: { ...desc(4, channel.states.at(-1)!.viewSeq), poolGeneration: 3 } }, performance.now());
    // Sent to the worker, and announced, at the page's animation frame, on
    // this realm's clock (the popout's starts 5 s later).
    runFrames(target.rafs, 100);
    await settleLong();
    expect(presentation.latest?.seq).toBe(4);
    expect(presentation.latest?.rafMs).toBe(5100);
    presentation.dispose();
    await (popout as unknown as HappyWindow).happyDOM.close();
    delete (HTMLCanvasElement.prototype as { transferControlToOffscreen?: unknown }).transferControlToOffscreen;
  });

  it('exclusive: a viewer that cannot draw says so instead of showing another engine', async () => {
    const service = new Service();
    Object.assign(service, { exclusive: true });
    const { stage, overlay } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => { throw new Error('WebGPU is not available'); } });
    await settle();
    expect(presentation.status).toBe(`${PRESENTATION_STATUS.unavailable} WebGPU is not available`);
    presentation.dispose();
  });

  it('does nothing without WebGPU and says why', async () => {
    const service = new Service();
    const logs: string[] = [];
    const { stage } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({
      service,
      stage,
      createPainter: async () => {
        throw new Error('WebGPU is not available');
      },
      log: (_level, message) => logs.push(message)
    });
    await settle();
    presentation.update(PARAMS);
    expect(service.channels).toHaveLength(0);
    expect(logs).toEqual(['engine presentation is unavailable: WebGPU is not available']);
    presentation.dispose();
  });
});

describe('overlayToCss', () => {
  it('scales rectangles, bounds, paths, the caret, and the 2D world matrix by 1/DPR', () => {
    const css = overlayToCss(OVERLAY, 2);
    expect(css.viewport).toEqual({ x: 50, y: 25, width: 500, height: 300 });
    expect(css.layers[0]!.world2d).toEqual([1, 0, 0, 0, 1, 0, 50, 25, 1]);
    expect(css.layers[0]!.bounds[2]).toEqual([150, 100]);
    expect(css.layers[0]!.paths[0]).toEqual([{ op: 'moveTo', x: 5, y: 10 }, { op: 'cubicTo', x1: 1, y1: 2, x2: 3, y2: 4, x: 5, y: 6 }, { op: 'close' }]);
    expect(css.caret).toEqual({ top: [20, 20], bottom: [20, 40] });
  });
});
