import { afterEach, describe, expect, it } from 'vitest';

import {
  selectionOutlineColor,
  anchorMoveValues,
  calculateResize, composeLocalLinear, compositionFramePosition, compositionIsOutOfView, editableTextAtPoint,
  install, layerContainsPoint, layerWorldPivot,
  localRotationForWorldDirection, multiplyLinear, resolveSelectionGeometry, resizeCursorForHandle,
  previewRenderSize, previewRenderViewport, resizeLocksAspect, rotateLinear, selectionTransformRoots, solveLocalTransformForWorldLinear,
  selectionBoundsCenter, shapeBoxFromDrag, transformPointAround, viewerWheelMode, zoomPanForPoint,
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
  it('detects a fully lost composition without firing while any pixels remain visible', () => {
    const stage = { left: 0, top: 0, right: 1000, bottom: 700, width: 1000, height: 700 };
    expect(compositionIsOutOfView(stage, {
      left: 999, top: 100, right: 1199, bottom: 300, width: 200, height: 200,
    })).toBe(false);
    expect(compositionIsOutOfView(stage, {
      left: 1000, top: 100, right: 1200, bottom: 300, width: 200, height: 200,
    })).toBe(true);
  });

  it('positions small and oversized compositions from the same center-plus-pan model', () => {
    expect(compositionFramePosition(
      { width: 1000, height: 700 }, { width: 1000, height: 500 }, .5, { x: 20, y: -10 },
    )).toEqual({ x: 270, y: 215 });
    expect(compositionFramePosition(
      { width: 1000, height: 700 }, { width: 1000, height: 500 }, 2, { x: 20, y: -10 },
    )).toEqual({ x: -480, y: -160 });
  });

  it('builds AE-style shape boxes from a corner, center, and constrained square', () => {
    expect(shapeBoxFromDrag({ x: 10, y: 20 }, { x: 110, y: 70 }))
      .toEqual({ x0: 10, y0: 20, x1: 110, y1: 70, w: 100, h: 50 });
    expect(shapeBoxFromDrag({ x: 60, y: 45 }, { x: 110, y: 70 }, { fromCenter: true }))
      .toEqual({ x0: 10, y0: 20, x1: 110, y1: 70, w: 100, h: 50 });
    expect(shapeBoxFromDrag({ x: 10, y: 20 }, { x: 110, y: 70 }, { constrain: true }))
      .toEqual({ x0: 10, y0: 20, x1: 110, y1: 120, w: 100, h: 100 });
  });

  it('keeps the point under the Zoom tool fixed on screen', () => {
    expect(zoomPanForPoint(
      { width: 1000, height: 700 }, { x: 750, y: 200 }, { x: 700, y: 300 },
      { width: 1000, height: 500 }, 2,
    )).toEqual({ x: -150, y: -250 });
  });

  it('zooms a mouse wheel and trackpad pinch while leaving two-finger scroll as pan', () => {
    expect(viewerWheelMode({ deltaX: 0, deltaY: 3, deltaMode: 1 })).toBe('zoom');
    expect(viewerWheelMode({ deltaX: 0, deltaY: 4, wheelDeltaY: -120 })).toBe('zoom');
    expect(viewerWheelMode({ deltaX: 0, deltaY: -12, ctrlKey: true })).toBe('zoom');
    expect(viewerWheelMode({ deltaX: 1.5, deltaY: 8.25 })).toBe('pan');
    expect(viewerWheelMode({ deltaX: 0, deltaY: 8.25 })).toBe('pan');
    expect(viewerWheelMode({ deltaX: 0, deltaY: 120 })).toBe('pan');
  });

  it('moves Pan Behind in layer space and compensates Position unless Option is held', () => {
    const compensated = anchorMoveValues(
      [0, 2, -3, 0], { x: 10, y: 20 }, { x: 100, y: 200 }, { x: 130, y: 220 },
    );
    expect(compensated).not.toBeNull();
    expect(compensated!.anchor.x).toBeCloseTo(20);
    expect(compensated!.anchor.y).toBeCloseTo(10);
    expect(compensated!.position).toEqual({ x: 130, y: 220 });

    expect(anchorMoveValues(
      [0, 2, -3, 0], { x: 10, y: 20 }, { x: 100, y: 200 }, { x: 130, y: 220 },
      { moveLayer: false },
    )).toEqual({ anchor: { x: 20, y: 10 }, position: { x: 100, y: 200 } });
  });

  it('renders fitted high-resolution compositions near their displayed pixel size', () => {
    expect(previewRenderSize(3840, 2160, 0.25, 2, 1)).toEqual({ width: 1920, height: 1080, scale: 0.5 });
    expect(previewRenderSize(7680, 4320, 0.1, 2, 0.5)).toEqual({ width: 768, height: 432, scale: 0.1 });
    expect(previewRenderSize(1920, 1080, 2, 2, 1)).toEqual({ width: 1920, height: 1080, scale: 1 });
  });
  it('renders only the visible high-zoom composition region at screen density', () => {
    const viewport = previewRenderViewport(1920, 1080, 8, 1200, 800, -7080, -3920, 2, 1);
    expect(viewport).toMatchObject({
      x: 869, y: 474, width: 182, height: 132,
      cssLeft: 6952, cssTop: 3792, cssWidth: 1456, cssHeight: 1056,
      renderWidth: 2912, renderHeight: 2112,
    });
    expect(viewport!.renderWidth).toBeLessThan(1920 * 8);
    expect(previewRenderViewport(1920, 1080, 1, 1200, 800, 0, 0, 2, 1)).toBeNull();
  });
  it('retains the presented viewport through pans covered by its overscan', () => {
    const first = previewRenderViewport(1920, 1080, 8, 1200, 800, -7080, -3920, 2, 1)!;
    expect(previewRenderViewport(1920, 1080, 8, 1200, 800, -7048, -3944, 2, 1, 128, first)).toBe(first);
    expect(previewRenderViewport(1920, 1080, 8, 1200, 800, -6900, -3920, 2, 1, 128, first)).not.toBe(first);
    expect(previewRenderViewport(1920, 1080, 4, 1200, 800, -7080, -3920, 2, 1, 128, first)).not.toBe(first);
    expect(previewRenderViewport(1920, 1080, 8, 1200, 800, -7080, -3920, 1, 1, 128, first)).not.toBe(first);
    expect(previewRenderViewport(1920, 1080, 8, 1200, 800, -7080, -3920, 2, .5, 128, first)).not.toBe(first);
    expect(previewRenderViewport(1920, 1080, 8, 1300, 800, -7080, -3920, 2, 1, 128, first)).not.toBe(first);
    expect(previewRenderViewport(3840, 2160, 8, 1200, 800, -7080, -3920, 2, 1, 128, first)).not.toBe(first);
  });
  it('keeps a fixed bounded allocation through all four composition edges', () => {
    for (const zoom of [1.1, 2.51184505912942, 8]) for (const dpr of [1, 2]) for (const quality of [.5, 1]) {
      const width = 1920 * zoom, height = 1080 * zoom;
      for (const edge of ['left', 'right', 'top', 'bottom']) {
        let previous: ReturnType<typeof previewRenderViewport> = null;
        for (let offset = -150; offset <= 350; offset += 2.7) {
          let x = (600-width)/2, y = (400-height)/2;
          if (edge === 'left') x = offset;
          if (edge === 'right') x = 600-width-offset;
          if (edge === 'top') y = offset;
          if (edge === 'bottom') y = 400-height-offset;
          const view: NonNullable<ReturnType<typeof previewRenderViewport>> = previewRenderViewport(1920,1080,zoom,600,400,x,y,dpr,quality,128,previous)!;
          expect(view.renderWidth).toBe(Math.round(856*dpr*quality));
          expect(view.renderHeight).toBe(Math.round(656*dpr*quality));
          expect(view.cssLeft).toBeGreaterThanOrEqual(0);
          expect(view.cssTop).toBeGreaterThanOrEqual(0);
          expect(view.cssLeft+view.cssWidth).toBeLessThanOrEqual(width+1e-8);
          expect(view.cssTop+view.cssHeight).toBeLessThanOrEqual(height+1e-8);
          expect(view.cssLeft).toBeLessThanOrEqual(Math.max(0,-x)+1e-8);
          expect(view.cssTop).toBeLessThanOrEqual(Math.max(0,-y)+1e-8);
          expect(view.cssLeft+view.cssWidth).toBeGreaterThanOrEqual(Math.min(width,600-x)-1e-8);
          expect(view.cssTop+view.cssHeight).toBeGreaterThanOrEqual(Math.min(height,400-y)-1e-8);
          previous=view;
        }
      }
    }
  });
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
      expect.objectContaining({ axis: 'x', from: { x: 200, y: 50 }, to: { x: 200, y: 0 } }),
      expect.objectContaining({ axis: 'y', from: { x: 200, y: 50 }, to: { x: 400, y: 50 } }),
    ]);

    const locked = V.snapBox(moving, candidates, 6, { x: true, y: false });
    expect(locked.dx).toBe(-3);
    expect(locked.dy).toBe(0);
    expect(locked.lines).toEqual([
      expect.objectContaining({ axis: 'x', from: { x: 200, y: 52 }, to: { x: 200, y: 0 } }),
    ]);
  });

  it('preserves enough snap-point context to render an unmistakable composition center', () => {
    const V = viewerRegistry().Viewer;
    const moving = V.snapCandidatesFromPoints(V.boxSnapPoints({ x0: 270, x1: 370, y0: 140, y1: 220 }));
    const composition = V.snapCandidatesFromPoints(V.boxSnapPoints(
      { x0: 0, x1: 640, y0: 0, y1: 360 }, 'composition',
    ));

    const snap = V.snapBox(moving, composition, 1);
    expect(snap.lines).toEqual([
      expect.objectContaining({ axis: 'x', sourceRole: 'center', targetRole: 'center', targetScope: 'composition' }),
      expect.objectContaining({ axis: 'y', sourceRole: 'center', targetRole: 'center', targetScope: 'composition' }),
    ]);
  });

  it('keeps sibling alignment targets for a multi-selection inside one parent', () => {
    const PM = viewerRegistry();
    const parent = { id: 'parent', parent: null };
    const left = { id: 'left', parent: parent.id };
    const middle = { id: 'middle', parent: parent.id };
    const target = { id: 'target', parent: parent.id };
    const layers = [parent, left, middle, target];
    PM.proj = { w: 640, h: 360, layers };
    PM.active = () => true;
    PM.L = (id: string) => layers.find((layer) => layer.id === id);
    PM.GL.bounds = () => ({ x0: -10, x1: 10, y0: -10, y1: 10 });
    PM.worldMatrix = (layer: any) => [1, 0, 0, 1, layer === target ? 290 : 50, 100];

    const candidates = PM.Viewer.snapshotSnapCandidates(0, [left, middle]);
    expect(candidates.x.map((candidate: any) => candidate.value)).toEqual(expect.arrayContaining([280, 290, 300]));
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
    expect(V.snapLinesChanged(
      [{ ...a[0], axis: 'x', targetRole: 'corner', targetScope: 'layer' }],
      [{ ...a[0], axis: 'x', targetRole: 'center', targetScope: 'composition' }],
    )).toBe(true);
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

  it('edits selected text beneath a full-frame top layer before using pixel pick', () => {
    const selected = { id: 'text', type: 'text' };
    const cover = { id: 'cover', type: 'shape' };
    const PM = {
      selLayers: () => [selected], active: () => true,
      GL: {
        pick: () => cover,
        bounds: () => ({ x0: -100, y0: -25, x1: 100, y1: 25 }),
      },
      worldMatrix: () => [1, 0, 0, 1, 500, 350],
    };

    expect(editableTextAtPoint(PM, 500, 350, 0)).toBe(selected);
    expect(editableTextAtPoint(PM, 800, 350, 0)).toBeNull();
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

  it('keeps a group bounds center explicit when its transform anchor is offset', () => {
    const group = { id: 'group', type: 'group', parent: null };
    const PM = {
      active: () => true, TYPE_META: { group: {} }, L: () => null,
      GL: { bounds: () => ({ x0: -70, y0: -40, x1: 70, y1: 40, w: 140, h: 80 }) },
      worldMatrix: () => [1, 0, 0, 1, 320, 180],
      ev: (_layer: any, path: string) => path === 'anchor.x' ? 10 : path === 'anchor.y' ? -20 : 0,
    };

    const selection = resolveSelectionGeometry(PM, [group], 0)!;
    expect(selectionBoundsCenter(selection)).toEqual({ x: 320, y: 180 });
    expect(selection.pivotWorld).toEqual({ x: 330, y: 160 });
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

describe('canvas selection contrast', () => {
  it('inverts both light and dark composition backgrounds', () => {
    expect(selectionOutlineColor({bg:'#FFFFFF'})).toBe('#000000');
    expect(selectionOutlineColor({bg:'#000000'})).toBe('#ffffff');
    expect(selectionOutlineColor({bg:'#000000',backgroundFill:{type:'solid',stops:[{color:'#123456'}]}})).toBe('#edcba9');
  });
});
