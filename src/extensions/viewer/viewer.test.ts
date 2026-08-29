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
  it('snaps to the nearest candidate and breaks ties by the shorter guide', () => {
    const V = viewerRegistry().Viewer;
    const source = V.snapCandidatesFromPoints([{ x: 103, y: 10 }]);
    const targets = V.snapCandidatesFromPoints([{ x: 100, y: 500 }, { x: 100, y: 40 }, { x: 200, y: 10 }]);

    expect(V.findSnapTarget(source.x, targets.x, 6, 'x'))
      .toEqual({ offset: -3, distance: 3, sourcePoint: { x: 103, y: 10 }, targetPoint: { x: 100, y: 40 } });
    expect(V.findSnapTarget(source.x, targets.x, 2, 'x')).toBeNull();
  });

  it('snaps each axis independently, respects a locked axis, and draws guides to the target', () => {
    const V = viewerRegistry().Viewer;
    const moving = V.snapCandidatesFromPoints(V.boxSnapPoints({ x0: 193, x1: 213, y0: 42, y1: 62 }));
    const candidates = V.snapCandidatesFromPoints([{ x: 200, y: 0 }, { x: 400, y: 50 }]);

    const both = V.snapBox(moving, candidates, 6);
    expect(both.dx).toBe(-3);
    expect(both.dy).toBe(-2);
    expect(both.lines).toEqual([
      { from: { x: 200, y: 50 }, to: { x: 200, y: 0 } },
      { from: { x: 200, y: 50 }, to: { x: 400, y: 50 } },
    ]);

    const locked = V.snapBox(moving, candidates, 6, { x: true, y: false });
    expect(locked).toEqual({ dx: -3, dy: 0, lines: [{ from: { x: 200, y: 52 }, to: { x: 200, y: 0 } }] });
  });

  it('fires the haptic on guide entry and retarget, not while a guide stays put', () => {
    const V = viewerRegistry().Viewer;
    const a = [{ from: { x: 0, y: 0 }, to: { x: 100, y: 0 } }];
    const b = [{ from: { x: 0, y: 0 }, to: { x: 200, y: 0 } }];

    expect(V.snapLinesChanged(null, a)).toBe(true);
    expect(V.snapLinesChanged(a, a)).toBe(false);
    expect(V.snapLinesChanged(a, b)).toBe(true);
    expect(V.snapLinesChanged(a, [...a, ...b])).toBe(true);
    expect(V.snapLinesChanged(a, null)).toBe(false);
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
