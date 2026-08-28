import { afterEach, describe, expect, it } from 'vitest';

import {
  calculateResize, composeLocalLinear, install, layerContainsPoint, layerWorldPivot,
  localRotationForWorldDirection, multiplyLinear, resolveSelectionGeometry, resizeCursorForHandle,
  resizeLocksAspect, rotateLinear, selectionTransformRoots, solveLocalTransformForWorldLinear,
  transformPointAround,
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

  it('filters transform roots so selected descendants never transform twice', () => {
    const parent = { id: 'parent', parent: null, type: 'shape' };
    const child = { id: 'child', parent: 'parent', type: 'shape' };
    const sibling = { id: 'sibling', parent: null, type: 'shape' };
    const locked = { id: 'locked', parent: null, type: 'shape', lock: true };
    const inactive = { id: 'inactive', parent: null, type: 'shape', active: false };
    const audio = { id: 'audio', parent: null, type: 'audio' };
    const layers = [parent, child, sibling, locked, inactive, audio];
    const PM = {
      active: (layer: any) => layer.active !== false,
      TYPE_META: { shape: {}, audio: { pickable: false } },
      L: (id: string) => layers.find((layer) => layer.id === id),
    };

    expect(selectionTransformRoots(PM, layers, 0).map((layer) => layer.id))
      .toEqual(['parent', 'sibling']);
  });

  it('resolves multiple layers to one axis-aligned common selection box', () => {
    const left = { id: 'left', type: 'shape', parent: null };
    const right = { id: 'right', type: 'shape', parent: null };
    const PM = {
      active: () => true, TYPE_META: { shape: {} }, L: () => null,
      GL: { bounds: () => ({ x0: -10, y0: -5, x1: 10, y1: 5, w: 20, h: 10 }) },
      worldMatrix: (layer: any) => layer === left
        ? [1, 0, 0, 1, 100, 120]
        : [0, 1, -1, 0, 220, 180],
      ev: () => 0,
    };

    const selection = resolveSelectionGeometry(PM, [left, right], 0);
    expect(selection?.mode).toBe('common');
    expect(selection?.roots).toEqual([left, right]);
    expect(selection?.bounds).toEqual({ x0: 90, y0: 115, x1: 225, y1: 190, w: 135, h: 75 });
    expect(selection?.handles.nw).toEqual({ x: 90, y: 115 });
    expect(selection?.handles.se).toEqual({ x: 225, y: 190 });
  });

  it('keeps locked members in common chrome while disabling the whole transform', () => {
    const left = { id: 'left', type: 'shape', parent: null };
    const locked = { id: 'locked', type: 'shape', parent: null, lock: true };
    const PM = {
      active: () => true, TYPE_META: { shape: {} }, L: () => null,
      GL: { bounds: () => ({ x0: -10, y0: -10, x1: 10, y1: 10, w: 20, h: 20 }) },
      worldMatrix: (layer: any) => [1, 0, 0, 1, layer === left ? 50 : 150, 100],
      ev: () => 0,
    };

    const selection = resolveSelectionGeometry(PM, [left, locked], 0);
    expect(selection?.layers).toEqual([left, locked]);
    expect(selection?.bounds).toEqual({ x0: 40, y0: 90, x1: 160, y1: 110, w: 120, h: 20 });
    expect(selection?.transformable).toBe(false);
    expect(selection?.roots).toEqual([]);
  });

  it('uses the transformed anchor as the true world rotation pivot', () => {
    const layer = { id: 'anchored' };
    const PM = {
      worldMatrix: () => [0, 1, -1, 0, 100, 200],
      ev: (_layer: any, path: string) => path === 'anchor.x' ? 10 : 20,
    };

    expect(layerWorldPivot(PM, layer, 0)).toEqual({ x: 80, y: 210 });
  });

  it('scales and rotates relative positions around the common fixed pivot', () => {
    expect(transformPointAround({ x: 30, y: 20 }, { x: 10, y: 10 }, 2, 2))
      .toEqual({ x: 50, y: 30 });
    const turned = transformPointAround({ x: 30, y: 10 }, { x: 10, y: 10 }, 1, 1, 90);
    expect(turned.x).toBeCloseTo(10);
    expect(turned.y).toBeCloseTo(30);
  });

  it('derives local rotation through a skewed non-uniform parent inverse', () => {
    const parent = [2, .5, .8, .75, 0, 0] as const;
    const angle = 30 * Math.PI / 180;
    const local = { x: Math.cos(angle), y: Math.sin(angle) };
    const world = {
      x: parent[0] * local.x + parent[2] * local.y,
      y: parent[1] * local.x + parent[3] * local.y,
    };
    expect(localRotationForWorldDirection(parent, world)).toBeCloseTo(30);
  });

  it('solves the complete rigid world rotation through a skewed non-uniform parent', () => {
    const parent = [1.7, .4, .6, .8] as const;
    const original = composeLocalLinear({ rotation: 17, scaleX: 1.4, scaleY: .65, skew: 23 });
    const world = multiplyLinear(parent, original);
    const desired = rotateLinear(world, 43);
    const solved = solveLocalTransformForWorldLinear(parent, desired);
    expect(solved).not.toBeNull();
    const actual = multiplyLinear(parent, composeLocalLinear(solved!));
    actual.forEach((value, index) => expect(value).toBeCloseTo(desired[index]!, 7));
  });

  it('preserves signed scale and refuses singular matrix decomposition safely', () => {
    const signed = { rotation: -28, scaleX: 1.2, scaleY: -.7, skew: 31 };
    const matrix = composeLocalLinear(signed);
    const solved = solveLocalTransformForWorldLinear(null, matrix);
    expect(solved?.scaleX).toBeCloseTo(signed.scaleX);
    expect(solved?.scaleY).toBeCloseTo(signed.scaleY);
    expect(solved?.skew).toBeCloseTo(signed.skew);
    composeLocalLinear(solved!).forEach((value, index) => expect(value).toBeCloseTo(matrix[index]!, 7));
    const flipped = { rotation: 22, scaleX: -1.3, scaleY: .8, skew: -15 };
    const flippedMatrix = composeLocalLinear(flipped);
    const flippedSolved = solveLocalTransformForWorldLinear(null, flippedMatrix, {
      scaleX: flipped.scaleX, rotation: flipped.rotation,
    });
    expect(flippedSolved?.rotation).toBeCloseTo(flipped.rotation);
    expect(flippedSolved?.scaleX).toBeCloseTo(flipped.scaleX);
    expect(flippedSolved?.scaleY).toBeCloseTo(flipped.scaleY);
    expect(flippedSolved?.skew).toBeCloseTo(flipped.skew);
    expect(solveLocalTransformForWorldLinear([1, 0, 0, 0], matrix)).toBeNull();
    expect(solveLocalTransformForWorldLinear(null, [0, 0, 1, 1])).toBeNull();
  });
});
