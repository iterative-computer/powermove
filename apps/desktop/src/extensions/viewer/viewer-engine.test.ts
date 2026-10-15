// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { PowermoveAPI } from 'powermove';
import { ENGINE_VIEWER_SERVICE, type EngineViewerHandlers, type EngineViewerService } from './engine-present/presentation';
import type { FrameDescriptor } from './engine-present/protocol';
import { createViewerRuntime } from './viewer';

// happy-dom has no WebGPU: a test can ask for a painter that draws nothing.
const gpu = vi.hoisted(() => ({ fake: false }));
vi.mock('./engine-present/gpu', async (original) => {
  const actual = await original<typeof import('./engine-present/gpu')>();
  return {
    ...actual,
    createWebGpuPainter: async (canvas: never) => gpu.fake
      ? { configure() {}, draw() {}, workDone: () => Promise.resolve(), dispose() {} }
      : actual.createWebGpuPainter(canvas)
  };
});

/* The viewer's hook for engine presentation (behind "Use New Engine"):
   nothing happens without the kernel's `engine.viewer` service; with it the
   viewer attaches a presentation canvas under its overlay and tears it down
   with the runtime. The presentation itself is tested in engine-present. */

function viewerApi(services: Map<string, unknown>, log: (level: string, message: string) => void): PowermoveAPI {
  const project: any = { id: 'comp-1', w: 1920, h: 1080, fps: 30, dur: 10, layers: [], assets: {} };
  return {
    project: { get: () => project },
    selection: { layers: () => [], first: () => null, select() {} },
    groups: { ancestors: () => [], transformRoots: () => [] },
    anim: { active: () => true, ev: () => 0, version: () => 0, worldMatrix: () => [1, 0, 0, 1, 0, 0] },
    model: { TYPE_META: {}, layer: () => undefined, curComp: () => project },
    render: { gl: { bounds: () => null, pick: () => null, context: null, previewViewport: null, resize: vi.fn() } },
    transport: { time: () => 0, playing: () => false, quality: 1, perf: { auto: false }, previewResolution: 1, invalidate() {} },
    util: { clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)) },
    events: { on: () => ({ dispose() {} }) },
    services: {
      register: (name: string, implementation: unknown) => { services.set(name, implementation); return { dispose: () => services.delete(name) }; },
      get: (name: string) => services.get(name) ?? null
    },
    log,
    ui: {}, dnd: {}, menus: {}, edit: {}, history: {}
  } as unknown as PowermoveAPI;
}

function mount(runtime: any, size = { width: 1000, height: 700 }): { stage: HTMLElement; overlay: HTMLCanvasElement } {
  const stage = document.createElement('div');
  const inner = document.createElement('div');
  const overlay = document.createElement('canvas');
  stage.append(inner, overlay);
  document.body.append(stage);
  stage.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: size.width, bottom: size.height, width: size.width, height: size.height, toJSON: () => ({}) }) as DOMRect;
  Object.defineProperty(stage, 'clientWidth', { get: () => size.width, configurable: true });
  Object.defineProperty(stage, 'clientHeight', { get: () => size.height, configurable: true });
  runtime.el = document.createElement('canvas');
  runtime.ov = overlay;
  runtime.octx = new Proxy({}, { get: () => () => {} });
  runtime.stage = stage;
  runtime.inner = inner;
  return { stage, overlay };
}

describe('viewer engine presentation hook', () => {
  it('does nothing without the engine.viewer service', () => {
    const services = new Map<string, unknown>();
    const runtime = createViewerRuntime(viewerApi(services, () => {})) as any;
    const { stage } = mount(runtime);
    runtime.layout();
    expect(stage.querySelector('#engine-present')).toBeNull();
    expect(runtime.engineFrame()).toBeNull();
    runtime.dispose();
  });

  it('attaches a presentation canvas under the overlay when the service is registered, and removes it on dispose', async () => {
    const services = new Map<string, unknown>();
    const logs: string[] = [];
    const service: EngineViewerService = {
      source: { videoFrame: () => null, onSurface: () => () => {}, retire: () => 0 },
      composition: () => 'comp-1',
      open: vi.fn(() => null),
      onReady: () => () => {}
    };
    services.set(ENGINE_VIEWER_SERVICE, service);
    const runtime = createViewerRuntime(viewerApi(services, (_level, message) => logs.push(message))) as any;
    const { stage, overlay } = mount(runtime);
    runtime.layout();
    const canvas = stage.querySelector<HTMLCanvasElement>('#engine-present');
    expect(canvas?.nextSibling).toBe(overlay);
    expect(canvas?.style.visibility).toBe('hidden');
    await new Promise((resolve) => setTimeout(resolve, 0));
    // happy-dom has no WebGPU: the presentation says so and opens no session.
    expect(logs).toContain('engine presentation is unavailable: WebGPU is not available');
    expect(service.open).not.toHaveBeenCalled();
    runtime.layout();
    expect(stage.querySelectorAll('#engine-present')).toHaveLength(1);
    runtime.dispose();
    expect(stage.querySelector('#engine-present')).toBeNull();
  });

  it('gives the WebGL canvas the size it skipped while the engine presented once the engine session ends', async () => {
    gpu.fake = true;
    const rafs = new Map<number, FrameRequestCallback>();
    let nextRaf = 1;
    const raf = vi.spyOn(window, 'requestAnimationFrame').mockImplementation((cb) => (rafs.set(nextRaf, cb), nextRaf++));
    const caf = vi.spyOn(window, 'cancelAnimationFrame').mockImplementation((id) => void rafs.delete(id));
    const runFrames = (): void => {
      const due = [...rafs.values()];
      rafs.clear();
      for (const cb of due) cb(performance.now());
    };
    try {
      const services = new Map<string, unknown>();
      let handlers: EngineViewerHandlers | null = null;
      const service: EngineViewerService = {
        source: { videoFrame: () => ({ codedWidth: 1000, codedHeight: 700, close() {} }) as unknown as VideoFrame, onSurface: () => () => {}, retire: () => 0 },
        composition: () => 'comp-1',
        offMainThread: false,
        open: (h) => {
          handlers = h;
          return { sendViewerState() {}, release() {}, close() {} };
        },
        onReady: () => () => {}
      };
      services.set(ENGINE_VIEWER_SERVICE, service);
      const api = viewerApi(services, () => {});
      const resize = api.render.gl.resize as unknown as ReturnType<typeof vi.fn>;
      const runtime = createViewerRuntime(api) as any;
      const size = { width: 1000, height: 700 };
      mount(runtime, size);
      runtime.layout();
      await new Promise((resolve) => setTimeout(resolve, 0));
      expect(handlers).not.toBeNull();
      const desc: FrameDescriptor = {
        poolIndex: 0, poolGeneration: 1, seq: 1, pts: 0, revision: 1, colorSpace: 'srgb', format: 'bgra8Unorm',
        quality: { resolution: 'full', representation: 'original', draft3d: false, cached: false },
        overlay: null, depthId: null, viewSeq: runtime.enginePresentation.state.viewSeq, duration: null
      };
      handlers!.message({ type: 'frame', frame: desc }, performance.now());
      runFrames();
      expect(runtime.enginePresentation.presenting).toBe(true);

      // A live resize while the engine presents: the WebGL canvas keeps its size.
      const before = resize.mock.calls.length;
      size.width = 1400; size.height = 900;
      runtime.layout();
      expect(resize.mock.calls.length).toBe(before);

      // The engine session ends: the WebGL canvas shows again at the new size.
      handlers!.closed('the interactive engine restarted');
      expect(runtime.enginePresentation.presenting).toBe(false);
      expect(resize.mock.calls.length).toBe(before + 1);
      const [width, height] = resize.mock.calls.at(-1)!;
      expect(width / height).toBeCloseTo(1920 / 1080, 1);
      runtime.dispose();
    } finally {
      gpu.fake = false;
      raf.mockRestore();
      caf.mockRestore();
    }
  });
});
