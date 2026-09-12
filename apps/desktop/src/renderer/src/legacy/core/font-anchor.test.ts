import { describe, expect, it } from 'vitest';
import { fontAnchorOffset } from './font-anchor';

describe('typography anchored reflow', () => {
  const before = { x0: 0, x1: 100, y0: 0, y1: 40 };
  const after = { x0: 0, x1: 200, y0: 0, y1: 80 };
  it('keeps centered, corner, and external anchors fixed as text grows', () => {
    expect(fontAnchorOffset(before, after, 50, 20)).toEqual({ x: -50, y: -20 });
    expect(fontAnchorOffset(before, after, 0, 0)).toEqual({ x: 0, y: 0 });
    expect(fontAnchorOffset(before, after, 100, 40)).toEqual({ x: -100, y: -40 });
    expect(fontAnchorOffset(before, after, 150, 60)).toEqual({ x: -150, y: -60 });
  });
  it('handles alignment-origin changes and degenerate bounds', () => {
    expect(fontAnchorOffset(before, { ...after, x0: -100, x1: 100 }, 50, 20).x).toBe(50);
    expect(fontAnchorOffset({ ...before, x1: 0 }, after, 50, 20).x).toBe(0);
  });
});

import { makePM } from '../__tests__/make-pm';
it('captures typography bounds in the edit transaction and restores them with Undo', () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 0;
  const layer = PM.mkLayer('text'); PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  PM.raster = () => ({ selection: { x0: 0, x1: 100, y0: 0, y1: 40 } });
  expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.size', value: 200, preserveHandEdits: false }).ok).toBe(true);
  expect(layer.d.fontAnchorBounds).toEqual({ x0: 0, x1: 100, y0: 0, y1: 40 });
  PM.hist.undo();
  expect(PM.L(layer.id).d.fontAnchorBounds).toBeUndefined();
});
