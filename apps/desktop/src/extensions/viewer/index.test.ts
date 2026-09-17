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
  const init = vi.fn(() => { context = {}; return true; });
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
    menus: { collect: () => [] },
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

    expect(first.eventDisposers).toHaveLength(7);
    expect(first.eventDisposers.every(dispose => dispose.mock.calls.length === 1)).toBe(true);
    expect(removeListener.mock.calls.some(([event]) => event === 'pointerdown')).toBe(true);

    const replacement = apiHarness(services);
    activateExtension(replacement.api);
    const replacementBody = document.createElement('div');
    replacement.panel?.build?.(replacementBody, { spec: {} });
    expect(replacement.viewer()?.stage).toBe(stage);
    expect(replacement.eventDisposers).toHaveLength(7);
  });
});
