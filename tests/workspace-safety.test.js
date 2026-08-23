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
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/capabilities.js'), 'utf8'), context);
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
      name: 'Test', w: 1920, h: 1080, fps: 30, dur: 10, shutter: .5, work: [0, 10],
      bg: '#111111',
      backgroundFill: { type: 'linear', angle: 15, stops: [
        { id: 'start', color: '#112233', position: 0 },
        { id: 'end', color: '#445566', position: 100 },
      ] },
    },
    normalizeFill(value) { return JSON.parse(JSON.stringify(value)); },
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/capabilities.js'), 'utf8'), context);
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

test('generated composition controls expose real width, height, timing, and work-area source', () => {
  const PM = workspaceRuntime();
  const width = PM.WS.sourceBinding({ type: 'slider', target: '$composition', path: 'composition.width', def: 16 });
  const height = PM.WS.sourceBinding({ type: 'slider', target: '$composition', path: 'composition.height', def: 16 });
  const start = PM.WS.sourceBinding({ type: 'slider', target: '$composition', path: 'composition.workArea.start', def: 0 });
  assert.equal(width.get(), 1920); assert.equal(height.get(), 1080);
  const widthCommand = width.command(2560), heightCommand = height.command(1440), startCommand = start.command(2);
  assert.equal(widthCommand.type, 'set_composition'); assert.equal(widthCommand.patch.width, 2560);
  assert.equal(heightCommand.type, 'set_composition'); assert.equal(heightCommand.patch.height, 1440);
  assert.deepEqual([...startCommand.patch.workArea], [2, 10]);
});

test('boot refreshes stale built-in presets while preserving custom workspaces', () => {
  const file = fs.readFileSync(path.join(root, 'js/core/workspace.js'), 'utf8');
  assert.match(file, /workspace\.id === preset\.id && workspace\.builtin/);
  assert.match(file, /WS\.all\[index\] = normalizeWorkspace\(preset\)/);
  assert.doesNotMatch(file, /findIndex\(workspace => workspace\.id === preset\.id\)(?! && workspace\.builtin)/,
    'a custom workspace is never replaced merely because its id resembles a preset');
});

test('the exact broken shortcut-only Layer Stagger panel migrates to the real generated tool', () => {
  const WS = workspaceModel();
  const controls = ['selectAll', 'deselect', 'prevEdge', 'nextEdge', 'prevFrame', 'nextFrame', 'split']
    .map((cmd, index) => ({ type: 'button', label: `Legacy ${index}`, cmd }));
  const workspace = WS.normalize({
    id: 'custom', name: 'Custom', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }, { id: 'layer-stagger-tools' }] }] },
    custom: [{ id: 'layer-stagger-tools', title: 'Layer Stagger', controls }],
  });
  const panel = workspace.custom[0];
  assert.equal(panel.id, 'layer-stagger-tools', 'the saved panel remains in the same layout slot');
  assert.equal(panel.tool, 'layer-stagger');
  assert.deepEqual([...panel.controls.map(control => control.label)], [
    'Selection', 'Offset', 'Order', 'Anchor', 'Preview', 'Apply Stagger', 'Undo last edit',
  ]);
  assert.equal(panel.controls.some(control => control.cmd === 'split'), false);
  assert.equal(panel.controls.find(control => control.label === 'Apply Stagger').action.type, 'transform');
  assert.doesNotMatch(panel.note, /split|frame navigation/i);
});

test('unrelated custom command panels are never rewritten by the stagger migration', () => {
  const WS = workspaceModel();
  const workspace = WS.normalize({
    id: 'custom', name: 'Custom', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }, { id: 'my-tools' }] }] },
    custom: [{ id: 'my-tools', title: 'My Tools', controls: [{ type: 'button', label: 'Split', cmd: 'split' }] }],
  });
  assert.equal(workspace.custom[0].tool, undefined);
  assert.equal(workspace.custom[0].controls[0].cmd, 'split');
});

test('the exact broken Text Splitter panel migrates to the working Decompose Text script', () => {
  const WS = workspaceModel();
  const workspace = WS.normalize({
    id: 'custom', name: 'Custom', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }, { id: 'text-splitter' }] }] },
    custom: [{ id: 'text-splitter', title: 'Text Splitter', controls: [
      { type: 'text', label: 'text', def: 'Powermove' },
      { type: 'button', label: 'Duplicate source layer', cmd: 'duplicate' },
      { type: 'button', label: 'Add text layer', cmd: 'addText' },
      { type: 'button', label: 'Undo last split step', cmd: 'undo' },
    ] }],
  });
  const panel = workspace.custom[0];
  assert.equal(panel.id, 'text-splitter', 'the existing panel keeps its layout identity');
  assert.equal(panel.tool, 'decompose-text');
  assert.equal(panel.title, 'Text Splitter', 'the saved user-facing title is preserved');
  assert.deepEqual([...panel.controls.map(control => control.label)], [
    'Selection', 'Split into', 'Original', 'Preview', 'Decompose', 'Undo last edit',
  ]);
  assert.equal(panel.controls.find(control => control.label === 'Decompose').action.type, 'script');
  assert.equal(panel.controls.some(control => control.cmd === 'duplicate'), false);
});

test('Timeline interface manifests are clamped, persisted, and limited to visual configuration', () => {
  const WS = workspaceModel();
  const edit = WS.sanitizeInterfaceEdit(JSON.stringify({ target: 'timeline', patch: {
    rowHeight: 12, gutterWidth: 900, rulerHeight: 30, clipRadius: 5, keyframeSize: 20,
    showLayerNumbers: false, showTypeBadges: false, toolbarDensity: 'compact', projectLayers: [],
  } }));
  assert.deepEqual({ ...edit.patch }, {
    rowHeight: 22, gutterWidth: 360, rulerHeight: 30, clipRadius: 5, keyframeSize: 12,
    showLayerNumbers: false, showTypeBadges: false, toolbarDensity: 'compact',
  });
  const workspace = WS.normalize({ id: 'w', name: 'W', layout: { docks: [{ id: 'center', panels: [{ id: 'viewer' }] }] } });
  assert.equal(WS.applyInterfaceEdit(workspace, edit), true);
  assert.equal(workspace.chrome.timeline.rowHeight, 22);
  assert.equal(workspace.chrome.timeline.gutterWidth, 360);
  assert.equal('projectLayers' in workspace.chrome.timeline, false);
});

test('workspace action menus render above the Library but below modal dialogs', () => {
  const css = fs.readFileSync(path.join(root, 'css/app.css'), 'utf8');
  const drop = Number(css.match(/\.drop\{position:absolute;z-index:(\d+)/)?.[1]);
  const library = Number(css.match(/#library-screen\{[^}]*z-index:(\d+)/)?.[1]);
  const modal = Number(css.match(/\.modal\{position:fixed;z-index:(\d+)/)?.[1]);
  assert.ok(drop > library, `menu z-index ${drop} must clear Library z-index ${library}`);
  assert.ok(drop < modal, `menu z-index ${drop} must stay below modal z-index ${modal}`);
});
