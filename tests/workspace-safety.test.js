const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

function workspaceModel() {
  let nextId = 0;
  const PM = {
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    uid: (prefix) => `${prefix}${++nextId}`,
    store: { get: (_key, fallback) => fallback, set() {} },
    bus: { emit() {} },
    registerPanel() {},
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8'), context);
  return PM.WS;
}

test('agent-authored workspaces always retain a fluid main dock', () => {
  const WS = workspaceModel();
  const workspace = WS.normalize({
    id: 'gradient', name: 'Gradient', layout: { docks: [
      { id: 'sidebar', size: 600, panels: [{ id: 'gradient-editor', flex: true }] },
      { id: 'main', size: 680, panels: [{ id: 'viewer' }, { id: 'timeline', size: 300 }] },
    ] },
  });
  const main = workspace.layout.docks.find(d => d.id === 'main');
  assert.equal(main.flex, true);
  assert.equal(main.size, 680);
});

test('common agent control aliases are repaired without rendering undefined', () => {
  const WS = workspaceModel();
  const workspace = WS.normalize({
    id: 'gradient', name: 'Gradient', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
    custom: [{ name: 'Gradient Editor', controls: [
      { type: 'color', parameter: 'Gradient Start', label: 'Control 1', param: 'Control 1', default: '#112233' },
      { name: 'Angle', value: 45, min: 0, max: 360 },
      {},
    ] }],
  });
  const [color, angle, fallback] = workspace.custom[0].controls;
  assert.deepEqual({ label: color.label, param: color.param, def: color.def }, { label: 'Gradient Start', param: 'Gradient Start', def: '#112233' });
  assert.deepEqual({ type: angle.type, label: angle.label, def: angle.def }, { type: 'slider', label: 'Angle', def: 45 });
  assert.equal(fallback.label, 'Control 3');
  assert.notEqual(fallback.param, 'undefined');
});

test('invalid replacement docks fall back to the previous usable layout', () => {
  const WS = workspaceModel();
  const fallback = { id: 'safe', name: 'Safe', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer', flex: true }] }] } };
  const workspace = WS.normalize({ id: 'safe', name: 'Broken', layout: { docks: [] } }, fallback);
  assert.equal(workspace.layout.docks[0].panels[0].id, 'viewer');
  assert.equal(workspace.layout.docks[0].flex, true);
});
