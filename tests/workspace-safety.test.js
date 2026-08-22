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

function workspaceRuntime() {
  let nextId = 0;
  const PM = {
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    uid: prefix => `${prefix}${++nextId}`,
    store: { get: (_key, fallback) => fallback, set() {} },
    bus: { emit() {} }, registerPanel() {},
    proj: {
      bg: '#111111',
      backgroundFill: { type: 'linear', angle: 15, stops: [
        { id: 'start', color: '#112233', position: 0 },
        { id: 'end', color: '#445566', position: 100 },
      ] },
    },
    normalizeFill(value) { return JSON.parse(JSON.stringify(value)); },
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8'), context);
  return PM;
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

test('the malformed app-only Gradient layout is narrowly recognized for migration', () => {
  const WS = workspaceModel();
  const legacy = {
    id: 'saved-gradient', name: 'Gradient', builtin: false,
    layout: { docks: [
      { id: 'left', panels: [{ id: 'gradient-controls' }, { id: 'assets' }] },
      { id: 'right', panels: [{ id: 'viewer' }, { id: 'timeline' }, { id: 'layers' }, { id: 'inspector' }] },
    ] },
  };
  assert.equal(WS.isLegacyGradient(legacy), true);
  assert.equal(WS.isLegacyGradient({ ...legacy, name: 'My Gradient' }), false);
  assert.equal(WS.isLegacyGradient({ ...legacy, layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] } }), false);
});

test('hidden panel recovery metadata survives validation while retired Generative metadata does not', () => {
  const WS = workspaceModel();
  const workspace = WS.normalize({
    id: 'custom', name: 'Custom',
    hiddenPanels: [
      { id: 'assets', dockId: 'left', dockIndex: 0, index: 1, spec: { id: 'assets', size: 180 } },
      { id: 'library', dockId: 'left', index: 0, spec: { id: 'library' } },
    ],
    layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] },
  });
  assert.equal(workspace.hiddenPanels.length, 1);
  assert.equal(workspace.hiddenPanels[0].id, 'assets');
  assert.equal(workspace.hiddenPanels[0].spec.size, 180);
});

test('generated gradient controls read and write the real composition background source', () => {
  const PM = workspaceRuntime();
  const binding = PM.WS.sourceBinding({
    type: 'color', label: 'End color', target: '$composition',
    path: 'composition.background.endColor', def: '#000000',
  });
  assert.ok(binding);
  assert.equal(binding.get(), '#445566');
  const command = binding.command('#AABBCC');
  assert.equal(command.type, 'set_composition');
  assert.equal(command.patch.backgroundFill.stops[1].color, '#AABBCC');
  assert.equal(PM.proj.backgroundFill.stops[1].color, '#445566', 'building the command does not mutate source early');
});

test('boot refreshes stale built-in presets while preserving custom workspaces', () => {
  const file = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');
  assert.match(file, /workspace\.id === preset\.id && workspace\.builtin/);
  assert.match(file, /WS\.all\[index\] = normalizeWorkspace\(preset\)/);
  assert.doesNotMatch(file, /findIndex\(workspace => workspace\.id === preset\.id\)(?! && workspace\.builtin)/,
    'a custom workspace is never replaced merely because its id resembles a preset');
});
