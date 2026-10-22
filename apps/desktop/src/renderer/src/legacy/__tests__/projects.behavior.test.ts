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


it('the journal keeps one copy of what each edit rewrites whole and replays to the same document', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'J1', name: 'Journal', revision: 0, edits: [], layers: [{ id: 'L1', p: { o: { v: 100 } } }, { id: 'L2', p: { o: { v: 100 } } }] };
  PM.Projects.put(PM.proj);
  const log = [];
  for (let revision = 1; revision <= 5; revision++) {
    log.push({ id: `e${revision}`, revision });
    PM.bus.emit('history:project-patch', {
      projectId: 'J1', revision,
      patches: [
        { path: ['revision'], exists: true, value: revision },
        { path: ['edits'], exists: true, value: structuredClone(log) },
        { path: ['layers', revision % 2, 'p', 'o', 'v'], exists: true, value: 10 * revision },
      ],
    });
  }
  const recovered = PM.Projects.get('J1');
  assert.equal(recovered.revision, 5);
  assert.deepEqual(recovered.edits.map(e => e.id), ['e1', 'e2', 'e3', 'e4', 'e5']);
  assert.deepEqual(recovered.layers.map(l => l.p.o.v), [40, 50]);
  const entries = mem.get('projectJournal.J1');
  assert.equal(entries.length, 5, 'every edit keeps its entry');
  assert.deepEqual(entries.map(e => e.patches.filter(p => p.path[0] === 'edits').length), [0, 0, 0, 0, 1], 'one provenance log');
  assert.deepEqual(entries.map(e => e.patches.length), [1, 1, 1, 1, 3]);
  // A checkpoint drops the journal; the next edit starts a new one.
  PM.Projects.put(PM.proj);
  assert.equal(mem.has('projectJournal.J1'), false);
  PM.bus.emit('history:project-patch', { projectId: 'J1', revision: 6, patches: [{ path: ['revision'], exists: true, value: 6 }] });
  // Each entry names the checkpoint it follows (Projects.get replays only those, and untagged ones).
  const cp = mem.get('project.J1').cp;
  assert.equal(typeof cp, 'string');
  assert.deepEqual(mem.get('projectJournal.J1'), [{ revision: 6, patches: [{ path: ['revision'], exists: true, value: 6 }], cp }]);
});

it('replays the journal entries of the slot\'s checkpoint and untagged ones, not those of an older checkpoint', () => {
  const { PM, mem } = projectsModel();
  const journal = [
    { revision: 2, cp: 'older', patches: [{ path: ['name'], exists: true, value: 'Stale' }, { path: ['revision'], exists: true, value: 2 }] },
    { revision: 3, patches: [{ path: ['layers', 0, 'name'], exists: true, value: 'Untagged' }] },
    { revision: 4, cp: 'newer', patches: [{ path: ['revision'], exists: true, value: 4 }] },
  ];
  mem.set('project.C1', { v: '1.0.0', cp: 'newer', proj: { id: 'C1', name: 'Slot', revision: 3, layers: [{ id: 'L1', name: 'Layer' }] } });
  mem.set('projectJournal.C1', structuredClone(journal));
  assert.deepEqual(PM.Projects.get('C1'), { id: 'C1', name: 'Slot', revision: 4, layers: [{ id: 'L1', name: 'Untagged' }] });
  // A slot from a build without checkpoint ids replays everything, as before.
  mem.set('project.C1', { v: '1.0.0', proj: { id: 'C1', name: 'Slot', revision: 1, layers: [{ id: 'L1', name: 'Layer' }] } });
  assert.deepEqual(PM.Projects.get('C1'), { id: 'C1', name: 'Stale', revision: 4, layers: [{ id: 'L1', name: 'Untagged' }] });
});

it('hands the journal to an asynchronous store as a snapshot of each version', () => {
  const { PM, mem } = projectsModel();
  const writes = [];
  PM.store.setAsync = (key, value) => { writes.push([key, value]); mem.set(key, value); return Promise.resolve(); };
  PM.proj = { id: 'A1', name: 'Async', revision: 0, layers: [{ id: 'L1', p: { o: { v: 1 } } }] };
  PM.Projects.put(PM.proj);
  for (let revision = 1; revision <= 3; revision++) {
    PM.bus.emit('history:project-patch', { projectId: 'A1', revision, patches: [{ path: ['layers', 0, 'p', 'o', 'v'], exists: true, value: revision }] });
  }
  assert.deepEqual(writes.map(([key, value]) => [key, value.length]), [['projectJournal.A1', 1], ['projectJournal.A1', 2], ['projectJournal.A1', 3]]);
  assert.equal(PM.Projects.get('A1').layers[0].p.o.v, 3);
});

it('checkpoints the project when the journal would pass its bound, to the byte', () => {
  const MAX = 4 * 1024 * 1024;
  const bytes = (value) => new TextEncoder().encode(JSON.stringify(value)).byteLength;
  for (const over of [0, 1]) {
    const { PM, mem } = projectsModel();
    PM.proj = { id: 'B1', name: 'Big', revision: 0, layers: [] };
    PM.Projects.put(PM.proj);
    const first = { revision: 1, patches: [{ path: ['layers', 0], exists: true, value: { id: 'L1' } }] };
    PM.bus.emit('history:project-patch', { projectId: 'B1', ...first });
    const second = (note) => ({ revision: 2, patches: [{ path: ['layers', 1], exists: true, value: { id: 'L2', note } }] });
    // The journal's JSON (entries tagged with their checkpoint) at exactly the bound, or one byte over it.
    const cp = mem.get('project.B1').cp;
    const n = MAX - bytes([{ ...first, cp }, { ...second(''), cp }]) + over;
    PM.bus.emit('history:project-patch', { projectId: 'B1', ...second('é'.repeat(n / 2) + 'x'.repeat(n % 2)) });
    if (over) assert.equal(mem.has('projectJournal.B1'), false, 'one byte over checkpoints the project');
    else assert.equal(bytes(mem.get('projectJournal.B1')), MAX, 'the bound itself fits');
  }
});

it('stores a project written for a newer reader once and never autosaves it afterwards', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'N1', name: 'Newer', layers: [{ id: 'L1' }], minReaderVersion: 2 };
  PM.Projects.put(PM.proj);
  assert.deepEqual(PM.Projects.get('N1').layers, [{ id: 'L1' }], 'the opened document is stored once');
  // This test store keeps references; the real one stores copies.
  PM.proj = { ...PM.proj, layers: [...PM.proj.layers, { id: 'L2' }] };
  PM.Projects.put(PM.proj);
  PM.Projects.recover({ ...PM.proj, revision: 50 });
  PM.bus.emit('history:project-patch', { projectId: 'N1', revision: 1, patches: [{ path: ['layers', 1], exists: true, value: { id: 'L2' } }] });
  assert.equal(mem.has('projectJournal.N1'), false, 'no recovery journal');
  assert.deepEqual(PM.Projects.get('N1').layers, [{ id: 'L1' }], 'edits never reach storage');
  assert.equal(PM.Projects.rename('N1', 'Renamed'), 'Renamed');
  assert.equal(PM.Projects.list().find(m => m.id === 'N1').name, 'Renamed', 'library metadata still updates');
  assert.deepEqual(PM.Projects.get('N1').layers, [{ id: 'L1' }]);
});

it('keeps autosaving projects stamped for this reader', () => {
  const { PM } = projectsModel();
  PM.proj = { id: 'C1', name: 'Current', layers: [], minReaderVersion: 1 };
  PM.Projects.put(PM.proj);
  PM.proj = { ...PM.proj, layers: [{ id: 'L1' }] };
  PM.Projects.put(PM.proj);
  assert.deepEqual(PM.Projects.get('C1').layers, [{ id: 'L1' }]);
  PM.bus.emit('history:project-patch', { projectId: 'C1', revision: 1, patches: [{ path: ['name'], exists: true, value: 'Journaled' }] });
  assert.equal(PM.Projects.get('C1').name, 'Journaled');
});
