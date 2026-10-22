import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 640, h: 360, fps: 10, dur: 3 }); PM.time = .5;
  const children = [PM.mkLayer('shape'), PM.mkLayer('text')];
  PM.proj.layers = [...children]; PM.ProjectIndex.invalidate(); PM.touch();
  const group = PM.groupLayers(children.map(layer => layer.id)); PM.hist.clear();
  return { PM, group, children };
}
const times = [.5, 1.5, 2.5];
function visibility(PM: any, children: any[]) { return children.map(child => times.map(time => PM.active(PM.L(child.id), time))); }
function state(PM: any) { return JSON.stringify({ source: PM.serialize(), selection: PM.sel, history: PM.hist.list() }); }
function ungroup(PM: any, ids: string[]) { return PM.Edit.apply({ type: 'ungroup_layers', targets: ids }, { origin: 'agent', label: 'Ungroup layers' }); }
function hidden(PM: any, group: any, keys = false) {
  if (keys) group.on = PM.P(true, { kf: [PM.KF(0, false, 'hold'), PM.KF(3, false, 'hold')] });
  else expect(PM.Edit.apply({ type: 'set_layer', target: group.id, patch: { visible: false } }, { origin: 'inspector' }).ok).toBe(true);
  PM.touch(); PM.hist.clear();
}
function rejected(PM: any, ids: string[]) {
  const before = state(PM), result = ungroup(PM, ids);
  expect(result.ok).toBe(false); expect(result.message).toContain('Pre-compose this group to keep its animated visibility');
  expect(state(PM)).toBe(before); expect(PM.hist.undo()).toBe(false);
}

describe('ungroup keeps inherited group visibility', () => {
  it.each([100, 50].flatMap(opacity => [false, true].map(keys => [opacity, keys] as const)))(
    'keeps a hidden group at %s% hidden and restores child tracks with Undo (constant keys: %s)', (opacity, keys) => {
      const { PM, group, children } = editor(); group.p.opacity.v = opacity;
      children[0]!.on = PM.P(true, { kf: [PM.KF(0, true, 'hold'), PM.KF(1, false, 'hold')] });
      children[1]!.on = PM.P(true, { expr: 'time < 2' });
      hidden(PM, group, keys);
      const original = children.map(child => JSON.stringify(child.on)), before = visibility(PM, children);
      expect(before).toEqual([[false, false, false], [false, false, false]]);
      const result = ungroup(PM, [group.id]); expect(result.ok, result.message).toBe(true);
      expect(PM.L(group.id)).toBeNull(); children.forEach(child => expect(PM.L(child.id).on).toBe(false));
      expect(visibility(PM, children)).toEqual(before); expect(PM.hist.list()).toEqual(['Ungroup layers']);
      expect(PM.hist.undo()).toBe(true); expect(PM.L(group.id).type).toBe('group');
      children.forEach((child, index) => expect(JSON.stringify(PM.L(child.id).on)).toBe(original[index]));
      expect(visibility(PM, children)).toEqual(before); expect(PM.hist.redo()).toBe(true);
      expect(visibility(PM, children)).toEqual(before); expect(PM.hist.undo()).toBe(true);
      expect(PM.hist.undo()).toBe(false);
    });

  it.each([false, true])('keeps ordinary true visibility and child animation editable (constant keys: %s)', keys => {
    const { PM, group, children } = editor();
    if (keys) group.on = PM.P(false, { kf: [PM.KF(0, true, 'hold'), PM.KF(3, true, 'hold')] });
    children[0]!.on = PM.P(true, { kf: [PM.KF(0, false, 'hold'), PM.KF(1, true, 'hold')] });
    children[1]!.on = PM.P(true, { expr: 'time < 2' }); PM.touch();
    const original = children.map(child => JSON.stringify(child.on)), before = visibility(PM, children);
    const result = ungroup(PM, [group.id]); expect(result.ok, result.message).toBe(true);
    children.forEach((child, index) => expect(JSON.stringify(PM.L(child.id).on)).toBe(original[index]));
    expect(visibility(PM, children)).toEqual(before); expect(PM.hist.undo()).toBe(true); expect(visibility(PM, children)).toEqual(before);
  });

  it.each([100, 50].flatMap(opacity => ['hold keys', 'expression'].map(mode => [opacity, mode] as const)))(
    'refuses dynamic visibility on a %s% group (%s) without revealing it', (opacity, mode) => {
      const { PM, group, children } = editor(); group.p.opacity.v = opacity;
      group.on = mode === 'hold keys' ? PM.P(false, { kf: [PM.KF(0, false, 'hold'), PM.KF(1, true, 'hold')] }) : PM.P(true, { expr: 'time >= 1' });
      PM.touch(); const before = visibility(PM, children); expect(before[0]).toEqual([false, true, true]);
      rejected(PM, [group.id]); expect(visibility(PM, children)).toEqual(before);
    });

  it.each([false, true])('keeps nested hidden boundaries through Undo (both selected: %s)', both => {
    const { PM, group, children } = editor(), outer = PM.groupLayers([group.id]);
    group.p.opacity.v = 50; outer.p.opacity.v = 80; hidden(PM, group); hidden(PM, outer, true);
    const before = visibility(PM, children);
    const result = ungroup(PM, both ? [outer.id, group.id] : [outer.id]); expect(result.ok, result.message).toBe(true);
    expect(visibility(PM, children)).toEqual(before);
    if (both) { expect(PM.L(group.id)).toBeNull(); children.forEach(child => expect(PM.L(child.id).on).toBe(false)); }
    else { expect(PM.L(group.id).on).toBe(false); expect(PM.L(group.id).group).toBeNull(); }
    expect(PM.hist.undo()).toBe(true); expect(PM.L(outer.id).type).toBe('group'); expect(PM.L(group.id).on).toBe(false);
    expect(visibility(PM, children)).toEqual(before); expect(PM.hist.redo()).toBe(true); expect(visibility(PM, children)).toEqual(before);
  });

  it.each(['agent command', 'direct core call'])('checks all selected groups before changing the first hidden group (%s)', mode => {
    const { PM, group: first, children } = editor(); hidden(PM, first);
    const extra = PM.mkLayer('shape'); PM.proj.layers.push(extra); PM.ProjectIndex.invalidate(); PM.touch();
    const second = PM.groupLayers([extra.id]); second.on = PM.P(true, { expr: 'time > 1' });
    PM.selectLayers([first.id, second.id]); PM.touch(); PM.hist.clear(); const before = state(PM);
    if (mode === 'agent command') rejected(PM, [first.id, second.id]);
    else expect(() => PM.ungroupLayers([first.id, second.id])).toThrow('Pre-compose this group to keep its animated visibility');
    expect(state(PM)).toBe(before); children.forEach(child => expect(PM.L(child.id).on).toBe(true)); expect(PM.hist.undo()).toBe(false);
  });

  it('checks a nested dynamic group before changing its selected hidden ancestor', () => {
    const { PM, group } = editor(), outer = PM.groupLayers([group.id]);
    group.on = PM.P(true, { expr: 'time > 1' }); hidden(PM, outer);
    rejected(PM, [outer.id, group.id]); expect(PM.L(outer.id).on).toBe(false); expect(PM.L(group.id).on.expr).toBe('time > 1');
  });

  it('preserves an outside locked parent dependent while keeping members hidden', () => {
    const { PM, group, children } = editor(), outside = PM.mkLayer('shape');
    PM.proj.layers.push(outside); PM.ProjectIndex.invalidate(); PM.touch();
    expect(PM.Edit.apply({ type: 'set_layer', target: outside.id, patch: { parent: group.id } }, { origin: 'agent' }).ok).toBe(true);
    group.p.rotation.kf = [PM.KF(0, 0), PM.KF(3, 60)]; outside.lock = true; hidden(PM, group);
    const poses = times.map(time => [...PM.worldMatrix(outside, time)]), source = JSON.stringify(outside);
    const result = ungroup(PM, [group.id]); expect(result.ok, result.message).toBe(true); expect(PM.L(group.id).type).toBe('null');
    expect(JSON.stringify(outside)).toBe(source); expect(times.map(time => PM.active(outside, time))).toEqual([true, true, true]);
    times.forEach((time, frame) => PM.worldMatrix(outside, time).forEach((value: number, index: number) => expect(value).toBeCloseTo(poses[frame]![index]!)));
    expect(visibility(PM, children)).toEqual([[false, false, false], [false, false, false]]);
    expect(PM.hist.undo()).toBe(true); expect(PM.L(group.id).type).toBe('group');
  });

  it('keeps the existing style and non100% solo restrictions on hidden groups', () => {
    const { PM, group } = editor(); hidden(PM, group); group.p.opacity.v = 50; group.solo = true; PM.touch();
    let before = state(PM), result = ungroup(PM, [group.id]); expect(result.ok).toBe(false); expect(state(PM)).toBe(before);
    const current = PM.L(group.id); current.solo = false; current.fx = [{ id: 'blur', type: 'blur', on: true, p: {} }]; PM.touch();
    before = state(PM); result = ungroup(PM, [group.id]); expect(result.ok).toBe(false); expect(result.message).toContain('Remove visual compositing');
    expect(state(PM)).toBe(before); expect(PM.hist.undo()).toBe(false);
  });
});
