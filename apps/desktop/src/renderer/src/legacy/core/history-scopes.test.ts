import { expect, it, vi } from 'vitest';
import { install } from './history';
import { makePM } from '../__tests__/make-pm';

function editor() {
  const PM: any = {
    proj: { layers: [{ id: 'a', name: 'Original' }], revision: 0 }, sel: {},
    uid: () => Math.random().toString(),
    replaceProject: (project: any) => { PM.proj = project; },
    bus: { emit: vi.fn() }, touch: vi.fn(), invalidate: vi.fn(), autosave: vi.fn(),
  };
  install(PM);
  return PM;
}

it.each(['undo', 'rollback'])('restores earlier child edits on %s after the transaction widens to a parent', (restore) => {
  const PM = editor();
  const before = structuredClone(PM.proj);
  PM.hist.beginScoped('Rename and add');
  PM.hist.track([['layers', 0]]);
  PM.proj.layers[0].name = 'Renamed';
  PM.hist.track([['layers']]);
  PM.proj.layers.push({ id: 'b', name: 'New layer' });
  const after = structuredClone(PM.proj);

  if (restore === 'undo') PM.hist.commit();
  PM.hist[restore]();
  expect(PM.proj).toEqual(before);
  if (restore === 'undo') {
    PM.hist.redo();
    expect(PM.proj).toEqual(after);
  }
});

it('preserves absent fields and multiple child snapshots when widening to the root', () => {
  const PM = editor();
  const before = structuredClone(PM.proj);
  PM.hist.beginScoped('Widen edit');
  PM.hist.track([['layers', 0, 'name'], ['layers', 0, 'color'], ['revision']]);
  PM.proj.layers[0].name = 'Renamed';
  PM.proj.layers[0].color = 'red';
  PM.proj.revision = 1;
  PM.hist.track([['layers', 0]]);
  PM.proj.layers[0].name = 'Renamed again';
  PM.hist.track([[]]);
  PM.hist.rollback();
  expect(PM.proj).toEqual(before);
});

it.each(['commit', 'cancel'])('restores a live source edit that renames then deletes a layer after %s', (finish) => {
  const PM = makePM('core/easing', 'core/model', 'core/selection', 'core/anim', 'core/history', 'core/editing');
  PM.proj = PM.mkProject({ name: 'Scoped history', w: 640, h: 360, fps: 30, dur: 5 });
  const layer = PM.mkLayer('text', { name: 'Original' });
  PM.addLayer(layer, 0);
  const before = JSON.parse(JSON.stringify(PM.proj));
  PM.Edit.begin('Rename and delete', { origin: 'interface' });
  expect(PM.Edit.dispatch({ type: 'set_layer', target: layer.id, patch: { name: 'Renamed' } }).ok).toBe(true);
  expect(PM.Edit.dispatch({ type: 'delete_layers', targets: [layer.id] }).ok).toBe(true);
  PM.Edit[finish]();
  if (finish === 'commit') PM.hist.undo();
  expect(PM.proj).toEqual(before);
});
