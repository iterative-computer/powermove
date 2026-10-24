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

it('drops the preserved anchor when alignment changes so the pivot follows the alignment origin', () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 0;
  const layer = PM.mkLayer('text'); PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  PM.raster = () => ({ selection: { x0: 0, x1: 100, y0: 0, y1: 40 } });
  PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.size', value: 200, preserveHandEdits: false });
  expect(layer.d.fontAnchorBounds).toBeDefined();
  PM.Edit.apply({ type: 'set_content', target: layer.id, patch: { align: 'left' }, preserveHandEdits: false });
  expect(PM.L(layer.id).d.fontAnchorBounds).toBeUndefined();
});

it.each(['set_property', 'set_content'])('keeps reflowed text in place when %s repeats its current alignment', type => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 0;
  const layer = PM.mkLayer('text', { d: { size: 20 } });
  PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  layer.p['anchor.x'].v = 50; layer.p['anchor.y'].v = 20;
  PM.raster = (target: any) => {
    const size = PM.resolveContent(target, 0).size;
    const bounds = { x0: 0, x1: size * 5, y0: 0, y1: size * 2 };
    const offset = fontAnchorOffset(target.d.fontAnchorBounds, bounds, 50, 20);
    return { selection: { x0: bounds.x0 + offset.x, x1: bounds.x1 + offset.x,
      y0: bounds.y0 + offset.y, y1: bounds.y1 + offset.y } };
  };
  expect(PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.size', value: 40, preserveHandEdits: false }).ok).toBe(true);
  const before = PM.raster(layer).selection;
  expect([(before.x0 + before.x1) / 2, (before.y0 + before.y1) / 2]).toEqual([50, 20]);
  const command = type === 'set_content'
    ? { type, target: layer.id, patch: { align: 'center' }, preserveHandEdits: false }
    : { type, target: layer.id, path: 'c.align', value: 'center', preserveHandEdits: false };
  expect(PM.Edit.apply(command).ok).toBe(true);
  expect(PM.raster(PM.L(layer.id)).selection).toEqual(before);
  expect(PM.L(layer.id).d.fontAnchorBounds).toBeDefined();
});

it('captures typography reflow when a content patch repeats alignment alongside a size change', () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 0;
  const layer = PM.mkLayer('text'); PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  const bounds = { x0: -50, x1: 50, y0: -20, y1: 20 };
  PM.raster = () => ({ selection: bounds });
  const result = PM.Edit.apply({ type: 'set_content', target: layer.id, patch: { align: 'center', size: 200 }, preserveHandEdits: false });
  expect(result.ok, result.message).toBe(true);
  expect(layer.d.fontAnchorBounds).toEqual(bounds);
});

it('releases typography anchoring when a same-value alignment edit introduces animation', () => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 0;
  const layer = PM.mkLayer('text'); PM.proj.layers = [layer]; PM.ProjectIndex.invalidate();
  PM.raster = () => ({ selection: { x0: -50, x1: 50, y0: -20, y1: 20 } });
  PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.size', value: 200, preserveHandEdits: false });
  expect(layer.d.fontAnchorBounds).toBeDefined();
  const result = PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'c.align', value: 'center',
    mode: 'keyframe', time: 1, preserveHandEdits: false });
  expect(result.ok, result.message).toBe(true);
  expect(layer.d.fontAnchorBounds).toBeUndefined();
  expect(layer.d.align.kf).toHaveLength(1);
});
