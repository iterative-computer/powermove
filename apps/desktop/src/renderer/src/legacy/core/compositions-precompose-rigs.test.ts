import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { compositingLayers } from '../gl/compositor';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 2 });
  return PM;
}

function pose(PM: any, layer: any, time: number, comp?: any): number[] {
  if (comp) PM.scope.push(comp);
  try { PM.beginEval(time); return [...PM.worldMatrix(layer, time)]; }
  finally { if (comp) PM.scope.pop(); }
}

describe('precompose boundary rig preservation', () => {
  it.each([false, true])('preserves animated external parents when adjustDuration is %s', adjustDuration => {
    const PM = editor();
    const parent = PM.mkLayer('null', { p: { 'position.x': 100, rotation: 20 } });
    parent.p.rotation.kf = [PM.KF(0, 0), PM.KF(2, 60)];
    const child = PM.mkLayer('shape', { from: 0.3, dur: 1.7, p: { 'position.x': 100, 'position.y': 100 } });
    child.parent = parent.id;
    PM.proj.layers = [child, parent];
    PM.ProjectIndex.invalidate(); PM.touch();
    const times = [0.4, 1, 1.8], before = times.map(time => pose(PM, child, time));
    const id = PM.Comps.precompose([child.id], { name: 'Nested rig', adjustDuration });
    const comp = PM.proj.comps[id], precomp = PM.proj.layers.find((layer: any) => layer.type === 'precomp');
    const nested = comp.layers.find((layer: any) => layer.id === child.id);
    times.forEach((time, index) => {
      const after = PM.mul(pose(PM, precomp, time), pose(PM, nested, time - precomp.from, comp));
      after.forEach((value: number, channel: number) => expect(value).toBeCloseTo(before[index]![channel]!, 6));
    });
  });

  it('preserves the animated pose of unselected children when their parent moves inside', () => {
    const PM = editor();
    const parent = PM.mkLayer('null', { p: { 'position.x': 100 } });
    parent.p.rotation.kf = [PM.KF(0, 0), PM.KF(2, 60)];
    const child = PM.mkLayer('shape', { p: { 'position.x': 100, 'position.y': 100 } });
    child.parent = parent.id;
    PM.proj.layers = [child, parent];
    PM.ProjectIndex.invalidate(); PM.touch();
    const times = [0.4, 1, 1.8], before = times.map(time => pose(PM, child, time));
    PM.Comps.precompose([parent.id], { name: 'Moved rig' });
    times.forEach((time, index) => {
      pose(PM, child, time).forEach((value, channel) => expect(value).toBeCloseTo(before[index]![channel]!, 6));
    });
  });

  it('copies shared group spaces once and preserves group opacity without doubling transforms', () => {
    const PM = editor();
    const group = PM.mkLayer('group', { p: { 'position.x': 40, opacity: 65 } });
    group.p.rotation.kf = [PM.KF(0, 0), PM.KF(2, 50)];
    const parent = PM.mkLayer('null', { p: { 'position.x': 80 } });
    parent.group = group.id;
    const first = PM.mkLayer('shape', { p: { 'position.x': 90, 'position.y': 100 } });
    const second = PM.mkLayer('shape', { p: { 'position.x': 140, 'position.y': 100 } });
    for (const child of [first, second]) { child.group = group.id; child.parent = parent.id; }
    PM.proj.layers = [group, first, second, parent];
    PM.ProjectIndex.invalidate(); PM.touch();
    const times = [0.4, 1, 1.8];
    const before = times.map(time => [pose(PM, first, time), pose(PM, second, time)]);
    const id = PM.Comps.precompose([first.id, second.id]);
    const comp = PM.proj.comps[id];
    expect(comp.layers.filter((layer: any) => layer.type === 'group')).toHaveLength(1);
    expect(first.parent).toBe(second.parent);
    expect(first.group).toBe(second.group);
    times.forEach((time, frame) => {
      [first, second].forEach((child, index) => {
        pose(PM, child, time, comp).forEach((value, channel) => expect(value).toBeCloseTo(before[frame]![index]![channel]!, 6));
        PM.scope.push(comp);
        try { expect(PM.worldOpacity(child, time)).toBeCloseTo(0.65, 6); }
        finally { PM.scope.pop(); }
      });
    });
  });

  it('preserves rig copies and links through undo, redo and serialized reopening', () => {
    const PM = editor();
    const parent = PM.mkLayer('null', { p: { 'position.x': 80 } });
    parent.p.rotation.kf = [PM.KF(0, 0), PM.KF(2, 60)];
    const child = PM.mkLayer('shape', { from: 0.3, dur: 1.7, p: { 'position.x': 100 } });
    child.parent = parent.id;
    PM.proj.layers = [child, parent];
    PM.ProjectIndex.invalidate(); PM.touch();
    const before = pose(PM, child, 1);
    const id = PM.Comps.precompose([child.id], { adjustDuration: true });
    expect(PM.proj.comps[id].layers[1].p.rotation.kf.map((key: any) => key.i)).not.toEqual(parent.p.rotation.kf.map((key: any) => key.i));
    PM.hist.undo();
    expect(PM.proj.layers.find((layer: any) => layer.id === child.id).parent).toBe(parent.id);
    expect(PM.proj.comps[id]).toBeUndefined();
    PM.hist.redo();
    PM.proj = PM.deserialize(PM.serialize());
    PM.ProjectIndex.invalidate(); PM.touch();
    const comp = PM.proj.comps[id], precomp = PM.proj.layers.find((layer: any) => layer.type === 'precomp');
    const nested = comp.layers.find((layer: any) => layer.id === child.id);
    const after = PM.mul(pose(PM, precomp, 1), pose(PM, nested, 1 - precomp.from, comp));
    after.forEach((value: number, channel: number) => expect(value).toBeCloseTo(before[channel]!, 6));
  });

  it('keeps grouped and ungrouped artwork in the same paint order', () => {
    const PM = editor();
    const group = PM.mkLayer('group');
    const grouped = PM.mkLayer('shape'), ungrouped = PM.mkLayer('shape');
    grouped.group = group.id;
    PM.proj.layers = [group, grouped, ungrouped];
    PM.ProjectIndex.invalidate(); PM.touch();
    const paintOrder = (layers: any[]) => compositingLayers(layers, null, () => false)
      .filter(layer => layer.type === 'shape').map(layer => layer.id);
    const before = paintOrder(PM.proj.layers);
    const id = PM.Comps.precompose([grouped.id, ungrouped.id]);
    expect(paintOrder(PM.proj.comps[id].layers)).toEqual(before);
  });
});
