// @ts-nocheck -- faithful behavioral transplant of unique project-registry oracle cases.
import assert from 'node:assert/strict';
import { it } from 'vitest';

import { makePM } from './make-pm';

function projectsModel() {
  const PM = makePM('core/projects');
  const mem = new Map();
  PM.store = {
    get(key, fallback) { return mem.has(key) ? mem.get(key) : fallback; },
    set(key, value) { mem.set(key, value); },
    del(key) { mem.delete(key); },
  };
  PM.serialize = () => JSON.stringify(PM.proj);
  PM.proj = null;
  return { PM, mem };
}

it('tab-owned editor state round-trips separately for each project', () => {
  const { PM } = projectsModel();
  PM.Projects.putState('P1', { time: 1, workspace: { id: 'edit-one' }, detached: ['assets'] });
  PM.Projects.putState('P2', { time: 7, workspace: { id: 'edit-two' }, detached: ['inspector'] });
  assert.deepEqual(PM.Projects.getState('P1'), { time: 1, workspace: { id: 'edit-one' }, detached: ['assets'] });
  assert.deepEqual(PM.Projects.getState('P2'), { time: 7, workspace: { id: 'edit-two' }, detached: ['inspector'] });
});

it('registry unwraps real save envelopes and boot picks the first project with content', () => {
  const { PM } = projectsModel();
  PM.store.set('project.empty', JSON.stringify({ v: '1.0.0', proj: { id: 'empty', name: 'Untitled', layers: [] } }));
  PM.store.set('project.hero', JSON.stringify({ v: '1.0.0', proj: { id: 'hero', name: 'Hero', layers: [{ id: 'L1' }] } }));
  PM.Projects.saveList([{ id: 'empty', name: 'Untitled', at: 2 }, { id: 'hero', name: 'Hero', at: 1 }]);
  PM.store.set('openTabs', ['empty']);
  assert.equal(PM.Projects.get('hero').layers.length, 1);
  assert.equal(PM.Projects.pickBoot().id, 'hero');
});

it('upsertMeta changes metadata without overwriting stored data', () => {
  const { PM } = projectsModel();
  PM.proj = { id: 'P1', name: 'Hero', layers: [] };
  PM.Projects.put(PM.proj);
  /* a rename must never write the CURRENT composition into another slot */
  PM.proj = { id: 'P9', name: 'Other', layers: [{ id: 'Lz' }] };
  PM.Projects.upsertMeta({ id: 'P1', name: 'Hero renamed', at: 42 });
  assert.equal(PM.Projects.get('P1').name, 'Hero', 'stored data untouched');
  const meta = PM.Projects.list().find(m => m.id === 'P1');
  assert.equal(meta.name, 'Hero renamed');
});

it('rename updates the project slot, registry metadata, and active project together', () => {
  const { PM } = projectsModel();
  PM.proj = { id: 'P1', name: 'Hero', layers: [{ id: 'L1' }] };
  PM.Projects.put(PM.proj);
  assert.equal(PM.Projects.rename('P1', '  Velocity Study  '), 'Velocity Study');
  assert.equal(PM.proj.name, 'Velocity Study');
  assert.equal(PM.Projects.get('P1').name, 'Velocity Study');
  assert.equal(PM.Projects.list().find(m => m.id === 'P1').name, 'Velocity Study');
  assert.equal(PM.Projects.rename('P1', '   '), 'Velocity Study', 'a blank rename keeps the current name');
});

it('remove clears both metadata and the storage slot', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'P1', name: 'X', layers: [] };
  PM.Projects.put(PM.proj);
  PM.Projects.remove('P1');
  assert.equal(PM.Projects.list().length, 0);
  assert.equal(mem.has('project.P1'), false);
  assert.equal(PM.Projects.get('P1'), null);
});

it('trash is recoverable until explicitly destroyed', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'P1', name: 'Recover me', layers: [{ id: 'L1' }] };
  PM.Projects.put(PM.proj);
  assert.equal(PM.Projects.trash('P1'), true);
  assert.equal(PM.Projects.list().length, 0);
  assert.equal(PM.Projects.trashList()[0].name, 'Recover me');
  assert.equal(mem.has('project.P1'), true, 'trash keeps project data');
  assert.equal(PM.Projects.restore('P1'), true);
  assert.equal(PM.Projects.get('P1').layers.length, 1);
  PM.Projects.trash('P1'); PM.Projects.destroy('P1');
  assert.equal(mem.has('project.P1'), false, 'delete forever removes data');
});

it('get() falls back to the legacy autosave slot for a matching id', () => {
  const { PM } = projectsModel();
  PM.store.set('autosave', { v: 1, at: 0, proj: { id: 'Pold', name: 'Legacy', layers: [] } });
  const p = PM.Projects.get('Pold');
  assert.ok(p && p.name === 'Legacy');
  assert.equal(PM.Projects.get('other'), null);
});

it('replays compact recovery patches over the last full checkpoint', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'P1', name: 'Before', revision: 0, layers: [{ id: 'L1', name: 'Layer' }] };
  PM.Projects.put(PM.proj);
  PM.bus.emit('history:project-patch', {
    projectId: 'P1', revision: 1,
    patches: [
      { path: ['name'], exists: true, value: 'After' },
      { path: ['revision'], exists: true, value: 1 },
      { path: ['layers', 0, 'name'], exists: true, value: 'Renamed layer' },
    ],
  });

  assert.equal(PM.Projects.get('P1').name, 'After');
  assert.equal(PM.Projects.get('P1').layers[0].name, 'Renamed layer');
  assert.equal(mem.has('projectJournal.P1'), true);
});

