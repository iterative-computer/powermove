// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, PowermoveAPI } from 'powermove';

import activate from './index';

const testSpace3d = { is3DLayer: () => false } as unknown as PowermoveAPI['space3d'];

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
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it('registers and builds the composition panel through the extension API', () => {
    let panel: PanelDefinition | undefined;
    const PM: Record<string, any> = {
      clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
      bus: { on: vi.fn(() => () => {}) },
      GL: { gl: null, init: vi.fn(), resize: vi.fn() },
      proj: { w: 1920, h: 1080, layers: [] },
      quality: 1,
      invalidate: vi.fn()
    };
    const api = {
      host: { pm: PM },
      space3d: testSpace3d,
      onDispose: vi.fn(),
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI;

    activate(api);

    expect(panel).toMatchObject({
      id: 'viewer', title: 'Composition', flush: true, noscroll: true,
      headless: true, hideMoveHandle: false
    });
    expect(PM.Viewer).toBeDefined();
    expect(PM.setOrKey).toBeTypeOf('function');

    const body = document.createElement('div');
    panel?.build?.(body, { spec: {} });

    expect(body.querySelector('#stage > #stage-inner > #gl')).not.toBeNull();
    expect(body.querySelector('#stage > #overlay')).not.toBeNull();
    expect(body.querySelector('#stage > #composition-recovery')?.textContent).toContain('Fit composition');
    expect(body.querySelector<HTMLButtonElement>('#composition-recovery')?.hidden).toBe(true);
    expect(body.querySelectorAll('#stage, #stage-inner, #gl, #overlay, #composition-recovery')).toHaveLength(5);
    expect(PM.GL.init).toHaveBeenCalledWith(body.querySelector('#gl'));
    expect(PM.Viewer.ov).toBe(body.querySelector('#overlay'));
  });

  it('keeps the original WebGL host when a panel rebuild attempts to attach a second stage', () => {
    let panel: PanelDefinition | undefined;
    const PM: Record<string, any> = {
      clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
      bus: { on: vi.fn(() => () => {}) },
      GL: { gl: null, init: vi.fn(), resize: vi.fn() },
      proj: { w: 1920, h: 1080, layers: [] },
      quality: 1,
      invalidate: vi.fn()
    };
    activate({
      host: { pm: PM },
      space3d: testSpace3d,
      onDispose: vi.fn(),
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI);

    const first = document.createElement('div');
    const second = document.createElement('div');
    panel?.build?.(first, { spec: {} });
    const stage = first.querySelector('#stage');
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    panel?.build?.(second, { spec: {} });

    expect(PM.GL.init).toHaveBeenCalledOnce();
    expect(PM.Viewer.stage).toBe(stage);
    expect(second.querySelector('#stage')).toBe(stage);
    expect(warn).not.toHaveBeenCalled();
  });

  it('disposes listeners and rebinds replacement closures to the same live WebGL stage', () => {
    const disconnect = vi.fn();
    vi.stubGlobal('ResizeObserver', class {
      observe(): void {}
      disconnect(): void { disconnect(); }
    });
    const busOffs: Array<ReturnType<typeof vi.fn>> = [];
    let firstPanel: PanelDefinition | undefined;
    let firstDispose: (() => void) | undefined;
    const GL: Record<string, any> = { gl: null, init: vi.fn(), resize: vi.fn() };
    GL.init.mockImplementation(() => { GL.gl = {}; });
    const PM: Record<string, any> = {
      clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
      bus: { on: vi.fn(() => {
        const off = vi.fn(); busOffs.push(off); return off;
      }) },
      GL,
      proj: { w: 1920, h: 1080, layers: [] },
      quality: 1,
      invalidate: vi.fn()
    };
    activate({
      host: { pm: PM },
      space3d: testSpace3d,
      onDispose: vi.fn((dispose: () => void) => void (firstDispose = dispose)),
      panels: { register: vi.fn((definition: PanelDefinition) => void (firstPanel = definition)) }
    } as unknown as PowermoveAPI);
    const firstBody = document.createElement('div');
    firstPanel?.build?.(firstBody, { spec: {} });
    const stage = firstBody.querySelector('#stage');
    const canvas = firstBody.querySelector('#gl');
    const firstAttach = PM.Viewer.attach;

    firstDispose?.();
    expect(disconnect).toHaveBeenCalledOnce();
    expect(busOffs.every((off) => off.mock.calls.length === 1)).toBe(true);
    expect(PM.Viewer.stage).toBe(stage);
    expect(PM.Viewer.el).toBe(canvas);

    let replacementPanel: PanelDefinition | undefined;
    let replacementDispose: (() => void) | undefined;
    activate({
      host: { pm: PM },
      space3d: testSpace3d,
      onDispose: vi.fn((dispose: () => void) => void (replacementDispose = dispose)),
      panels: { register: vi.fn((definition: PanelDefinition) => void (replacementPanel = definition)) }
    } as unknown as PowermoveAPI);
    const replacementBody = document.createElement('div');
    replacementPanel?.build?.(replacementBody, { spec: {} });

    expect(PM.Viewer.attach).not.toBe(firstAttach);
    expect(replacementBody.querySelector('#stage')).toBe(stage);
    expect(replacementBody.querySelector('#gl')).toBe(canvas);
    expect(PM.GL.init).toHaveBeenCalledOnce();
    replacementDispose?.();
    expect(disconnect).toHaveBeenCalledTimes(2);
  });

  it('keeps the legacy-listener fence and removes only the stale overlay closure across replacements', () => {
    const stage = document.createElement('div');
    stage.id = 'stage';
    const inner = document.createElement('div'); inner.id = 'stage-inner';
    const canvas = document.createElement('canvas'); canvas.id = 'gl'; inner.append(canvas);
    const overlay = document.createElement('canvas'); overlay.id = 'overlay';
    stage.append(inner, overlay);
    const addListener = vi.spyOn(stage, 'addEventListener');
    const legacyOverlay = function drawOverlay(): void {};
    const unrelatedOverlay = function extensionOverlay(): void {};
    const handlers = new Map<string, Set<(...args: any[]) => void>>([
      ['overlay', new Set([legacyOverlay, unrelatedOverlay])],
    ]);
    const PM: Record<string, any> = {
      Viewer: { stage, zoom: 1, fit: true, pan: [0, 0], attach() {} },
      clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
      bus: {
        m: handlers,
        on(event: string, handler: (...args: any[]) => void) {
          const set = handlers.get(event) || new Set(); handlers.set(event, set); set.add(handler);
          return () => set.delete(handler);
        },
      },
      GL: { gl: {}, init: vi.fn(), resize: vi.fn() },
      proj: { w: 1920, h: 1080, layers: [] }, quality: 1, invalidate: vi.fn(),
    };
    let firstDispose: (() => void) | undefined;
    activate({
      host: { pm: PM },
      space3d: testSpace3d,
      onDispose: vi.fn((dispose: () => void) => void (firstDispose = dispose)),
      panels: { register: vi.fn() },
    } as unknown as PowermoveAPI);
    expect(handlers.get('overlay')?.has(legacyOverlay)).toBe(false);
    expect(handlers.get('overlay')?.has(unrelatedOverlay)).toBe(true);
    expect(addListener.mock.calls.find(([event]) => event === 'pointerdown')?.[2]).toBe(true);

    firstDispose?.();
    addListener.mockClear();
    activate({
      host: { pm: PM }, space3d: testSpace3d, onDispose: vi.fn(), panels: { register: vi.fn() },
    } as unknown as PowermoveAPI);
    expect(addListener.mock.calls.find(([event]) => event === 'pointerdown')?.[2]).toBe(true);
  });
});
