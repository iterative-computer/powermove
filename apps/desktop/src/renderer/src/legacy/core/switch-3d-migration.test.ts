import { describe, expect, it } from 'vitest';
import { makePM } from '../__tests__/make-pm';
import { is3DLayer } from './space-3d';
import { migrateInherited3D } from './switch-3d-migration';

function setup() {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ w: 640, h: 360, dur: 2, fps: 10 }); PM.time = 0;
  const a = PM.mkLayer('shape', { name: 'A' }), b = PM.mkLayer('text', { name: 'B' });
  PM.proj.layers = [a, b]; PM.ProjectIndex.invalidate();
  const g = PM.groupLayers([a.id, b.id]); PM.touch();
  return { PM, a, b, g };
}

describe('3D switch ownership', () => {
  it("a group's switch sets its members, and a member can then be turned off on its own", () => {
    const { PM, a, b, g } = setup();
    expect(PM.Edit.apply({ type: 'set_layer', target: g.id, patch: { threeD: true } }).ok).toBe(true);
    expect([g.threeD, a.threeD, b.threeD]).toEqual([true, true, true]);
    expect(PM.Edit.apply({ type: 'set_layer', target: a.id, patch: { threeD: false } }).ok).toBe(true);
    expect(is3DLayer(PM, a)).toBe(false); expect(is3DLayer(PM, b)).toBe(true);
    expect(PM.Edit.apply({ type: 'set_layer', target: g.id, patch: { threeD: false } }).ok).toBe(true);
    expect([g.threeD, a.threeD, b.threeD]).toEqual([false, false, false]);
    PM.hist.undo();
    expect([PM.L(g.id).threeD, PM.L(a.id).threeD, PM.L(b.id).threeD]).toEqual([true, false, true]);
  });

  it('old projects keep members of 3D groups in 3D; new ones are left as saved', () => {
    const old = { layers: [{ id: 'g', type: 'group', threeD: true }, { id: 'a', type: 'shape', group: 'g' }, { id: 'n', type: 'group', group: 'g' },
      { id: 'b', type: 'text', group: 'n' }, { id: 's', type: 'audio', group: 'g' }, { id: 'flat', type: 'shape' }],
      comps: { c: { layers: [{ id: 'cg', type: 'group', threeD: true }, { id: 'cm', type: 'image', group: 'cg' }] } } };
    const migrated = migrateInherited3D(old);
    const on = (layers: any[]) => Object.fromEntries(layers.map(l => [l.id, !!l.threeD]));
    expect(on(migrated.layers)).toEqual({ g: true, a: true, n: true, b: true, s: false, flat: false });
    expect(on(migrated.comps.c.layers)).toEqual({ cg: true, cm: true });
    expect(migrated.switch3DVersion).toBe(2);
    const current = { switch3DVersion: 2, layers: [{ id: 'g', type: 'group', threeD: true }, { id: 'a', type: 'shape', group: 'g' }] };
    expect(migrateInherited3D(current).layers[1].threeD).toBeUndefined();
  });
});
