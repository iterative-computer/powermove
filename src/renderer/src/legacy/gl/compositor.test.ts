import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { effectParamValue, hasRenderableEffects, install, paramUniformName } from './compositor';

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
