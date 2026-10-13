// @vitest-environment happy-dom
import { Window as HappyWindow } from 'happy-dom';
import { describe, expect, it } from 'vitest';

import type { FramePainter } from './gpu';
import type { FrameSource } from './presenter';
import { overlayToCss, ViewerPresentation, type EngineViewerChannel, type EngineViewerHandlers, type EngineViewerService, type ViewParams } from './presentation';
import type { FrameDescriptor, FrameRelease, OverlayGeometry, ViewerState } from './protocol';

class FakeFrame {
  closed = false;
  readonly codedWidth = 1280;
  readonly codedHeight = 720;
  close(): void {
    this.closed = true;
  }
}

class Source implements FrameSource {
  private listeners = new Set<(g: number, i: number) => void>();
  videoFrame(): VideoFrame | null {
    return new FakeFrame() as unknown as VideoFrame;
  }
  onSurface(listener: (g: number, i: number) => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }
  retire(): number {
    return 0;
  }
}

class Painter implements FramePainter {
  draws = 0;
  disposed = false;
  configure(): void {}
  draw(): void {
    this.draws++;
  }
  workDone(): Promise<void> {
    return Promise.resolve();
  }
  dispose(): void {
    this.disposed = true;
  }
}

class Channel implements EngineViewerChannel {
  states: ViewerState[] = [];
  releases: FrameRelease[] = [];
  closed = false;
  constructor(readonly handlers: EngineViewerHandlers) {}
  sendViewerState(state: ViewerState): void {
    this.states.push(state);
  }
  release(release: FrameRelease): void {
    this.releases.push(release);
  }
  close(): void {
    this.closed = true;
  }
}

class Service implements EngineViewerService {
  readonly source = new Source();
  connected = true;
  channels: Channel[] = [];
  private ready = new Set<() => void>();
  composition(): string {
    return 'comp-1';
  }
  open(handlers: EngineViewerHandlers): EngineViewerChannel | null {
    if (!this.connected) return null;
    const channel = new Channel(handlers);
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

function desc(seq: number, viewSeq: number): FrameDescriptor {
  return {
    poolIndex: seq % 4,
    poolGeneration: 1,
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

describe('ViewerPresentation', () => {
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

  it('hides on a closed session and reopens with the current state when the engine is ready again', async () => {
    const service = new Service();
    const { stage, overlay, rafs } = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage, before: overlay, createPainter: async () => new Painter() });
    await settle();
    presentation.update(PARAMS);
    const first = service.channels[0]!;
    first.handlers.message({ type: 'frame', frame: desc(1, 1) }, performance.now());
    runFrames(rafs, performance.now());
    expect(presentation.presenting).toBe(true);
    service.connected = false;
    first.handlers.closed('the interactive engine restarted');
    expect(presentation.presenting).toBe(false);
    expect(presentation.latest).toBeNull();
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

  it('follows the stage into a popout: the loop runs on its frames and its display sets the state', async () => {
    const service = new Service();
    const main = stageIn(window as unknown as Window);
    const presentation = new ViewerPresentation({ service, stage: main.stage, before: main.overlay, createPainter: async () => new Painter() });
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
