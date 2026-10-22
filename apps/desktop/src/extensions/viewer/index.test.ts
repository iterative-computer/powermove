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
    project: { get: () => project, revision: () => 1 },
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

  function libraryClone(width = 1280, height = 720): { source: HTMLElement; clone: HTMLElement; inner: HTMLElement; overlay: HTMLCanvasElement } {
    const source = document.createElement('div');
    const clone = document.createElement('div');
    // A clone of the live viewer, panned: the overlay is a stage-sized sibling.
    clone.innerHTML = `<div data-library-source-id="stage"><div data-library-source-id="stage-inner" style="width: ${width}px; height: ${height}px; left: -213.5px; top: -97.25px;"><canvas data-library-source-id="gl" style="position: absolute; left: 192px; top: 80px; width: 704px; height: 528px;"></canvas></div><canvas data-library-source-id="overlay" width="640" height="480" style="width: 640px; height: 480px;"></canvas></div>`;
    document.body.append(source, clone);
    return {
      source, clone,
      inner: clone.querySelector('[data-library-source-id="stage-inner"]')!,
      overlay: clone.querySelector('[data-library-source-id="overlay"]')!,
    };
  }

  it('renders the Library viewer card at the live render size through the public async render API', async () => {
    vi.stubGlobal('devicePixelRatio', 1);
    const harness = apiHarness();
    harness.api.transport.previewResolution = 'auto';
    const frame = document.createElement('canvas'); frame.width = 1280; frame.height = 720;
    const drawImage = vi.fn();
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue({ drawImage } as never);
    harness.api.services.register('engine.viewer', { exclusive: true });
    const renderFrameToAsync = vi.fn(async (..._args: unknown[]) => frame);
    Object.assign(harness.api.render, { renderFrameToAsync });
    activateExtension(harness.api);
    const { source, clone, inner, overlay } = libraryClone();
    const innerStyle = inner.getAttribute('style');
    const overlayBefore = overlay.outerHTML;

    const render = (harness.panel!.library as any).render;
    expect(render.enginePreview).toBe(true);
    render({ source, clone, width: 640, height: 480 });
    await vi.waitFor(() => expect(drawImage).toHaveBeenCalledWith(frame, 0, 0));
    // The full 1920×1080 composition at the live 2/3 zoom, the paused viewer's blur, no draft 3D.
    expect(renderFrameToAsync).toHaveBeenCalledWith(0, 1280, 720, { mblur: true, mbSamples: 12, priority: 'idle' });
    const cover = inner.querySelector<HTMLCanvasElement>(':scope > canvas[data-library-engine-frame]')!;
    expect([cover.style.left, cover.style.top, cover.style.width, cover.style.height]).toEqual(['0px', '0px', '100%', '100%']);
    expect(inner.getAttribute('style')).toBe(innerStyle);
    expect(overlay.outerHTML).toBe(overlayBefore);
    harness.dispose();
    vi.unstubAllGlobals();
  });

  it.each([
    ['retina 100%', 1920, 2, 1, 'auto', [1920, 1080]],
    ['retina 400%', 7680, 2, 1, 'auto', [1920, 1080]],
    ['retina 800%', 15360, 2, 1, 'auto', [1920, 1080]],
    ['retina fit 25%', 480, 2, 1, 'auto', [960, 540]],
    ['half quality', 1920, 2, 0.5, 'auto', [960, 540]],
    ['quarter quality', 1920, 2, 0.25, 'auto', [480, 270]],
    ['fixed full resolution at fit 25%', 480, 1, 1, '1', [1920, 1080]],
    ['fixed half resolution at 400%', 7680, 2, 0.5, '0.5', [960, 540]],
  ] as const)('sizes the Library card like the live viewer: %s', async (_, innerWidth, dpr, quality, resolution, [width, height]) => {
    vi.stubGlobal('devicePixelRatio', dpr);
    const harness = apiHarness();
    Object.assign(harness.api.transport, { quality, previewResolution: resolution });
    const drawImage = vi.fn();
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue({ drawImage } as never);
    harness.api.services.register('engine.viewer', { exclusive: true });
    const frame = document.createElement('canvas'); frame.width = width; frame.height = height;
    const renderFrameToAsync = vi.fn(async (..._args: unknown[]) => frame);
    Object.assign(harness.api.render, { renderFrameToAsync });
    activateExtension(harness.api);
    const { source, clone, inner } = libraryClone(innerWidth, innerWidth * 9 / 16);
    const innerStyle = inner.getAttribute('style');
    (harness.panel!.library as any).render({ source, clone, width: 640, height: 480 });
    await vi.waitFor(() => expect(drawImage).toHaveBeenCalledWith(frame, 0, 0));
    expect(renderFrameToAsync).toHaveBeenCalledWith(0, width, height, { mblur: true, mbSamples: 12, priority: 'idle' });
    // The cover still stretches over the clone's own stage-inner box.
    const cover = inner.querySelector<HTMLCanvasElement>(':scope > canvas[data-library-engine-frame]')!;
    expect([cover.width, cover.height]).toEqual([width, height]);
    expect([cover.style.left, cover.style.top, cover.style.width, cover.style.height]).toEqual(['0px', '0px', '100%', '100%']);
    expect(inner.getAttribute('style')).toBe(innerStyle);
    harness.dispose();
    vi.unstubAllGlobals();
  });

  it('finishes a Library card the Library detached while its render was pending', async () => {
    const harness = apiHarness();
    harness.api.transport.previewResolution = 'auto';
    const drawImage = vi.fn();
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue({ drawImage } as never);
    harness.api.services.register('engine.viewer', { exclusive: true });
    const frame = document.createElement('canvas');
    let reply!: (frame: HTMLCanvasElement) => void;
    const renderFrameToAsync = vi.fn(() => new Promise<HTMLCanvasElement>((resolve) => { reply = resolve; }));
    Object.assign(harness.api.render, { renderFrameToAsync });
    activateExtension(harness.api);
    const { source, clone, inner } = libraryClone();
    (harness.panel!.library as any).render({ source, clone, width: 640, height: 480 });
    await vi.waitFor(() => expect(renderFrameToAsync).toHaveBeenCalledTimes(1));
    // A shelf switch or search keystroke detaches the cached card before the frame arrives.
    clone.remove();
    reply(frame);
    await vi.waitFor(() => expect(drawImage).toHaveBeenCalledWith(frame, 0, 0));
    expect(inner.querySelector(':scope > canvas[data-library-engine-frame]')).not.toBeNull();
    // Coming back shows the cached clone as is: nothing renders again.
    document.body.append(clone);
    await new Promise((resolve) => setTimeout(resolve, 250));
    expect(renderFrameToAsync).toHaveBeenCalledTimes(1);
    expect(drawImage).toHaveBeenCalledTimes(1);
    harness.dispose();
  });

  it('drops a Library card frame from an older revision and renders again', async () => {
    const harness = apiHarness();
    const drawImage = vi.fn();
    vi.mocked(HTMLCanvasElement.prototype.getContext).mockReturnValue({ drawImage } as never);
    harness.api.services.register('engine.viewer', { exclusive: true });
    let revision = 1;
    Object.assign(harness.api.project, { revision: () => revision });
    const frames = [document.createElement('canvas'), document.createElement('canvas')];
    const renderFrameToAsync = vi.fn(async () => {
      const frame = frames[renderFrameToAsync.mock.calls.length - 1]!;
      if (renderFrameToAsync.mock.calls.length === 1) revision = 2;
      return frame;
    });
    Object.assign(harness.api.render, { renderFrameToAsync });
    activateExtension(harness.api);
    const { source, clone } = libraryClone();
    (harness.panel!.library as any).render({ source, clone, width: 640, height: 480 });
    await vi.waitFor(() => expect(drawImage).toHaveBeenCalled(), { timeout: 2000 });
    expect(renderFrameToAsync).toHaveBeenCalledTimes(2);
    expect(drawImage).toHaveBeenCalledTimes(1);
    expect(drawImage).toHaveBeenCalledWith(frames[1], 0, 0);
    harness.dispose();
  });

  it('leaves the Library card alone without the exclusive engine viewer or the known #gl', async () => {
    const harness = apiHarness();
    const renderFrameToAsync = vi.fn(async () => document.createElement('canvas'));
    Object.assign(harness.api.render, { renderFrameToAsync });
    activateExtension(harness.api);
    const render = (harness.panel!.library as any).render;
    const { source, clone } = libraryClone();
    render({ source, clone, width: 640, height: 480 });
    harness.api.services.register('engine.viewer', { exclusive: true });
    const custom = document.createElement('div');
    custom.innerHTML = '<div data-library-source-id="stage"><canvas class="ruler"></canvas><div data-library-source-id="stage-inner" style="width: 640px; height: 360px;"><canvas class="custom"></canvas></div></div>';
    document.body.append(custom);
    render({ source, clone: custom, width: 640, height: 480 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(renderFrameToAsync).not.toHaveBeenCalled();
    expect(document.querySelector('[data-library-engine-frame]')).toBeNull();
    harness.dispose();
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
