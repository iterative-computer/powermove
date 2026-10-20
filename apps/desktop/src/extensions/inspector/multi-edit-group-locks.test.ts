import { describe, expect, it } from 'vitest';
import { makePM } from '../../renderer/src/legacy/__tests__/make-pm';
import { createInspectorEdit, inspectorMixed, inspectorPM, inspectorTargets } from './multi-edit';
import { toggleSelectionKey } from './selection-animation';

function fixture(lock: 'group' | 'nested group' | 'layer' = 'group') {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ dur: 5 }); PM.time = 1;
  const protectedLayer = PM.mkLayer('shape', { name: 'Locked group child' });
  const editableLayer = PM.mkLayer('shape', { name: 'Editable layer' });
  PM.proj.layers = [protectedLayer, editableLayer]; PM.ProjectIndex.invalidate();
  for (const layer of [protectedLayer, editableLayer]) {
    PM.setKey(layer, 'opacity', 0, 20);
    PM.setKey(layer, 'opacity', 1, 80);
  }
  let group = null;
  if (lock === 'layer') protectedLayer.lock = true;
  else {
    group = PM.groupLayers([protectedLayer.id]);
    if (lock === 'nested group') group = PM.groupLayers([group.id]);
    group.lock = true;
  }
  PM.selectLayers([editableLayer.id, protectedLayer.id]);
  PM.hist.clear();
  const legacy = inspectorPM(PM);
  const api = { ...legacy, edit: PM.Edit,
    anim: { ...legacy.anim, hasKeyAt: PM.hasKeyAt, removeKey: PM.removeKey, setKeyOn: PM.setKeyOn },
    history: { do: PM.hist.do }, project: { get: () => PM.proj } };
  return { PM, api, protectedLayer, editableLayer, group };
}

describe('inspector group locks', () => {
  it('removes playhead keys only from editable members of a mixed selection and undoes once', () => {
    const { PM, api, protectedLayer, editableLayer } = fixture();
    const protectedKeys = JSON.stringify(protectedLayer.p.opacity);
    toggleSelectionKey(api, editableLayer, ['opacity'], 1, 'Toggle opacity key');
    expect(JSON.stringify(protectedLayer.p.opacity)).toBe(protectedKeys);
    expect(editableLayer.p.opacity.kf).toHaveLength(1);
    expect(PM.hist.undo()).toBe(true);
    expect(PM.L(editableLayer.id).p.opacity.kf).toHaveLength(2);
    expect(JSON.stringify(PM.L(protectedLayer.id).p.opacity)).toBe(protectedKeys);
  });

  it('adds playhead keys only to editable members when their group-locked sibling is selected', () => {
    const { PM, api, protectedLayer, editableLayer } = fixture();
    const protectedKeys = JSON.stringify(protectedLayer.p.opacity);
    toggleSelectionKey(api, editableLayer, ['opacity'], 2, 'Add opacity key');
    expect(JSON.stringify(protectedLayer.p.opacity)).toBe(protectedKeys);
    expect(editableLayer.p.opacity.kf).toHaveLength(3);
  });

  it('edits an unlocked layer without making its group-locked sibling fail the atomic batch', () => {
    const { PM, api, protectedLayer, editableLayer } = fixture();
    const protectedKeys = JSON.stringify(protectedLayer.p.opacity);
    const result = createInspectorEdit(api).apply({ type: 'set_property', target: editableLayer.id,
      path: 'opacity', value: 50, time: 1, preserveHandEdits: false });
    expect(result.ok, result.message).toBe(true);
    expect(JSON.stringify(PM.L(protectedLayer.id).p.opacity)).toBe(protectedKeys);
    expect(PM.ev(PM.L(editableLayer.id), 'opacity', 1)).toBe(50);
  });

  it.each(['layer', 'nested group'] as const)('respects a protected sibling locked through its %s', lock => {
    const { PM, api, protectedLayer, editableLayer } = fixture(lock);
    const protectedKeys = JSON.stringify(protectedLayer.p.opacity);
    toggleSelectionKey(api, editableLayer, ['opacity'], 1, 'Remove opacity key');
    expect(editableLayer.p.opacity.kf).toHaveLength(1);
    expect(JSON.stringify(protectedLayer.p.opacity)).toBe(protectedKeys);
    const result = createInspectorEdit(api).apply({ type: 'set_property', target: editableLayer.id,
      path: 'opacity', value: 60, time: 0, preserveHandEdits: false });
    expect(result.ok, result.message).toBe(true);
    expect(PM.ev(PM.L(editableLayer.id), 'opacity', 0)).toBe(60);
    expect(JSON.stringify(PM.L(protectedLayer.id).p.opacity)).toBe(protectedKeys);
  });

  it('does not report protected sibling values as editable mixed values', () => {
    const { PM, api, protectedLayer, editableLayer } = fixture();
    PM.setKey(protectedLayer, 'opacity', 1, 40);
    expect(inspectorMixed(api, { mode: 'command', label: 'Opacity', command: value => ({
      type: 'set_property', target: editableLayer.id, path: 'opacity', value: Number(value),
    }) }, 80)).toBe(false);
  });

  it('safely excludes a protected target when there is no layer selection', () => {
    const { PM, api, protectedLayer } = fixture();
    PM.selectLayers([]);
    const before = JSON.stringify(PM.proj);
    expect(inspectorTargets(api, protectedLayer, 'opacity')).toEqual([]);
    expect(() => toggleSelectionKey(api, protectedLayer, ['opacity'], 1, 'Toggle opacity key')).not.toThrow();
    expect(JSON.stringify(PM.proj)).toBe(before);
  });
});
