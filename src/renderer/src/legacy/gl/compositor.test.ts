import { describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './compositor';

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
