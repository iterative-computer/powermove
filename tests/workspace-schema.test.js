const test = require('node:test');
const assert = require('node:assert/strict');
const Schema = require('../js/core/workspace-schema.js');

const legacy = {
  id: 'design', name: 'Design', density: 'normal',
  layout: { docks: [
    { id: 'left', size: 240, panels: [{ id: 'assets', flex: true }] },
    { id: 'center', panels: [{ id: 'viewer', flex: true }, { id: 'timeline', size: 280 }] },
    { id: 'right', size: 300, panels: [{ id: 'inspector', flex: true }] },
  ] },
};

test('migrates fixed docks into a recursive workspace without losing sections', () => {
  const workspace = Schema.normalizeWorkspace(legacy);
  assert.equal(workspace.manifestVersion, 2);
  assert.equal(workspace.layout.root.type, 'split');
  assert.deepEqual(Schema.listPanels(workspace).map((entry) => entry.panel), ['assets', 'viewer', 'timeline', 'inspector']);
  assert.deepEqual(workspace.layout.overlays, []);
  assert.deepEqual(workspace.layout.floating, []);
  const center = workspace.layout.root.children[1];
  assert.ok(center.sizes[0] > center.sizes[1], 'the flexible viewer remains larger than the fixed timeline');
  assert.ok(workspace.layout.root.sizes[1] > workspace.layout.root.sizes[0], 'the flexible center remains wider than the side dock');
});

test('repairs stale ratios that would make a section unreachable', () => {
  const sizes = Schema.normalizeSizes([0.003, 0.997], 2);
  assert.ok(sizes[0] >= 0.1);
  assert.equal(Math.round((sizes[0] + sizes[1]) * 1000), 1000);
});

test('places sections on every edge and as tabs', () => {
  let workspace = Schema.normalizeWorkspace(legacy);
  workspace = Schema.placePanel(workspace, Schema.panel('chat', { instance: 'chat-main' }), { target: 'viewer', where: 'left', duplicate: true });
  workspace = Schema.placePanel(workspace, Schema.panel('perf', { instance: 'perf-main' }), { target: 'timeline', where: 'bottom', duplicate: true });
  workspace = Schema.placePanel(workspace, Schema.panel('notes', { instance: 'notes-main' }), { target: 'inspector', where: 'tab', duplicate: true });
  const panels = Schema.listPanels(workspace);
  assert.equal(panels.length, 7);
  assert.ok(panels.some((entry) => entry.instance === 'chat-main'));
  assert.ok(panels.some((entry) => entry.instance === 'perf-main'));
  assert.ok(panels.some((entry) => entry.instance === 'notes-main'));
  const inspectorPath = Schema.findPath(workspace.layout.root, 'inspector');
  assert.equal(Schema.atPath(workspace.layout.root, inspectorPath.slice(0, -1)).type, 'tabs');
});

test('supports duplicate linked sections with unique instances', () => {
  let workspace = Schema.normalizeWorkspace(legacy);
  workspace = Schema.placePanel(workspace, Schema.panel('viewer', { instance: 'viewer' }), { target: 'viewer', where: 'right', duplicate: true });
  workspace = Schema.placePanel(workspace, Schema.panel('timeline', { instance: 'timeline' }), { target: 'timeline', where: 'bottom', duplicate: true });
  const viewers = Schema.listPanels(workspace).filter((entry) => entry.panel === 'viewer');
  const timelines = Schema.listPanels(workspace).filter((entry) => entry.panel === 'timeline');
  assert.equal(viewers.length, 2);
  assert.equal(timelines.length, 2);
  assert.equal(new Set(viewers.map((entry) => entry.instance)).size, 2);
  assert.equal(new Set(timelines.map((entry) => entry.instance)).size, 2);
});

test('moves sections into overlays and floating regions', () => {
  let workspace = Schema.normalizeWorkspace(legacy);
  const assets = Schema.listPanels(workspace).find((entry) => entry.panel === 'assets');
  workspace = Schema.placePanel(workspace, assets, { mode: 'overlay', anchor: 'top-right', width: 360, height: 240 });
  const inspector = Schema.listPanels(workspace).find((entry) => entry.panel === 'inspector');
  workspace = Schema.placePanel(workspace, inspector, { mode: 'floating', x: 120, y: 90 });
  assert.equal(workspace.layout.overlays.length, 1);
  assert.equal(workspace.layout.floating.length, 1);
  assert.equal(workspace.layout.overlays[0].node.panel, 'assets');
  assert.equal(workspace.layout.floating[0].node.panel, 'inspector');
});

test('removal prunes empty containers and preserves a valid tree', () => {
  let workspace = Schema.normalizeWorkspace(legacy);
  for (const instance of ['assets', 'timeline', 'inspector']) workspace = Schema.removeInstance(workspace, instance).workspace;
  const panels = Schema.listPanels(workspace);
  assert.deepEqual(panels.map((entry) => entry.panel), ['viewer']);
  assert.equal(workspace.layout.root.type, 'panel');
  assert.equal(Schema.validate(workspace, ['viewer']).ok, true);
});

test('validation rejects unknown sections and duplicate instances', () => {
  const unknown = Schema.normalizeWorkspace({ id: 'bad', layout: { root: Schema.panel('mystery') } });
  assert.equal(Schema.validate(unknown, ['viewer']).ok, false);
  const duplicate = {
    id: 'duplicate', name: 'Duplicate', density: 'normal', theme: {}, features: {}, custom: [],
    layout: { root: { type: 'split', direction: 'row', sizes: [.5, .5], children: [
      { type: 'panel', panel: 'viewer', instance: 'same' },
      { type: 'panel', panel: 'timeline', instance: 'same' },
    ] }, overlays: [], floating: [] },
  };
  assert.equal(Schema.validate(duplicate, ['viewer', 'timeline']).ok, false);
});
