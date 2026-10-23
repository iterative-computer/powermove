import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { compositingLayers, groupContainsSolo } from '../gl/compositor';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 640, h: 360, dur: 3 }); PM.time = .5;
  const inside = [PM.mkLayer('shape'), PM.mkLayer('text')], outside = PM.mkLayer('shape');
  PM.proj.layers = [...inside, outside]; PM.ProjectIndex.invalidate(); PM.touch();
  const group = PM.groupLayers(inside.map(layer => layer.id)); PM.hist.clear();
  return { PM, group, inside, outside };
}
// PM.active handles visibility/time; Solo is a separate global compositor filter.
function visible(PM: any) {
  const source = PM.proj.layers, solo = source.some((layer: any) => layer.solo);
  return compositingLayers(source, null, () => false).filter(layer =>
    (!solo || layer.solo || PM.groupAncestors(layer).some((parent: any) => parent.solo)
      || groupContainsSolo(layer, source, child => PM.groupAncestors(child)))
    && PM.TYPE_META[layer.type]?.visual !== false && PM.active(layer, .5)).map(layer => layer.id);
}
function state(PM: any) { return JSON.stringify({ source: PM.serialize(), selection: PM.sel, history: PM.hist.list() }); }
function ungroup(PM: any, ids: string[]) { return PM.Edit.apply({ type: 'ungroup_layers', targets: ids }, { origin: 'agent', label: 'Ungroup layers' }); }
function rejected(PM: any, ids: string[]) {
  const before = state(PM), rendered = visible(PM), result = ungroup(PM, ids);
  expect(result.ok).toBe(false); expect(result.message).toContain('Turn off Solo before ungrouping this group');
  expect(state(PM)).toBe(before); expect(visible(PM)).toEqual(rendered); expect(PM.hist.undo()).toBe(false);
}

describe('ungroup preserves the global Solo filter', () => {
  it('does not reveal a live outside layer previously excluded by group Solo', () => {
    const { PM, group, inside, outside } = editor(); group.solo = true; PM.touch();
    expect(PM.active(outside, .5)).toBe(true); expect(visible(PM)).toEqual(inside.map(layer => layer.id));
    const before = visible(PM), result = ungroup(PM, [group.id]);
    if (result.ok) expect(visible(PM)).toEqual(before);
    else rejected(PM, [group.id]);
  });

  it.each([100, 50].flatMap(opacity => [false, true].map(hidden => [opacity, hidden] as const)))(
    'rejects removing a %s% solo group without changing its filter (hidden: %s)', (opacity, hidden) => {
      const { PM, group, outside } = editor(); group.solo = true; group.on = !hidden; group.p.opacity.v = opacity; PM.touch();
      expect(PM.active(outside, .5)).toBe(true); rejected(PM, [group.id]);
    });

  it.each(['ordinary group', 'child Solo', 'ancestor Solo', 'nested child group Solo', 'outside Solo'])(
    'allows an ungroup operation that preserves %s', mode => {
      const { PM, group, inside, outside } = editor();
      if (mode === 'child Solo') inside[0]!.solo = true;
      if (mode === 'ancestor Solo') { const outer = PM.groupLayers([group.id]); outer.solo = true; }
      if (mode === 'nested child group Solo') {
        const nested = PM.groupLayers(inside.map(layer => layer.id)); nested.solo = true;
      }
      if (mode === 'outside Solo') outside.solo = true;
      PM.selectLayers(group.id); PM.touch(); PM.hist.clear(); const before = visible(PM);
      if (mode === 'child Solo') expect(before).toEqual([inside[0]!.id]);
      if (mode === 'outside Solo') expect(before).toEqual([outside.id]);
      const result = ungroup(PM, [group.id]); expect(result.ok, result.message).toBe(true);
      expect(visible(PM)).toEqual(before); expect(PM.hist.list()).toEqual(['Ungroup layers']);
      expect(PM.hist.undo()).toBe(true); expect(visible(PM)).toEqual(before);
      expect(PM.hist.redo()).toBe(true); expect(visible(PM)).toEqual(before);
    });

  it.each(['agent command', 'direct core call'])('validates a later selected Solo group before touching an ordinary group (%s)', mode => {
    const { PM, group: first } = editor(), extra = PM.mkLayer('shape');
    PM.proj.layers.push(extra); PM.ProjectIndex.invalidate(); PM.touch();
    const second = PM.groupLayers([extra.id]); second.solo = true;
    PM.selectLayers([first.id, second.id]); PM.touch(); PM.hist.clear(); const before = state(PM);
    if (mode === 'agent command') rejected(PM, [first.id, second.id]);
    else expect(() => PM.ungroupLayers([first.id, second.id])).toThrow('Turn off Solo before ungrouping this group');
    expect(state(PM)).toBe(before); expect(PM.hist.undo()).toBe(false);
  });

  it('validates a selected nested Solo group before removing its normal ancestor', () => {
    const { PM, group } = editor(), outer = PM.groupLayers([group.id]); group.solo = true; PM.touch(); PM.hist.clear();
    rejected(PM, [outer.id, group.id]);
  });

  it('rejects an empty Solo boundary before revealing the outside layer', () => {
    const { PM, group, outside } = editor(); PM.proj.layers = [group, outside]; group.solo = true;
    PM.ProjectIndex.invalidate(); PM.touch(); expect(visible(PM)).toEqual([]); rejected(PM, [group.id]);
  });
});
