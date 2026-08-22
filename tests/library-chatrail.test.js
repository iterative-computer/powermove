const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

/* ── library harness: easing + model + library with UI/runtime stubs ── */
function libraryModel() {
  let nextId = 0;
  const PM = {
    clamp: (v, a, b) => Math.max(a, Math.min(b, v)),
    uid: (p) => `${p}${++nextId}`,
    snapF: (t) => Math.round(t * 30) / 30,
    time: 0,
    round: (v) => v,
    bus: { emit() {}, on() {} },
    invalidate: () => {},
    toast: () => {},
    hist: { do: (_label, fn) => fn() },
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

test('drop removes exactly one library entry and trims overflow at 24', () => {
  const PM = libraryModel();
  baseProject(PM);
  const a = PM.mkLayer('solid', { name: 'A' }, PM.proj);
  PM.proj.layers.push(a);
  const e1 = PM.Library.saveSection('one');
  PM.Library.saveSection('two');
  assert.equal(PM.Library.all().sections.length, 2);
  PM.Library.drop('sections', e1.id);
  assert.deepEqual([...PM.Library.all().sections.map(s => s.name)], ['two']);

  for (let i = 0; i < 30; i++) PM.Library.saveSection('bulk ' + i);
  assert.equal(PM.Library.all().sections.length, 24, 'library stays bounded');
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

test('no built-in preset docks retired assistant UI; Design keeps the generative library', () => {
  const src = utf8('js/core/workspace.js');
  const chatDocked = [...src.matchAll(/p\('chat'/g)];
  assert.equal(chatDocked.length, 0, 'presets reference no retired chat panel');
  assert.match(src, /p\('library', \{ size: 220 \}\)/, 'Design preset includes the Generative panel');
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
