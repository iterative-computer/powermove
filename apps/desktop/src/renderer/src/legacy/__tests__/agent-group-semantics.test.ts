import { describe, expect, it } from 'vitest';
import { makePM } from './make-pm';
import { editLayerStrips } from '../assistant/layer-strips';

function editor() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 4 });
  PM.time = 2;
  const outside = PM.mkLayer('shape', { name: 'Outside' });
  const a = PM.mkLayer('shape', { name: 'A' });
  const b = PM.mkLayer('shape', { name: 'B' });
  PM.proj.layers = [outside, a, b];
  PM.ProjectIndex.invalidate();
  return { PM, outside, a, b };
}

describe('agent grouping coordinate and ordering contracts', () => {
  it('rebases local scale to preserve the playhead pose when joining an animated group', () => {
    const { PM, outside, a } = editor();
    const group = PM.groupLayers([a.id], 'Camera rig');
    expect(PM.Edit.apply(['scale.x', 'scale.y'].flatMap(path => [
      { type: 'set_property', target: group.id, path, value: 100, time: 0, mode: 'keyframe' },
      { type: 'set_property', target: group.id, path, value: 200, time: 2, mode: 'keyframe' },
    ])).ok).toBe(true);
    const pose = PM.worldMatrix(outside, PM.time);
    const result = PM.Edit.apply({ type: 'move_to_group', targets: [outside.id], group: group.id });
    expect(result.ok, result.message).toBe(true);
    PM.worldMatrix(outside, PM.time).forEach((value: number, i: number) => expect(value).toBeCloseTo(pose[i]));
    expect(PM.ev(outside, 'scale.x', PM.time)).toBeCloseTo(50);
    expect(PM.ev(outside, 'scale.y', PM.time)).toBeCloseTo(50);
    // Pose preservation samples this time; it does not bake the group's whole animation.
    expect(PM.worldMatrix(outside, 0)[0]).toBeCloseTo(pose[0] / 2);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(outside.id).group ?? null).toBeNull();
    expect(PM.ev(PM.L(outside.id), 'scale.x', 2)).toBeCloseTo(100);
  });

  it('uses composition-wide indices for reorder_layer, including destination membership', () => {
    const { PM, outside, a, b } = editor();
    const group = PM.groupLayers([a.id, b.id]);
    expect(PM.proj.layers[0].id).toBe(outside.id);
    const result = PM.Edit.apply({ type: 'reorder_layer', target: b.id, index: 0 });
    expect(result.ok, result.message).toBe(true);
    expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual([b.id, outside.id, group.id, a.id]);
    expect(b.group).toBeNull();
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(b.id).group).toBe(group.id);
  });

  it('reorders group siblings through strip anchors without changing membership', () => {
    const { PM, outside, a, b } = editor();
    const group = PM.groupLayers([a.id, b.id]);
    const result = editLayerStrips(PM, {
      operation: 'reorder', layerIds: [b.id], anchorId: a.id, placement: 'before',
    }, { label: 'Reorder siblings', origin: 'agent' });
    expect(result.ok, result.message).toBe(true);
    expect(PM.proj.layers.map((layer: any) => layer.id)).toEqual([outside.id, group.id, b.id, a.id]);
    expect(b.group).toBe(group.id);
    const before = PM.serialize();
    const rejected = editLayerStrips(PM, {
      operation: 'reorder', layerIds: [b.id], anchorId: outside.id, placement: 'before',
    }, { label: 'Invalid reorder', origin: 'agent' });
    expect(rejected.ok).toBe(false);
    expect(rejected.message).toContain('Reorder siblings');
    expect(PM.serialize()).toBe(before);
  });
});
