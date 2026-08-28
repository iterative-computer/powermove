import { afterEach, describe, expect, it } from 'vitest';

import {
  calculateResize, install, layerContainsPoint, resizeCursorForHandle, resizeLocksAspect,
} from './viewer';

const originalWindow = (globalThis as any).window;

afterEach(() => {
  if (originalWindow === undefined) delete (globalThis as any).window;
  else (globalThis as any).window = originalWindow;
});

function viewerRegistry(): Record<string, any> {
  (globalThis as any).window = { addEventListener() {} };
  const PM: Record<string, any> = {
    h() {},
    clamp: (value: number, min: number, max: number) => Math.max(min, Math.min(max, value)),
    registerPanel() {},
    bus: { on() {} },
    GL: {},
  };
  install(PM);
  return PM;
}

describe('viewer runtime', () => {
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

  it('detects guide entry and target changes without repeating while a guide stays visible', () => {
    const V = viewerRegistry().Viewer;

    expect(V.alignmentGuideChanged(null, { x: 100, y: null })).toBe(true);
    expect(V.alignmentGuideChanged({ x: 100, y: null }, { x: 100, y: null })).toBe(false);
    expect(V.alignmentGuideChanged({ x: 100, y: null }, { x: 200, y: null })).toBe(true);
    expect(V.alignmentGuideChanged({ x: 100, y: null }, { x: 100, y: 300 })).toBe(true);
    expect(V.alignmentGuideChanged({ x: 100, y: null }, null)).toBe(false);
  });

  it('keeps click jitter below the move-drag threshold', () => {
    const V = viewerRegistry().Viewer;

    expect(V.passedMoveDragThreshold(0, 0)).toBe(false);
    expect(V.passedMoveDragThreshold(1, 1)).toBe(false);
    expect(V.passedMoveDragThreshold(2, 2)).toBe(false);
    expect(V.passedMoveDragThreshold(3, 0)).toBe(true);
  });

  it('measures rotated layer bounds in composition space', () => {
    const PM = viewerRegistry();
    PM.GL.bounds = () => ({ x0: -10, y0: -20, x1: 10, y1: 20 });
    PM.worldMatrix = () => [0, 1, -1, 0, 100, 200];

    expect(PM.Viewer.worldBounds({}, 0)).toEqual({ x0: 80, x1: 120, y0: 190, y1: 210, cx: 100, cy: 200 });
  });

  it('recognizes the selected layer box beneath a full-frame top layer', () => {
    const PM = viewerRegistry();
    const selected = { id: 'text' };
    PM.GL.bounds = (layer: any) => layer === selected
      ? { x0: -100, y0: -25, x1: 100, y1: 25 }
      : { x0: 0, y0: 0, x1: 1920, y1: 1080 };
    PM.worldMatrix = (layer: any) => layer === selected
      ? [1, 0, 0, 1, 960, 800]
      : [1, 0, 0, 1, 0, 0];

    expect(layerContainsPoint(PM, selected, 960, 800, 0)).toBe(true);
    expect(layerContainsPoint(PM, selected, 700, 800, 0)).toBe(false);
  });

  it('resizes edge handles on one axis and preserves the opposite midpoint', () => {
    const bounds = { x0: -100, y0: -50, x1: 100, y1: 50, w: 200, h: 100 };

    expect(calculateResize(
      bounds, [1, .5], { x: 200, y: 0 }, { x: 0, y: 0 }, { x: 100, y: 80 },
    )).toEqual({ scaleX: 150, scaleY: 80, pivotLocal: { x: -100, y: 0 } });

    expect(calculateResize(
      bounds, [.5, 0], { x: 0, y: -100 }, { x: 0, y: 0 }, { x: 100, y: 80 },
    )).toEqual({ scaleX: 100, scaleY: 120, pivotLocal: { x: 0, y: 50 } });
  });

  it('preserves the exact grab offset so resize does not jump on pointer down', () => {
    const bounds = { x0: 0, y0: 0, x1: 200, y1: 100, w: 200, h: 100 };
    const next = calculateResize(
      bounds, [1, 1], { x: 205, y: 96 }, { x: 5, y: -4 }, { x: 125, y: 75 },
    );

    expect(next).toEqual({ scaleX: 125, scaleY: 75, pivotLocal: { x: 0, y: 0 } });
  });

  it('projects Shift-constrained corners continuously onto their diagonal', () => {
    const bounds = { x0: 0, y0: 0, x1: 200, y1: 100, w: 200, h: 100 };
    const next = calculateResize(
      bounds, [1, 1], { x: 300, y: 100 }, { x: 0, y: 0 }, { x: 100, y: 100 },
      { lockAspect: true },
    );

    expect(next.scaleX).toBeCloseTo(140);
    expect(next.scaleY).toBeCloseTo(140);
    expect(next.pivotLocal).toEqual({ x: 0, y: 0 });
  });

  it('uses the center as the fixed point for Option-resize', () => {
    const bounds = { x0: 0, y0: 0, x1: 200, y1: 100, w: 200, h: 100 };
    const next = calculateResize(
      bounds, [1, .5], { x: 250, y: 50 }, { x: 0, y: 0 }, { x: 100, y: 100 },
      { fromCenter: true },
    );

    expect(next).toEqual({ scaleX: 150, scaleY: 100, pivotLocal: { x: 100, y: 50 } });
  });

  it('never permits canvas handles to stretch text or intrinsic media', () => {
    expect(resizeLocksAspect('text')).toBe(true);
    expect(resizeLocksAspect('image')).toBe(true);
    expect(resizeLocksAspect('video')).toBe(true);
    expect(resizeLocksAspect('shape')).toBe(false);
    expect(resizeLocksAspect('shape', true)).toBe(true);
  });

  it('orients every resize cursor with the transformed selection', () => {
    const identity = [1, 0, 0, 1, 0, 0] as const;
    expect(resizeCursorForHandle(identity, 'e')).toBe('ew-resize');
    expect(resizeCursorForHandle(identity, 'n')).toBe('ns-resize');
    expect(resizeCursorForHandle(identity, 'nw')).toBe('nwse-resize');
    expect(resizeCursorForHandle(identity, 'ne')).toBe('nesw-resize');

    const rotated90 = [0, 1, -1, 0, 0, 0] as const;
    expect(resizeCursorForHandle(rotated90, 'e')).toBe('ns-resize');
    expect(resizeCursorForHandle(rotated90, 'n')).toBe('ew-resize');
  });
});
