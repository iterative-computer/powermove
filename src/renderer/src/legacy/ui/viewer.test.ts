import { afterEach, describe, expect, it } from 'vitest';

import type { PMRegistry } from '../registry';
import { install } from './viewer';

const originalWindow = (globalThis as any).window;

afterEach(() => {
  if (originalWindow === undefined) delete (globalThis as any).window;
  else (globalThis as any).window = originalWindow;
});

function viewerRegistry(): PMRegistry {
  (globalThis as any).window = { addEventListener() {} };
  const PM: PMRegistry = {
    h() {},
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    registerPanel() {},
    bus: { on() {} },
    GL: {},
  };
  install(PM);
  return PM;
}

describe('legacy viewer install', () => {
  it('chooses the closest snap within the screen-space threshold', () => {
    const V = viewerRegistry().Viewer;

    expect(V.snapAxis([93, 103, 113], [{ value: 0 }, { value: 100 }, { value: 200 }], 8))
      .toEqual({ delta: -3, value: 100, distance: 3 });
    expect(V.snapAxis([30, 40, 50], [100], 8)).toBeNull();
  });

  it('resolves alignment guides independently and respects a locked axis', () => {
    const V = viewerRegistry().Viewer;
    const bounds = { x0: 193, cx: 203, x1: 213, y0: 42, cy: 52, y1: 62 };

    expect(V.alignmentSnap(bounds, { x: [0, 200, 400], y: [0, 50, 300] }, 6))
      .toEqual({
        dx: -3,
        dy: -2,
        x: { delta: -3, value: 200, distance: 3 },
        y: { delta: -2, value: 50, distance: 2 },
      });
    expect(V.alignmentSnap(bounds, { x: [200], y: [50] }, 6, { x: true, y: false }))
      .toEqual({ dx: -3, dy: 0, x: { delta: -3, value: 200, distance: 3 }, y: null });
  });

  it('measures rotated layer bounds in composition space', () => {
    const PM = viewerRegistry();
    PM.GL.bounds = () => ({ x0: -10, y0: -20, x1: 10, y1: 20 });
    PM.worldMatrix = () => [0, 1, -1, 0, 100, 200];

    expect(PM.Viewer.worldBounds({}, 0)).toEqual({ x0: 80, x1: 120, y0: 190, y1: 210, cx: 100, cy: 200 });
  });
});
