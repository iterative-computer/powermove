// @vitest-environment happy-dom
import { describe, expect, it, vi } from 'vitest';

import type { PowermoveAPI } from 'powermove';
import { ENGINE_VIEWER_SERVICE, type EngineViewerService } from './engine-present/presentation';
import { createViewerRuntime } from './viewer';

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

function mount(runtime: any): { stage: HTMLElement; overlay: HTMLCanvasElement } {
  const stage = document.createElement('div');
  const inner = document.createElement('div');
  const overlay = document.createElement('canvas');
  stage.append(inner, overlay);
  document.body.append(stage);
  stage.getBoundingClientRect = () => ({ x: 0, y: 0, left: 0, top: 0, right: 1000, bottom: 700, width: 1000, height: 700, toJSON: () => ({}) }) as DOMRect;
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
});
