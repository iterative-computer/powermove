// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, PowermoveAPI, ServicesAPI, ViewerService } from 'powermove';

import activateExtension from './index';

function serviceHarness(): { services: ServicesAPI; disposeAll(): void } {
  const implementations = new Map<string, unknown>();
  const disposers: Array<() => void> = [];
  return {
    services: {
      register<T>(name: string, implementation: T) {
        implementations.set(name, implementation);
        const dispose = (): void => {
          if (implementations.get(name) === implementation) implementations.delete(name);
        };
        disposers.push(dispose);
        return { dispose };
      },
      get<T>(name: string): T | null {
        return (implementations.get(name) as T | undefined) ?? null;
      }
    },
    disposeAll(): void {
      for (const dispose of disposers.reverse()) dispose();
    }
  };
}

function apiHarness(services = serviceHarness().services) {
  let panel: PanelDefinition | undefined;
  let context: object | null = null;
  const init = vi.fn((canvas: HTMLCanvasElement) => { context = { canvas }; return true; });
  const resize = vi.fn(() => true);
  const eventDisposers: Array<ReturnType<typeof vi.fn>> = [];
  const disposeCallbacks: Array<() => void> = [];
  const project: any = { w: 1920, h: 1080, fps: 30, dur: 10, layers: [], assets: {} };
  const api = {
    panels: { register: vi.fn((definition: PanelDefinition) => { panel = definition; return { dispose() {} }; }) },
    services,
    onDispose: vi.fn((dispose: () => void) => { disposeCallbacks.push(dispose); }),
    commands: { register: vi.fn(() => ({ dispose() {} })) },
    keybindings: { bind: vi.fn(() => ({ dispose() {} })) },
    project: { get: () => project },
    selection: { layers: () => [], first: () => null, select: vi.fn() },
    groups: { ancestors: () => [], transformRoots: () => [] },
    anim: {
      active: () => true, ev: () => 0, version: () => 0,
      worldMatrix: () => [1, 0, 0, 1, 0, 0], localMatrix: () => [1, 0, 0, 1, 0, 0],
      transformParentMatrix: () => [1, 0, 0, 1, 0, 0],
    },
    model: { TYPE_META: {}, layer: () => null, curComp: () => project },
    transport: {
      time: () => 0, playing: () => false, quality: 1,
      perf: { fps: 0, ms: 0, drops: 0, budget: 0, auto: false },
      previewResolution: 1, invalidate: vi.fn(), pause: vi.fn(),
    },
    render: {
      gl: { get context() { return context; }, get previewViewport() { return null; }, init, resize, bounds: () => null, pick: () => null },
      raster: () => null,
    },
    util: { clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)) },
    ui: { icon: () => '<svg></svg>', drag: vi.fn() },
    dnd: { hasFxDrag: () => false, readFxDrag: () => null },
    menus: { collect: () => [], gather: async () => [] },
    events: { on: vi.fn(() => { const dispose = vi.fn(); eventDisposers.push(dispose); return { dispose }; }) },
    space3d: { is3DLayer: () => false },
    media: { assets: { get: () => undefined } },
    edit: {}, history: {},
  } as unknown as PowermoveAPI;
  return {
    api, init, eventDisposers,
    get panel() { return panel; },
    viewer: () => services.get<ViewerService>('viewer'),
    dispose: () => { for (const callback of disposeCallbacks.splice(0).reverse()) callback(); },
  };
}

describe('viewer extension', () => {
  it('reattaches the renderer canvas after a module update with a new service facade', async () => {
    const original = apiHarness();
    activateExtension(original.api);
    const body = document.createElement('div');
    original.panel!.build!(body, { spec: {} });
    const canvas = body.querySelector('#gl');
    const stage = original.viewer()!.stage;
    original.dispose();
    body.replaceChildren();

    vi.resetModules();
    const replacement = apiHarness();
    Object.assign(replacement.api.render, { gl: original.api.render.gl });
    const { default: activateUpdated } = await import('./index');
    activateUpdated(replacement.api);
    replacement.panel!.build!(body, { spec: {} });

    expect(body.querySelector('#gl')).toBe(canvas);
    expect(replacement.viewer()!.stage).toBe(stage);
    expect(original.init).toHaveBeenCalledTimes(1);
    replacement.dispose();
  });
  beforeEach(() => {
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({} as never));
    vi.stubGlobal('ResizeObserver', class {
      observe(): void {}
      disconnect(): void {}
      unobserve(): void {}
    });
  });

  afterEach(() => {
    document.body.replaceChildren();
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('registers the live viewer service and builds the composition panel', () => {
    const services = serviceHarness();
    const harness = apiHarness(services.services);
    activateExtension(harness.api);

    expect(harness.panel).toMatchObject({
      id: 'viewer', title: 'Composition', flush: true, noscroll: true,
      headless: true, hideMoveHandle: false
    });
    const runtime = harness.viewer();
    expect(runtime).not.toBeNull();
    expect(runtime?.snapshotSnapCandidates).toBeTypeOf('function');

    const body = document.createElement('div');
    harness.panel?.build?.(body, { spec: {} });

    expect(body.querySelector('#stage > #stage-inner > #gl')).not.toBeNull();
    expect(body.querySelector('#stage > #overlay')).not.toBeNull();
    expect(body.querySelector('#stage > #composition-recovery')?.textContent).toContain('Fit composition');
    expect(body.querySelector<HTMLButtonElement>('#composition-recovery')?.hidden).toBe(true);
    expect(body.querySelectorAll('#stage, #stage-inner, #gl, #overlay, #composition-recovery')).toHaveLength(5);
    expect(harness.init).toHaveBeenCalledWith(body.querySelector('#gl'));
    expect(runtime?.ov).toBe(body.querySelector('#overlay'));
    services.disposeAll();
    expect(harness.viewer()).toBeNull();
  });

  it.each(['select', 'zoom', 'shape', 'text'])('paints the %s tool outline at display cadence without rerendering the composition', (tool) => {
    const context = new Proxy({} as Record<string, any>, {
      get: (target, key: string) => target[key] ??= vi.fn(),
    });
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue(context as never);
    const frames = new Map<number, FrameRequestCallback>();
    let frameId = 0;
    vi.mocked(window.requestAnimationFrame).mockImplementation(callback => { frames.set(++frameId, callback); return frameId; });
    vi.spyOn(window, 'cancelAnimationFrame').mockImplementation(id => { frames.delete(id); });
    const harness = apiHarness();
    activateExtension(harness.api);
    const body = document.createElement('div');
    harness.panel!.build!(body, { spec: {} });
    const runtime = harness.viewer() as any;
    const rect = { left: 0, top: 0, width: 1000, height: 700, right: 1000, bottom: 700, x: 0, y: 0, toJSON() {} };
    runtime.stage.getBoundingClientRect = () => rect;
    runtime.inner.getBoundingClientRect = vi.fn(() => rect);
    runtime.layout();
    for (const [id, callback] of [...frames]) { frames.delete(id); callback(0); }
    frames.clear();
    runtime.temporaryTool = tool;
    let drag: any;
    vi.mocked(harness.api.ui.drag).mockImplementation((_event, options) => { drag = options; return { cancel: () => drag.cancel() }; });
    runtime.stage.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0, clientX: 100, clientY: 100 }));
    vi.mocked(harness.api.transport.invalidate).mockClear();
    context.clearRect.mockClear();
    const positions = new Set<number>();
    for (let frame = 0; frame < 120; frame++) {
      drag.move(10 + frame / 4, 10, { clientX: 110 + frame / 4, clientY: 110 });
      positions.add((runtime.zoomRect ?? runtime.toolRect.box).x1);
      expect(frames.size).toBe(1);
      for (const [id, callback] of [...frames]) { frames.delete(id); callback(frame * 1000 / 120); }
    }
    expect(positions.size).toBe(120);
    expect(context.clearRect).toHaveBeenCalledTimes(120);
    expect(harness.api.transport.invalidate).not.toHaveBeenCalled();
    drag.cancel();
    expect(runtime.zoomRect).toBeNull();
    expect(runtime.toolRect).toBeNull();
    harness.dispose();
  });

  it('does not repeat hover hit testing while a viewer drag owns the cursor', () => {
    const harness = apiHarness();
    activateExtension(harness.api);
    const body = document.createElement('div');
    harness.panel!.build!(body, { spec: {} });
    const runtime = harness.viewer() as any;
    vi.mocked(harness.api.ui.drag).mockReturnValue({ cancel: vi.fn() });
    runtime.stage.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true, button: 0, clientX: 100, clientY: 100 }));
    const read = vi.spyOn(runtime.inner, 'getBoundingClientRect');
    for (let frame = 0; frame < 120; frame++) {
      runtime.stage.dispatchEvent(new MouseEvent('pointermove', { bubbles: true, clientX: 100 + frame / 4, clientY: 100 }));
    }
    expect(read).not.toHaveBeenCalled();
    harness.dispose();
  });

  it('keeps the original WebGL host when a panel rebuild attempts to attach a second stage', () => {
    const harness = apiHarness();
    activateExtension(harness.api);
    const first = document.createElement('div');
    const second = document.createElement('div');
    harness.panel?.build?.(first, { spec: {} });
    const stage = first.querySelector('#stage');
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    harness.panel?.build?.(second, { spec: {} });

    expect(harness.init).toHaveBeenCalledOnce();
    expect(harness.viewer()?.stage).toBe(stage);
    expect(second.querySelector('#stage')).toBe(stage);
    expect(warn).not.toHaveBeenCalled();
  });

  it('disposes listeners and rebinds replacement closures to the same live WebGL stage', () => {
    const disconnect = vi.fn();
    vi.stubGlobal('ResizeObserver', class {
      observe(): void {}
      disconnect(): void { disconnect(); }
    });
    const services = serviceHarness().services;
    const first = apiHarness(services);
    activateExtension(first.api);
    const firstBody = document.createElement('div');
    first.panel?.build?.(firstBody, { spec: {} });
    const stage = firstBody.querySelector('#stage');
    const canvas = firstBody.querySelector('#gl');
    const firstAttach = first.viewer()?.attach;

    first.dispose();
    expect(disconnect).toHaveBeenCalledOnce();
    expect(first.eventDisposers.every(dispose => dispose.mock.calls.length === 1)).toBe(true);
    expect(first.viewer()?.stage).toBe(stage);

    const replacement = apiHarness(services);
    activateExtension(replacement.api);
    const replacementBody = document.createElement('div');
    replacement.panel?.build?.(replacementBody, { spec: {} });

    expect(replacement.viewer()?.attach).not.toBe(firstAttach);
    expect(replacementBody.querySelector('#stage')).toBe(stage);
    expect(replacementBody.querySelector('#gl')).toBe(canvas);
    expect(first.init).toHaveBeenCalledOnce();
    replacement.dispose();
    expect(disconnect).toHaveBeenCalledTimes(2);
  });

  it('replaces typed event subscriptions without installing duplicate stage listeners', () => {
    const services = serviceHarness().services;
    const first = apiHarness(services);
    activateExtension(first.api);
    const body = document.createElement('div');
    first.panel?.build?.(body, { spec: {} });
    const stage = first.viewer()?.stage!;
    const removeListener = vi.spyOn(stage, 'removeEventListener');
    first.dispose();

    expect(first.eventDisposers).toHaveLength(8);
    expect(first.eventDisposers.every(dispose => dispose.mock.calls.length === 1)).toBe(true);
    expect(removeListener.mock.calls.some(([event]) => event === 'pointerdown')).toBe(true);

    const replacement = apiHarness(services);
    activateExtension(replacement.api);
    const replacementBody = document.createElement('div');
    replacement.panel?.build?.(replacementBody, { spec: {} });
    expect(replacement.viewer()?.stage).toBe(stage);
    expect(replacement.eventDisposers).toHaveLength(8);
  });
});
