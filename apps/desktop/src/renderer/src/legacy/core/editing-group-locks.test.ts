import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';

function fixture(lock: 'direct' | 'group' | 'nested group' = 'nested group') {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: 'Inherited edit locks', dur: 5 }); PM.time = 1;
  const editable = PM.mkLayer('shape', { name: 'Editable sibling' });
  const protectedLayer = PM.mkLayer('shape', { name: 'Protected child' });
  PM.proj.layers = [editable, protectedLayer]; PM.ProjectIndex.invalidate();
  for (const layer of [editable, protectedLayer]) {
    PM.setKeyOn(layer.p.opacity, 0, 20, 'linear', PM.proj.fps);
    PM.setKeyOn(layer.p.opacity, 2, 80, 'linear', PM.proj.fps);
  }
  const inner = PM.groupLayers([protectedLayer.id], 'Inner');
  const outer = lock === 'nested group' ? PM.groupLayers([inner.id], 'Locked outer') : inner;
  if (lock === 'direct') protectedLayer.lock = true;
  else outer.lock = true;
  PM.selectLayers([editable.id, protectedLayer.id]); PM.hist.clear();
  const ease = (ids: string[], overrideLock = false) => ({ type: 'set_easing', keyframes: ids, curve: [.2, .1, .8, .9], ...(overrideLock ? { overrideLock: true } : {}) });
  return { PM, editable, protectedLayer, outer, ease };
}

describe('inherited source edit locks', () => {
  it.each(['direct', 'group', 'nested group'] as const)('blocks deleting a sole protected target with a %s lock', lock => {
    const { PM, protectedLayer } = fixture(lock);
    const before = JSON.stringify(PM.proj);
    const result = PM.Edit.apply({ type: 'delete_layers', targets: [protectedLayer.id] }, { origin: 'agent' });
    expect(result.ok).toBe(false);
    expect(result.message).toMatch(/locked/i);
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(['group', 'nested group'] as const)('skips protected children of a %s during mixed deletion, with Undo/redo', lock => {
    const { PM, editable, protectedLayer, outer } = fixture(lock);
    const before = JSON.stringify(PM.proj.layers);
    const protectedBefore = JSON.stringify(protectedLayer);
    const result = PM.Edit.apply({ type: 'delete_layers', targets: [editable.id, protectedLayer.id] }, { origin: 'timeline' });
    expect(result.ok).toBe(true);
    expect(result.message).toMatch(/Skipped locked.*Protected child/);
    expect(PM.L(editable.id)).toBeNull();
    expect(JSON.stringify(PM.L(protectedLayer.id))).toBe(protectedBefore);
    expect(PM.L(outer.id).lock).toBe(true);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
    expect(PM.hist.redo()).toBe(true);
    expect(PM.L(editable.id)).toBeNull();
    expect(JSON.stringify(PM.L(protectedLayer.id))).toBe(protectedBefore);
  });

  it('makes no source or history changes when every deletion target is protected', () => {
    const { PM, editable, protectedLayer } = fixture();
    editable.lock = true;
    const before = JSON.stringify(PM.proj);
    const result = PM.Edit.apply({ type: 'delete_layers', targets: [editable.id, protectedLayer.id] }, { origin: 'agent' });
    expect(result.ok).toBe(false);
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it.each(['direct', 'group', 'nested group'] as const)('blocks easing changes through a %s lock', lock => {
    const { PM, protectedLayer, ease } = fixture(lock);
    const before = JSON.stringify(PM.proj);
    const result = PM.Edit.apply(ease(protectedLayer.p.opacity.kf.map((key: any) => key.i)), { origin: 'agent' });
    expect(result.ok).toBe(false);
    expect(result.message).toMatch(/Protected child.*locked/);
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('rejects mixed easing changes atomically when any key belongs to a locked group', () => {
    const { PM, editable, protectedLayer, ease } = fixture();
    const before = JSON.stringify(PM.proj);
    const keys = [...editable.p.opacity.kf, ...protectedLayer.p.opacity.kf].map(key => key.i);
    expect(PM.Edit.apply(ease(keys), { origin: 'timeline' }).ok).toBe(false);
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('checks inherited easing locks inside a composition that is not currently open', () => {
    const { PM, editable, protectedLayer, ease } = fixture();
    const nested = PM.mkProject({ name: 'Nested composition', dur: 5 });
    nested.layers = PM.proj.layers.filter((layer: any) => layer.id !== editable.id);
    PM.proj.layers = [editable]; PM.proj.comps = { [nested.id]: nested }; PM.ProjectIndex.invalidate();
    const before = JSON.stringify(PM.proj);
    const result = PM.Edit.apply(ease(protectedLayer.p.opacity.kf.map((key: any) => key.i)), { origin: 'agent' });
    expect(result.ok).toBe(false);
    expect(result.message).toMatch(/Protected child.*locked/);
    expect(JSON.stringify(PM.proj)).toBe(before);
    expect(PM.hist.undo()).toBe(false);
  });

  it('keeps editable easing changes available with Undo/redo', () => {
    const { PM, editable, protectedLayer, ease } = fixture();
    const before = JSON.stringify(editable.p.opacity);
    const protectedBefore = JSON.stringify(protectedLayer.p.opacity);
    expect(PM.Edit.apply(ease(editable.p.opacity.kf.map((key: any) => key.i)), { origin: 'timeline' }).ok).toBe(true);
    const after = JSON.stringify(PM.L(editable.id).p.opacity);
    expect(after).not.toBe(before);
    expect(JSON.stringify(PM.L(protectedLayer.id).p.opacity)).toBe(protectedBefore);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.L(editable.id).p.opacity)).toBe(before);
    expect(PM.hist.redo()).toBe(true);
    expect(JSON.stringify(PM.L(editable.id).p.opacity)).toBe(after);
  });

  for (const origin of ['agent', 'generated-ui', undefined]) {
    it.each(['delete_layers', 'set_easing'])(`does not grant ${origin ?? 'unset'} callers an explicit override for %s`, type => {
      const { PM, protectedLayer, ease } = fixture();
      const before = JSON.stringify(PM.proj);
      const command = type === 'delete_layers' ? { type, targets: [protectedLayer.id], overrideLock: true }
        : ease(protectedLayer.p.opacity.kf.map((key: any) => key.i), true);
      expect(PM.Edit.apply(command, { origin }).ok).toBe(false);
      expect(JSON.stringify(PM.proj)).toBe(before);
      expect(PM.hist.undo()).toBe(false);
    });
  }

  it.each(['delete_layers', 'set_easing'])('preserves trusted human overrides for %s', type => {
    const { PM, protectedLayer, ease } = fixture();
    const before = JSON.stringify(PM.proj.layers);
    const command = type === 'delete_layers' ? { type, targets: [protectedLayer.id], overrideLock: true }
      : ease(protectedLayer.p.opacity.kf.map((key: any) => key.i), true);
    expect(PM.Edit.apply(command, { origin: 'timeline' }).ok).toBe(true);
    expect(JSON.stringify(PM.proj.layers)).not.toBe(before);
    expect(PM.hist.undo()).toBe(true);
    expect(JSON.stringify(PM.proj.layers)).toBe(before);
  });
});
