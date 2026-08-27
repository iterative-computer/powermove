// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { PanelDefinition, PowermoveAPI } from 'powermove';

import activate from './index';

describe('viewer extension', () => {
  beforeEach(() => {
    vi.spyOn(window, 'requestAnimationFrame').mockImplementation(() => 1);
    vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockImplementation(() => ({} as CanvasRenderingContext2D));
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
      panels: { register: vi.fn((definition: PanelDefinition) => void (panel = definition)) }
    } as unknown as PowermoveAPI;

    activate(api);

    expect(panel).toMatchObject({
      id: 'viewer', title: 'Composition', flush: true, noscroll: true,
      headless: true, hideMoveHandle: true
    });
    expect(PM.Viewer).toBeDefined();
    expect(PM.setOrKey).toBeTypeOf('function');

    const body = document.createElement('div');
    panel?.build?.(body, { spec: {} });

    expect(body.querySelector('#stage > #stage-inner > #gl')).not.toBeNull();
    expect(body.querySelector('#stage > #overlay')).not.toBeNull();
    expect(body.querySelectorAll('#stage, #stage-inner, #gl, #overlay')).toHaveLength(4);
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
});
