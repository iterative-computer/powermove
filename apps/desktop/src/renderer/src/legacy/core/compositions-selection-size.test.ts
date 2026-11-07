import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { selectionBounds, transformedBounds } from './selection-bounds';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'gl/compositor');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 8 }); PM.time = 2;
  return PM;
}
function pose(PM: any, layer: any, time: number, comp?: any) {
  if (comp) PM.scope.push(comp);
  try { PM.beginEval(time); return [...PM.worldMatrix(layer, time)]; }
  finally { if (comp) PM.scope.pop(); }
}
function close(actual: number[], expected: number[]) {
  actual.forEach((value, i) => expect(value).toBeCloseTo(expected[i]!, 7));
}

describe('size precomposition to selection', () => {
  it('unions rotated, scaled, reflected layers and rounds outward', () => {
    const PM = editor();
    const a = PM.mkLayer('shape', { d: { w: 100, h: 40, stroke: 4 },
      p: { 'position.x': 100.2, 'position.y': 80.4, rotation: 90, 'scale.x': 200, 'scale.y': 50 } });
    const b = PM.mkLayer('shape', { d: { w: 20, h: 20 },
      p: { 'position.x': 250.5, 'position.y': 90.2, 'scale.x': -100 } });
    PM.proj.layers = [a, b]; PM.ProjectIndex.invalidate(); PM.touch();
    const bounds = selectionBounds(PM, [a, b], PM.time);
    expect(bounds).toEqual({ x0: 89, y0: -24, x1: 261, y1: 185, w: 172, h: 209 });
    const before = [a, b].map(layer => pose(PM, layer, 2));
    const id = PM.Comps.precompose([a.id, b.id], { sizeToSelection: true });
    const comp = PM.proj.comps[id], outer = PM.proj.layers[0];
    expect(comp).toMatchObject({ w: bounds.w, h: bounds.h, dur: 8 });
    expect(outer.p['position.x'].v).toBe(bounds.x0 + bounds.w / 2);
    [a, b].forEach((layer, i) => close(PM.mul(pose(PM, outer, 2), pose(PM, layer, 2, comp)), before[i]!));
  });

  it.each([false, true])('keeps animated external parent/group rigs and layer-local key times (trim %s)', adjustDuration => {
    const PM = editor();
    const group = PM.mkLayer('group', { p: { 'position.x': 30, rotation: 17 } });
    const parent = PM.mkLayer('null', { p: { 'position.x': 70, 'scale.x': 130 } }); parent.group = group.id;
    parent.p.rotation.kf = [PM.KF(0, 10), PM.KF(8, 80)];
    const a = PM.mkLayer('shape', { from: 1, dur: 4, d: { w: 100, h: 40 }, p: { 'position.x': 40, 'anchor.x': -25 } });
    a.parent = parent.id; a.group = group.id;
    a.p['position.y'].kf = [PM.KF(0, 30), PM.KF(4, 90)];
    const b = PM.mkLayer('shape', { from: 2, dur: 4, d: { w: 70, h: 60 }, p: { 'position.x': 150 } });
    PM.proj.layers = [group, a, parent, b]; PM.ProjectIndex.invalidate(); PM.touch();
    const times = [2, 3.25, 4.8], before = times.map(t => [pose(PM, a, t), pose(PM, b, t)]);
    const parentBefore = times.map(t => pose(PM, parent, t));
    const keys = structuredClone(a.p['position.y'].kf);
    const bounds = selectionBounds(PM, [a, b], PM.time);
    const id = PM.Comps.precompose([a.id, b.id], { sizeToSelection: true, adjustDuration });
    const comp = PM.proj.comps[id], outer = PM.proj.layers.find((l: any) => l.type === 'precomp');
    expect(comp).toMatchObject({ w: bounds.w, h: bounds.h, dur: adjustDuration ? 5 : 8 });
    expect(outer.from).toBe(adjustDuration ? 1 : 0);
    expect(a.p['position.y'].kf).toEqual(keys);
    times.forEach((t, frame) => {
      [a, b].forEach((layer, i) => close(PM.mul(pose(PM, outer, t), pose(PM, layer, t - outer.from, comp)), before[frame]![i]!));
      close(pose(PM, parent, t), parentBefore[frame]!);
    });
    PM.hist.undo(); expect(PM.proj.comps[id]).toBeUndefined();
    PM.hist.redo(); PM.proj = PM.deserialize(PM.serialize()); PM.ProjectIndex.invalidate(); PM.touch();
    const reopened = PM.proj.comps[id], nested = reopened.layers.find((l: any) => l.id === a.id);
    const use = PM.proj.layers.find((l: any) => l.type === 'precomp');
    close(PM.mul(pose(PM, use, 3.25), pose(PM, nested, 3.25 - use.from, reopened)), before[1]![0]!);
  });

  it('uses text raster selection bounds, including its asymmetric origin', () => {
    const PM = editor();
    PM.raster = () => ({ selection: { x0: -18, y0: -42, x1: 112, y1: 14, w: 130, h: 56 }, anchorX: 18, anchorY: 42 });
    const text = PM.mkLayer('text', { p: { 'position.x': 200, 'position.y': 100, rotation: 90 } });
    PM.proj.layers = [text]; PM.ProjectIndex.invalidate(); PM.touch();
    expect(selectionBounds(PM, [text], 2)).toMatchObject({ w: 56, h: 130, x0: 186, y0: 82 });
  });

  it('trims selected groups to the contents, excluding the group timeline bar', () => {
    const PM = editor(), group = PM.mkLayer('group');
    const child = PM.mkLayer('shape', { from: 2, dur: 3 }); child.group = group.id;
    PM.proj.layers = [group, child]; PM.ProjectIndex.invalidate(); PM.touch();
    const id = PM.Comps.precompose([group.id], { sizeToSelection: true, adjustDuration: true });
    expect(PM.proj.comps[id].dur).toBe(3); expect(PM.proj.layers[0].from).toBe(2);
  });

  it('does not mutate a scene when visual bounds are unavailable', () => {
    const PM = editor(), audio = PM.mkLayer('audio'); PM.proj.layers = [audio];
    const before = PM.serialize();
    expect(() => PM.Comps.precompose([audio.id], { sizeToSelection: true })).toThrow('visual layer');
    expect(PM.serialize()).toBe(before);
  });

  it('transforms every corner, including skew and reflection', () => {
    expect(transformedBounds({ x0: -10, y0: -20, x1: 30, y1: 40 }, [-2, 0.5, 0.25, 1, 50, 80]))
      .toEqual({ x0: -15, y0: 55, x1: 80, y1: 135 });
  });
});


it('refuses a 3D parent projection before changing the selection', () => {
  const PM = editor(), parent = PM.mkLayer('null'), child = PM.mkLayer('shape');
  parent.threeD = true; child.parent = parent.id;
  PM.proj.layers = [child, parent]; PM.ProjectIndex.invalidate(); PM.touch();
  const before = PM.serialize();
  expect(() => PM.Comps.precompose([child.id], { sizeToSelection: true })).toThrow('2D layers');
  expect(PM.serialize()).toBe(before);
});
