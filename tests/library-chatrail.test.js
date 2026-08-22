const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

/* ── library harness: easing + model + library with UI/runtime stubs ── */
function libraryModel() {
  let nextId = 0;
  const historyLabels = [];
  const PM = {
    clamp: (v, a, b) => Math.max(a, Math.min(b, v)),
    uid: (p) => `${p}${++nextId}`,
    snapF: (t) => Math.round(t * 30) / 30,
    time: 0,
    round: (v) => v,
    bus: { emit() {}, on() {} },
    invalidate: () => {},
    toast: () => {},
    hist: { do: (label, fn) => { historyLabels.push(label); return fn(); } },
    selectLayers: () => {},
    addLayer: null, // provided by model.js
    firstSel: () => null,
    Export: { snapshot: () => 'data:image/jpeg;base64,thumb' },
    GL: { dropProgram() {} },
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/easing.js'), 'utf8'), context);
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/model.js'), 'utf8'), context);
  vm.runInContext(fs.readFileSync(path.join(root, 'js/gl/shaders.js'), 'utf8'), context);
  /* mirror the inspector's real uniform sync so shader layers get their defaults */
  PM.syncShaderUniforms = (L) => {
    const defs = PM.parseUniforms(L.d.code);
    L._udefs = defs;
    const u = L.d.uniforms;
    defs.forEach(d => { if (!u[d.name]) u[d.name] = PM.P(d.def); });
    for (const k in u) if (!defs.some(d => d.name === k)) delete u[k];
  };
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/library.js'), 'utf8'), context);
  PM.__historyLabels = historyLabels;
  return PM;
}
function utf8(rel) { return fs.readFileSync(path.join(root, rel), 'utf8'); }

function baseProject(PM) {
  const p = PM.mkProject({ name: 'T', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.proj = p;
  return p;
}

test('saveSection snapshots selected layers into the project library', () => {
  const PM = libraryModel();
  const p = baseProject(PM);
  const a = PM.mkLayer('shape', { name: 'A' }, p);
  const b = PM.mkLayer('text', { name: 'B' }, p);
  p.layers.push(a, b);
  const entry = PM.Library.saveSection('Hero pair', [a.id]);
  assert.ok(entry, 'entry created');
  assert.equal(entry.name, 'Hero pair');
  assert.equal(entry.thumb.startsWith('data:image'), true, 'thumbnail captured');
  assert.deepEqual([...entry.layers.map(l => l.name)], ['A'], 'only requested layers stored');
  assert.equal(entry.versions.length, 1, 'first snapshot is version one');
  assert.ok(p.library.sections.includes(entry), 'stored inside proj (persists with saves)');
});

test('saveVersion promotes the current selection without mutating older versions', () => {
  const PM = libraryModel();
  const p = baseProject(PM);
  const a = PM.mkLayer('shape', { name: 'A' }, p);
  const b = PM.mkLayer('text', { name: 'B' }, p);
  p.layers.push(a, b);
  const entry = PM.Library.saveSection('Hero', [a.id]);
  PM.Library.saveVersion(entry.id, [b.id]);
  assert.equal(entry.versions.length, 2);
  assert.deepEqual([...entry.versions[0].layers.map(l => l.name)], ['A']);
  assert.deepEqual([...entry.layers.map(l => l.name)], ['B'], 'latest version becomes insert source');
});

test('insertSection re-inserts with fresh identities, shifted to the playhead', () => {
  const PM = libraryModel();
  const p = baseProject(PM);
  const inner = PM.mkLayer('shape', { name: 'Inner' }, p); inner.from = 2;
  const outer = PM.mkLayer('null', { name: 'Outer' }, p); outer.from = 3;
  inner.parent = outer.id;
  p.layers.push(inner, outer);
  const entry = PM.Library.saveSection('Pair', [inner.id, outer.id]);
  p.layers.length = 0; // clear the comp; the section survives in the library

  PM.time = 5;
  const clones = PM.Library.insertSection(entry.id);
  assert.ok(clones && clones.length === 2);
  assert.notEqual(clones[0].id, inner.id, 'fresh layer ids');
  assert.equal(clones[0].from, 5, 'earliest layer lands on the playhead');
  assert.equal(clones[1].from, 6, 'relative spacing preserved');
  assert.equal(clones[0].parent, clones[1].id, 'parenting inside the section survives');
  assert.deepEqual([...p.layers.map(l => l.id)].sort(), [...clones.map(c => c.id)].sort());
});

test('saveLook keeps code and uniform values; applyLook restores them in a new layer', () => {
  const PM = libraryModel();
  const p = baseProject(PM);
  const sh = PM.mkLayer('shader', { name: 'Ember' }, p);
  sh.d.code = 'uniform vec3 uTint; void main(){ fragColor = vec4(uTint, 1.); }';
  PM.syncShaderUniforms(sh);
  sh.d.uniforms.uTint.v = '#123456';
  p.layers.push(sh);
  const entry = PM.Library.saveLook('Ember saved', sh);
  assert.equal(entry.code.includes('vec4(uTint, 1.)'), true);
  assert.equal(entry.uniforms.uTint.v, '#123456');

  p.layers.length = 0;
  PM.Library.applyLook(entry.id);
  assert.equal(p.layers.length, 1);
  const L = p.layers[0];
  assert.equal(L.type, 'shader');
  assert.equal(L.d.code.includes('vec4(uTint, 1.)'), true);
  assert.equal(L.d.uniforms.uTint.v, '#123456', 'uniform values restored');
});

test('section deletion is recoverable and library storage stays bounded at 24', () => {
  const PM = libraryModel();
  baseProject(PM);
  const a = PM.mkLayer('solid', { name: 'A' }, PM.proj);
  PM.proj.layers.push(a);
  const e1 = PM.Library.saveSection('one');
  PM.Library.saveSection('two');
  assert.equal(PM.Library.all().sections.length, 2);
  const layersBefore = PM.proj.layers.map(layer => layer.id);
  PM.Library.trashSection(e1.id);
  assert.deepEqual([...PM.Library.catalog('project').map(s => s.name)], ['two'], 'trashed sections leave the active catalog');
  assert.equal(PM.Library.catalog('project', true).find(s => s.id === e1.id).deletedAt > 0, true);
  assert.equal(PM.proj.layers.map(layer => layer.id).join(','), layersBefore.join(','), 'deleting a reusable section does not delete scene layers');
  assert.equal(PM.__historyLabels.at(-1), 'Delete section');
  PM.Library.restoreSection(e1.id);
  assert.deepEqual([...PM.Library.catalog('project').map(s => s.name)].sort(), ['one', 'two']);
  assert.equal(PM.__historyLabels.at(-1), 'Restore section');

  for (let i = 0; i < 30; i++) PM.Library.saveSection('bulk ' + i);
  assert.equal(PM.Library.all().sections.length, 24, 'library stays bounded');
});

test('inserted sections retain an editable source reference instead of flattening', () => {
  const PM = libraryModel();
  const p = baseProject(PM);
  const layer = PM.mkLayer('shape', { name: 'Editable source' }, p);
  p.layers.push(layer);
  const section = PM.Library.saveSection('Reusable', [layer.id]);
  p.layers.length = 0;
  const [inserted] = PM.Library.insertSection(section.id);
  assert.equal(inserted.sectionRef.sectionId, section.id);
  assert.equal(inserted.sectionRef.sourceProjectId, p.id);
  assert.equal(typeof inserted.sectionRef.instanceId, 'string');
  assert.notEqual(inserted.id, layer.id, 'scene layer is a real editable clone with a stable source link');
});

test('library scope combines sections from multiple projects without parallel storage', () => {
  const PM = libraryModel();
  const first = baseProject(PM); first.name = 'Velocity Study';
  const firstLayer = PM.mkLayer('shape', { name: 'Hero' }, first); first.layers.push(firstLayer);
  PM.Library.saveSection('Hero block', [firstLayer.id]);

  const second = PM.mkProject({ name: 'Campaign', w: 1920, h: 1080, fps: 30, dur: 10 });
  const secondLayer = PM.mkLayer('text', { name: 'Title' }, second); second.layers.push(secondLayer);
  PM.proj = second;
  PM.Library.saveSection('Campaign title', [secondLayer.id]);
  PM.proj = first;
  const projects = new Map([[first.id, first], [second.id, second]]);
  PM.Projects = {
    list: () => [...projects.values()].map(project => ({ id: project.id, name: project.name })),
    get: id => projects.get(id), put: project => projects.set(project.id, project),
  };
  const names = PM.Library.catalog('global').map(section => `${section.sourceProjectName}:${section.name}`).sort();
  assert.deepEqual([...names], ['Campaign:Campaign title', 'Velocity Study:Hero block']);
  assert.strictEqual(second.library.sections[0].layers[0].name, 'Title', 'catalog reads the project-owned editable source');
});

/* ── chat rail migration ───────────────────────────────── */
function workspaceModel() {
  const PM = { uid: (p) => `${p}${++nextId}`, clamp: (v, a, b) => Math.max(a, Math.min(b, v)), h: () => ({}) };
  let nextId = 0;
  // uid closure must be shared with the module
  PM.uid = (p) => `${p}${++nextId}`;
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(utf8('js/core/workspace.js'), context);
  return PM;
}

test('normalization strips legacy docked chat panels', () => {
  const PM = workspaceModel();
  const w = PM.WS.normalize({
    id: 'w1', name: 'Legacy',
    layout: { docks: [
      { id: 'left', panels: [{ id: 'chat' }, { id: 'assets' }] },
      { id: 'center', panels: [{ id: 'viewer', flex: true }] },
    ] },
  });
  const panelIds = w.layout.docks.flatMap(d => d.panels.map(q => q.id));
  assert.equal(panelIds.includes('chat'), false, 'retired chat never renders inside a dock');
  assert.ok(panelIds.includes('assets'));
});

test('obsolete Generative panels migrate out while saved reusable content remains project-owned', () => {
  const src = utf8('js/core/workspace.js');
  const chatDocked = [...src.matchAll(/p\('chat'/g)];
  assert.equal(chatDocked.length, 0, 'presets reference no retired chat panel');
  assert.doesNotMatch(src, /p\('library'/, 'built-ins no longer dock the retired Generative panel');
  assert.doesNotMatch(utf8('js/ui/panels.js'), /registerPanel\('library'/, 'old panel registration is gone');
  assert.match(src, /id === 'library'/, 'saved manifests are narrowly migrated');
  assert.match(utf8('js/core/library.js'), /project\.library\.sections/, 'saved section data is not deleted');
  assert.match(utf8('js/core/library.js'), /project\.library\.looks/, 'saved look data is not deleted');
});

test('the spatial assistant is the only loaded assistant surface', () => {
  const index = utf8('index.html');
  const app = utf8('js/app.js');
  const shortcuts = utf8('js/ui/shortcuts.js');
  assert.match(index, /js\/assistant\/spatial\.js/);
  assert.doesNotMatch(index, /js\/agent\/|js\/ui\/chat/);
  assert.doesNotMatch(app, /ChatRail|Show assistant/);
  assert.doesNotMatch(shortcuts, /focusChat|Toggle assistant/);
});

test('library edits still autosave after the old agent runtime is removed', () => {
  assert.match(utf8('js/app.js'), /\['layers','project','assets','library'\]/, 'library mutations enter autosave');
});

test('snapshot renders in composition coordinates before thumbnail downscaling', () => {
  const src = utf8('js/core/exporter.js');
  assert.match(src, /renderFrameTo\(T, p\.w, p\.h\)/, 'full composition is rendered first');
  assert.match(src, /drawImage\(full, 0, 0, cv\.width, cv\.height\)/, 'finished frame is downscaled');
});
