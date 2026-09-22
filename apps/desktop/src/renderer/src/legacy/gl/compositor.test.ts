import { describe, expect, it, vi } from 'vitest';

import type { PMRegistry } from '../registry';
import {
  compositingLevel, continuousRasterScale, effectParamValue, groupContainsSolo,
  hasRenderableEffects, install, paramUniformName, svgRasterDimensions, trackPresentedVideoFrames,
} from './compositor';

function compositorRegistry(): PMRegistry {
  const PM: PMRegistry = {
    proj: { w: 1920, h: 1080, layers: [] },
    TYPE_META: {},
    active: (item: any, time: any) => time >= item.from && time < item.from + item.dur,
    worldMatrix: () => [1, 0, 0, 1, 0, 0],
    raster: () => ({
      w: 500,
      h: 300,
      anchorX: 250,
      anchorY: 150,
      selection: { x0: -180, y0: -42, x1: 180, y1: 42, w: 360, h: 84 },
    }),
  };
  install(PM);
  return PM;
}

describe('legacy compositor install', () => {
  it('gives transparent null objects a compact canvas hit target', () => {
    const PM = compositorRegistry();
    expect(PM.GL.bounds({ type: 'null', d: { w: 100, h: 100 } }, 0)).toEqual({
      x0: -0, y0: -0, x1: 100, y1: 100, w: 100, h: 100, ax: 0, ay: 0
    });
  });

  it('measures primitive selection bounds without allocating a source bitmap', () => {
    const PM = compositorRegistry();
    PM.raster = vi.fn(() => { throw new Error('Bounds must not rasterize a primitive'); });
    const bounds = PM.GL.bounds({ type: 'shape', d: { w: 3000, h: 2000, stroke: 10 } }, 0);
    expect(bounds).toEqual({ x0: -1505, y0: -1005, x1: 1505, y1: 1005, w: 3010, h: 2010, ax: 1509, ay: 1009 });
    expect(PM.raster).not.toHaveBeenCalled();
  });

  it('continuously rasterizes editable vector sources at their displayed scale', () => {
    expect(continuousRasterScale([4, 0, 0, 4, 0, 0], 1)).toBe(4);
    expect(continuousRasterScale([0, 3, -3, 0, 0, 0], 1)).toBe(3);
    expect(continuousRasterScale([4, 0, 0, 4, 0, 0], .5)).toBe(2);
    expect(continuousRasterScale([.25, 0, 0, .25, 0, 0], 1)).toBe(.25);
    expect(continuousRasterScale([32, 0, 0, 32, 0, 0], 1)).toBe(32);
    expect(continuousRasterScale([64, 0, 0, 64, 0, 0], 1)).toBe(32);
  });

  it('sizes SVG backing textures from their displayed box without changing source aspect ratio', () => {
    expect(svgRasterDimensions(100, 50, 400, 200, [2, 0, 0, 2, 0, 0])).toEqual({
      width: 800,
      height: 400,
      scale: 8,
    });
    expect(svgRasterDimensions(100, 50, 400, 400, [1, 0, 0, 1, 0, 0])).toEqual({
      width: 800,
      height: 400,
      scale: 8,
    });
    const capped = svgRasterDimensions(100, 50, 20_000, 10_000, [1, 0, 0, 1, 0, 0]);
    expect(capped).toEqual({ width: 8192, height: 4096, scale: 81.92 });
  });

  it('invalidates a paused seek as soon as the decoded frame is available', () => {
    const video = new EventTarget() as any;
    video.requestVideoFrameCallback = vi.fn();
    const invalidated = vi.fn();
    const state = trackPresentedVideoFrames(video, invalidated);
    video.dispatchEvent(new Event('seeked'));
    expect(state.version).toBe(1);
    expect(invalidated).toHaveBeenCalledOnce();
  });

  it('versions textures from each frame the browser presents', () => {
    const callbacks: Array<() => void> = [];
    const invalidated = vi.fn();
    const video = {
      requestVideoFrameCallback(callback: () => void) {
        callbacks.push(callback);
        return callbacks.length;
      },
    };

    const state = trackPresentedVideoFrames(video, invalidated);
    expect(state).toMatchObject({ version: 0, supported: true });
    callbacks.shift()!();
    expect(state.version).toBe(1);
    expect(invalidated).toHaveBeenCalledOnce();
    expect(callbacks).toHaveLength(1);
  });

  it('uses tight text selection bounds and keeps the background fill shader path', () => {
    const PM = compositorRegistry();
    const text = { type: 'text', d: {} };
    const bounds = PM.GL.bounds(text, 0);

    expect(bounds.h).toBeLessThan(300 * .5);
    expect(bounds.w).toBeLessThan(500);
    expect(bounds.w).toBeGreaterThan(350);
    expect(Math.abs(bounds.x0 + bounds.x1)).toBeLessThan(1);
    expect(bounds.ax).toBe(250);
    expect(bounds.ay).toBe(150);
    expect(String(PM.GL.renderProject)).toContain('PM.FRAG_BACKGROUND_FILL');
  });

  it('routes adjustment layers through the accumulated lower-layer image', () => {
    const PM = compositorRegistry();
    const source = String(PM.GL.renderProject);

    expect(source).toMatch(/L\.type === ["']adjustment["']/);
    expect(source).toContain('compositeAdjustment(L, T, acc, W, H, alpha, hasMasks, blend)');
  });

  it('builds nested compositing levels and retains a group containing the solo layer', () => {
    const outer = { id: 'outer', type: 'group', group: null };
    const child = { id: 'child', type: 'shape', group: 'outer' };
    const inner = { id: 'inner', type: 'group', group: 'outer' };
    const solo = { id: 'solo', type: 'text', group: 'inner', solo: true };
    const top = { id: 'top', type: 'shape', group: null };
    const layers = [outer, child, inner, solo, top];
    const ancestors = (layer: any) => layer === solo ? [inner, outer]
      : layer === child || layer === inner ? [outer] : [];

    expect(compositingLevel(layers, null)).toEqual([outer, top]);
    expect(compositingLevel(layers, 'outer')).toEqual([child, inner]);
    expect(compositingLevel(layers, 'inner')).toEqual([solo]);
    expect(groupContainsSolo(outer, layers, ancestors)).toBe(true);
    expect(groupContainsSolo(inner, layers, ancestors)).toBe(true);
    expect(groupContainsSolo(top, layers, ancestors)).toBe(false);
  });

  it('renders group contents into their own effect, mask, blend, matte, motion-blur and transition pass', () => {
    const PM = compositorRegistry();
    const source = String(PM.GL.renderProject);

    expect(source).toMatch(/L\.type !== ["']group["']/);
    expect(source).toContain('groupParent: L.id');
    expect(source).toMatch(/runEffects\(L, T, res, W, H, acc\)/);
    expect(source).toMatch(/applyMasks\(L, T, res, W, H\)/);
    expect(source).toMatch(/applyTrackMatte\(L, T, res, W, H/);
    expect(source).toMatch(/compositeAlpha = L\.type === ["']group["'] \? alpha : 1/);
  });
});

describe('effect and transition parameter uniforms', () => {
  /* Kernel-generated shaders declare `u_<param>`; legacy raw shaders declare
     the positional `u_p<i>` / `u_c<i>` names. Bind by name first, fall back. */
  it('prefers the named uniform when the program declares it', () => {
    expect(paramUniformName({ k: 'amount' }, 0, true)).toBe('u_amount');
    expect(paramUniformName({ k: 'shadow', type: 'color' }, 4, true)).toBe('u_shadow');
  });

  it('falls back to the positional name for legacy raw shaders', () => {
    expect(paramUniformName({ k: 'amount' }, 0, false)).toBe('u_p0');
    expect(paramUniformName({ k: 'angle' }, 1, false)).toBe('u_p1');
    expect(paramUniformName({ k: 'shadow', type: 'color' }, 4, false)).toBe('u_c4');
    expect(paramUniformName({ k: 'on', type: 'toggle' }, 2, false)).toBe('u_p2');
  });
});

describe('compositor effect fast path', () => {
  it('ignores enabled missing-effect placeholders when choosing the fast path', () => {
    expect(hasRenderableEffects([{ on: true, missing: true }])).toBe(false);
    expect(hasRenderableEffects([
      { on: true, missing: true },
      { on: false, missing: false }
    ])).toBe(false);
    expect(hasRenderableEffects([
      { on: true, missing: true },
      { on: true, missing: false }
    ])).toBe(true);
  });

  it('starts a fresh evaluator window for pixel export and defaults absent effect channels', () => {
    const PM = compositorRegistry();
    PM.evP = () => 42;
    expect(String(PM.GL.renderToPixels)).toContain('PM.beginEval(T)');
    expect(effectParamValue(PM, {}, { p: {} }, { k: 'amount', def: 12 }, 0)).toBe(12);
    expect(effectParamValue(PM, {}, { p: { amount: { v: 42 } } }, { k: 'amount', def: 12 }, 0)).toBe(42);
  });
});
