import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { compositingLayers } from '../gl/compositor';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 30, dur: 3 });
  const first = PM.mkLayer('shape', { p: { 'position.x': 200, 'position.y': 180 } });
  const second = PM.cloneLayer(first);
  PM.proj.layers = [first, second]; PM.ProjectIndex.invalidate(); PM.touch();
  const group = PM.groupLayers([first.id, second.id]); PM.hist.clear();
  return { PM, group, first, second };
}

function outsideLayer(PM: any, parent: string, type = 'shape') {
  const layer = PM.mkLayer(type, { p: { 'position.x': 400, 'position.y': 220 } });
  PM.proj.layers.push(layer); PM.ProjectIndex.invalidate(); PM.touch();
  expect(PM.Edit.apply({ type: 'set_layer', target: layer.id, patch: { parent } }).ok).toBe(true);
  return layer;
}

function pose(PM: any, layer: any, time: number) { PM.beginEval(time); return [...PM.worldMatrix(layer, time)]; }
function poses(PM: any, layer: any, times: number[]) { return times.map(time => pose(PM, layer, time)); }
function unchangedPose(PM: any, id: string, times: number[], before: number[][]) {
  times.forEach((time, frame) => pose(PM, PM.L(id), time).forEach((value, index) => expect(value).toBeCloseTo(before[frame]![index]!, 6)));
}
function ungroup(PM: any, ids: string[]) {
  const result = PM.Edit.apply({ type: 'ungroup_layers', targets: ids }, { origin: 'agent', label: 'Ungroup layers' });
  expect(result.ok, result.message).toBe(true); return result;
}

const times = [.5, 1.517, 2.5];
describe('ungroup retains needed parent rigs', () => {
  it.each(['static', 'keyframes', 'time expression'].flatMap(animation => [false, true].map(locked => [animation, locked] as const)))(
    'preserves an outside child with %s motion (locked: %s)', (animation, locked) => {
      const { PM, group, first, second } = editor(), outside = outsideLayer(PM, group.id);
      if (animation === 'static') group.p['position.x'].v += 80;
      if (animation === 'keyframes') group.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 90)];
      if (animation === 'time expression') group.p.rotation.expr = 'time * 30';
      outside.lock = locked; PM.touch(); PM.hist.clear();
      if (animation === 'time expression') expect(PM.ev(group, 'rotation', 1.5)).toBe(45);
      const before = poses(PM, outside, times), source = JSON.stringify(outside), order = PM.proj.layers.map((layer: any) => layer.id);
      const result = ungroup(PM, [group.id]);
      unchangedPose(PM, outside.id, times, before);
      expect(JSON.stringify(outside)).toBe(source); expect(outside.parent).toBe(group.id);
      expect(PM.L(group.id).type).toBe('null'); expect(PM.TYPE_META.null.visual).toBe(false);
      expect(PM.L(group.id).solo).toBe(false); expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual(order);
      expect(result.data.results[0].data.ids).toEqual([first.id, second.id]); expect(PM.sel.layers).toEqual([first.id, second.id]);
      const visual = compositingLayers(PM.proj.layers, null, () => false).filter(layer => PM.TYPE_META[layer.type]?.visual !== false);
      expect(visual.map(layer => layer.id)).toEqual([first.id, second.id, outside.id]);
    });

  it('preserves an indirect locked descendant through its existing null parent', () => {
    const { PM, group } = editor(), middle = outsideLayer(PM, group.id, 'null'), outside = outsideLayer(PM, middle.id);
    group.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 90)]; middle.lock = outside.lock = true; PM.touch(); PM.hist.clear();
    const before = poses(PM, outside, times), source = JSON.stringify([middle, outside]);
    ungroup(PM, [group.id]); unchangedPose(PM, outside.id, times, before);
    expect(JSON.stringify([middle, outside])).toBe(source); expect(middle.parent).toBe(group.id); expect(outside.parent).toBe(middle.id);
  });

  it('preserves nested parent rigs when both containing groups are selected', () => {
    const { PM, group } = editor(), outer = PM.groupLayers([group.id]);
    const innerChild = outsideLayer(PM, group.id), outerChild = outsideLayer(PM, outer.id);
    outer.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 60)]; group.p['position.x'].kf = [PM.KF(0, 200), PM.KF(3, 280)];
    PM.touch(); PM.hist.clear();
    const frames = [.5, 1.5, 2.5], before = [innerChild, outerChild].map(layer => poses(PM, layer, frames));
    ungroup(PM, [outer.id, group.id]);
    [innerChild, outerChild].forEach((layer, index) => unchangedPose(PM, layer.id, frames, before[index]!));
    expect(PM.L(outer.id).type).toBe('null'); expect(PM.L(group.id).type).toBe('null');
    expect(PM.L(group.id).group ?? null).toBeNull();
  });

  it('keeps sibling group parenting valid when both groups are ungrouped', () => {
    const { PM, group, first } = editor(), siblingLayer = PM.mkLayer('shape', { p: { 'position.x': 500 } });
    PM.proj.layers.push(siblingLayer); PM.ProjectIndex.invalidate(); PM.touch();
    const sibling = PM.groupLayers([siblingLayer.id]);
    expect(PM.Edit.apply({ type: 'set_layer', target: sibling.id, patch: { parent: group.id } }).ok).toBe(true);
    const outside = outsideLayer(PM, sibling.id);
    group.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 60)]; sibling.p['position.x'].kf = [PM.KF(0, 500), PM.KF(3, 550)];
    PM.touch(); PM.hist.clear(); const before = poses(PM, outside, times);
    ungroup(PM, [group.id, sibling.id]); unchangedPose(PM, outside.id, times, before);
    expect(PM.L(group.id).type).toBe('null'); expect(PM.L(sibling.id).type).toBe('null');
    expect(PM.L(sibling.id).parent).toBe(group.id); expect(PM.L(first.id).group ?? null).toBeNull();
    expect(PM.wouldCycle(PM.L(group.id), sibling.id)).toBe(true);
  });

  it('preserves a retained parent through Undo, redo and serialized reopening', () => {
    const { PM, group } = editor(), outside = outsideLayer(PM, group.id);
    group.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 90)]; PM.touch(); PM.hist.clear();
    const before = poses(PM, outside, times); ungroup(PM, [group.id]);
    expect(PM.hist.list()).toEqual(['Ungroup layers']); expect(PM.hist.undo()).toBe(true);
    expect(PM.L(group.id).type).toBe('group'); unchangedPose(PM, outside.id, times, before);
    expect(PM.hist.redo()).toBe(true); expect(PM.L(group.id).type).toBe('null');
    PM.proj = PM.deserialize(PM.serialize()); PM.ProjectIndex.invalidate(); PM.touch();
    expect(PM.L(outside.id).parent).toBe(group.id); unchangedPose(PM, outside.id, times, before);
    expect(PM.hist.undo()).toBe(true); expect(PM.L(group.id).type).toBe('group');
    expect(PM.hist.undo()).toBe(false);
  });

  it('removes an unused group and keeps ordinary child motion and Undo', () => {
    const { PM, group, first, second } = editor(); group.p.rotation.v = 20; PM.touch();
    const before = [first, second].map(layer => poses(PM, layer, times)); ungroup(PM, [group.id]);
    expect(PM.L(group.id)).toBeNull();
    [first, second].forEach((layer, index) => unchangedPose(PM, layer.id, times, before[index]!));
    expect(PM.hist.undo()).toBe(true); expect(PM.L(group.id).type).toBe('group'); expect(PM.hist.undo()).toBe(false);
  });
});
