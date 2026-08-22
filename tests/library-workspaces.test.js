const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const ui = fs.readFileSync(path.join(root, 'js/ui/library.js'), 'utf8');
const index = fs.readFileSync(path.join(root, 'index.html'), 'utf8');
const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');

function workspaceModel() {
  let nextId = 0;
  const memory = new Map();
  const PM = {
    proj: { id: 'project-1' },
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    uid: prefix => `${prefix}${++nextId}`,
    store: { get: (key, fallback) => memory.has(key) ? memory.get(key) : fallback, set: (key, value) => memory.set(key, value) },
    bus: { emit() {} }, registerPanel() {},
  };
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8'), vm.createContext({ window: { PM }, console }));
  return { PM, memory };
}

test('Library exposes Sections, Workspaces, scope, previews, and explicit save decisions', () => {
  assert.match(index, /js\/ui\/library\.js/);
  assert.match(ui, />?Sections|['"]Sections['"]/);
  assert.match(ui, />?Workspaces|['"]Workspaces['"]/);
  assert.match(ui, /This project/);
  assert.match(ui, /My library/);
  assert.match(ui, /Update section/);
  assert.match(ui, /Save as new/);
  assert.match(ui, /Existing placed layers remain exactly as edited/);
  assert.match(ui, /Apply workspace\?/);
  assert.match(ui, /Delete section/);
});

test('Library is a focus-trapped modal with a full scrim and safe close paths', () => {
  assert.match(ui, /div#library-overlay/);
  assert.match(ui, /'aria-modal': 'true'/);
  assert.match(ui, /document\.getElementById\('app'\)\.inert = true/);
  assert.match(ui, /document\.getElementById\('app'\)\.inert = false/);
  assert.match(ui, /event\.target === state\.overlay\) close\(\)/);
  assert.match(ui, /event\.key === 'Escape'/);
  assert.match(ui, /event\.key !== 'Tab'/);
  assert.match(css, /#library-overlay\{[^}]*position:fixed[^}]*inset:0[^}]*background:rgba\(12,12,15,\.48\)/s);
  assert.match(css, /#library-screen\{[^}]*position:relative[^}]*width:min\(1120px,calc\(100vw - 36px\)\)/s);
});

test('Library belongs only to the project tab where it opened', () => {
  assert.match(ui, /state\.originProjectId = PM\.proj\.id/);
  assert.match(ui, /PM\.bus\.on\('project'/);
  assert.match(ui, /state\.originProjectId !== PM\.proj\.id\) close\(\)/);
  assert.doesNotMatch(ui, /openTabs|splice|sort/, 'modal scoping never mutates tab order');
});

test('workspace manifests always preserve Composition and are versioned', () => {
  const { PM } = workspaceModel();
  const repaired = PM.WS.normalize({
    id: 'custom', name: 'Custom', scope: 'project', projectId: 'project-1',
    layout: { docks: [{ id: 'left', panels: [{ id: 'assets' }] }] },
  });
  assert.equal(repaired.schemaVersion, 1);
  assert.equal(repaired.scope, 'project');
  assert.equal(repaired.projectId, 'project-1');
  assert.equal(repaired.layout.docks.some(dock => dock.panels.some(panel => panel.id === 'viewer')), true);
});

test('custom workspace deletion is recoverable', () => {
  const { PM } = workspaceModel();
  const custom = PM.WS.normalize({ id: 'custom', name: 'Custom', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] } });
  const design = PM.WS.normalize({ id: 'design', name: 'Design', builtin: true, layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] } });
  PM.WS.all = [design, custom];
  PM.WS.current = custom;
  PM.WS.save = () => {};
  PM.WS.activate = id => { PM.WS.current = PM.WS.get(id); };
  PM.WS.remove('custom');
  assert.equal(PM.WS.get('custom'), undefined);
  assert.equal(PM.WS.trashList()[0].id, 'custom');
  const restored = PM.WS.restore('custom');
  assert.equal(restored.id, 'custom');
  assert.equal(PM.WS.get('custom').name, 'Custom');
});

test('temporary workspace editing has Save, Save as new, and Cancel recovery paths', () => {
  assert.match(ui, /PM\.WS\.beginEdit/);
  assert.match(ui, /PM\.WS\.cancelEdit/);
  assert.match(ui, /PM\.WS\.saveEdit\(false\)/);
  assert.match(ui, /PM\.WS\.saveEdit\(asNew/);
  assert.match(ui, /PM\.Layout\.addPanel/);
  assert.match(ui, /PM\.Layout\.hidePanel/);
  assert.match(ui, /PM\.Layout\.restorePanel/);
});
