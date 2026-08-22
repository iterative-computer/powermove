const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
function utf8(rel) { return fs.readFileSync(path.join(root, rel), 'utf8'); }

/* ── registry harness: in-memory store ─────────────────── */
function projectsModel() {
  const mem = new Map();
  const PM = {
    uid: (() => { let n = 0; return p => `${p}${++n}`; })(),
    store: {
      get(k, d) { return mem.has(k) ? mem.get(k) : d; },
      set(k, v) { mem.set(k, v); },
      del(k) { mem.delete(k); },
    },
    serialize() { return JSON.stringify(PM.proj); },
    proj: null,
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(utf8('js/core/projects.js'), context);
  return { PM, mem };
}

test('registry round-trips project data and metadata', () => {
  const { PM } = projectsModel();
  const proj = { id: 'P1', name: 'Hero', layers: [{ id: 'La' }] };
  PM.proj = proj;
  PM.Projects.put(proj);
  assert.deepEqual([...PM.Projects.list().map(m => m.name)], ['Hero']);
  const back = PM.Projects.get('P1');
  assert.equal(back.name, 'Hero');
  assert.deepEqual([...back.layers.map(l => l.id)], ['La']);
});

test('upsertMeta changes metadata without overwriting stored data', () => {
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

test('remove clears both metadata and the storage slot', () => {
  const { PM, mem } = projectsModel();
  PM.proj = { id: 'P1', name: 'X', layers: [] };
  PM.Projects.put(PM.proj);
  PM.Projects.remove('P1');
  assert.equal(PM.Projects.list().length, 0);
  assert.equal(mem.has('project.P1'), false);
  assert.equal(PM.Projects.get('P1'), null);
});

test('open-tab bookkeeping is MRU-ordered and self-heals', () => {
  const { PM } = projectsModel();
  PM.proj = { id: 'A', name: 'A', layers: [] };
  PM.Projects.put(PM.proj);
  PM.proj = { id: 'B', name: 'B', layers: [] };
  PM.Projects.put(PM.proj);
  PM.Projects.markOpen('A');
  PM.Projects.markOpen('B');
  PM.Projects.markOpen('A');
  assert.deepEqual([...PM.Projects.tabs()], ['A', 'B'], 'most recent first, deduped');
  PM.Projects.markClosed('A');
  assert.deepEqual([...PM.Projects.tabs()], ['B']);
  /* a tab pointing at a deleted project disappears instead of crashing boot */
  PM.Projects.markOpen('GONE');
  assert.deepEqual([...PM.Projects.tabs()], ['B']);
});

test('get() falls back to the legacy autosave slot for a matching id', () => {
  const { PM } = projectsModel();
  PM.store.set('autosave', { v: 1, at: 0, proj: { id: 'Pold', name: 'Legacy', layers: [] } });
  const p = PM.Projects.get('Pold');
  assert.ok(p && p.name === 'Legacy');
  assert.equal(PM.Projects.get('other'), null);
});

/* ── layout & wiring pins ──────────────────────────────── */
test('normalization strips removed Layers panels from saved workspaces', () => {
  let n = 0;
  const PM = { uid: p => `${p}${++n}`, clamp: (v, a, b) => Math.max(a, Math.min(b, v)), h: () => ({}) };
  vm.runInContext(utf8('js/core/workspace.js'), vm.createContext({ window: { PM }, console }));
  const w = PM.WS.normalize({
    id: 'w1', name: 'Old',
    layout: { docks: [
      { id: 'left', panels: [{ id: 'layers' }, { id: 'assets' }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }] },
    ] },
  });
  const ids = w.layout.docks.flatMap(d => d.panels.map(q => q.id));
  assert.equal(ids.includes('layers'), false, 'Layers panel no longer docks');
  assert.ok(ids.includes('assets'));
});

test('tabs, projects screen, and drag preview are wired end to end', () => {
  const app = utf8('js/app.js');
  const index = utf8('index.html');
  const layout = utf8('js/ui/layout.js');
  const shortcuts = utf8('js/ui/shortcuts.js');
  assert.match(app, /function openTab\(id\)/, 'tab switching exists');
  assert.match(app, /function closeTab\(id\)/, 'tab closing exists');
  assert.match(app, /pm-open-project/, 'projects screen opens projects through the app bridge');
  assert.match(layout, /panel-drop-preview/, 'drag shows a live placement preview');
  assert.match(layout, /drag-src/, 'the dragged source is dimmed during preview');
  assert.match(shortcuts, /Projects screen/, '⌘P opens the projects screen');
  assert.ok(index.indexOf('js/core/projects.js') > -1 && index.indexOf('js/ui/projects.js') > -1,
    'registry and screen are loaded');
});
