const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = (name) => fs.readFileSync(path.join(root, name), 'utf8');

function editor() {
  let id = 0;
  const events = [];
  const PM = {
    version: 1,
    uid: (prefix) => `${prefix}-${++id}`,
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    round: (value, places = 0) => Number(Number(value).toFixed(places)),
    snapF: (time, fps) => Math.round(time * fps) / fps,
    Ease: {
      handles: () => ({ eo: [.33, 0], ei: [.67, 1] }),
      bezier: () => (value) => value,
      spring: (value) => value,
      nameOf: () => 'power',
    },
    SHADER_TEMPLATE: 'void main(){}',
    bus: { emit: (name) => events.push(name) },
    invalidate() {},
    toast() {},
    store: { get: (_key, fallback) => fallback, set() {} },
    FX: {},
    GL: { dropProgram() {} },
  };
  const context = vm.createContext({ window: { PM }, console, Date, JSON, Object, Set, Map });
  for (const file of ['js/core/model.js', 'js/core/anim.js']) vm.runInContext(source(file), context, { filename: file });
  PM.proj = PM.mkProject({ name: 'Test', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 1;
  PM.syncShaderUniforms = () => {};
  PM.mkEffect = () => null;
  vm.runInContext(source('js/core/history.js'), context, { filename: 'js/core/history.js' });
  vm.runInContext(source('js/core/editing.js'), context, { filename: 'js/core/editing.js' });
  return { PM, events };
}

function addText(PM, id = 'title') {
  const layer = PM.mkLayer('text', { name: 'Title' });
  layer.id = id;
  PM.addLayer(layer, 0);
  PM.selectLayers(layer.id);
  return layer;
}

test('canvas, agent, and generated UI origins use the same source command', () => {
  const outcomes = [];
  for (const origin of ['canvas', 'agent', 'generated-ui']) {
    const { PM } = editor();
    const layer = addText(PM);
    const result = PM.Edit.apply(
      { type: 'set_property', target: layer.id, path: 'position.x', value: 1040, mode: 'static', preserveHandEdits: false },
      { label: 'Move title', origin },
    );
    assert.equal(result.ok, true);
    outcomes.push({ value: layer.p['position.x'].v, revision: PM.proj.revision, operation: PM.proj.edits[0].operations[0] });
    assert.equal(PM.proj.edits[0].origin, origin);
  }
  assert.deepEqual(outcomes, outcomes.map(() => outcomes[0]));
});

test('a live gesture becomes one coalesced source transaction and one undo step', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const before = layer.p['position.x'].v;
  PM.Edit.begin('Move title', { origin: 'canvas' });
  for (const value of [980, 1000, 1040]) {
    const result = PM.Edit.dispatch({ type: 'set_property', target: layer.id, path: 'position.x', value, mode: 'static', preserveHandEdits: false });
    assert.equal(result.ok, true);
  }
  const committed = PM.Edit.commit();
  assert.equal(committed.ok, true);
  assert.equal(PM.proj.revision, 1);
  assert.equal(PM.proj.edits.length, 1);
  assert.equal(PM.proj.edits[0].operations.length, 1);
  assert.equal(PM.proj.edits[0].operations[0].value, 1040);
  assert.equal(PM.L(layer.id).p['position.x'].v, 1040);
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.L(layer.id).p['position.x'].v, before);
  assert.equal(PM.proj.revision, 0);
});

test('cancelling an empty control gesture preserves live source object bindings', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const project = PM.proj;
  const property = layer.p['position.x'];
  PM.Edit.begin('Adjust field', { origin: 'generated-ui' });
  assert.equal(PM.Edit.cancel(), true);
  assert.equal(PM.proj, project);
  assert.equal(PM.L(layer.id).p['position.x'], property);
  const result = PM.Edit.apply({ type: 'set_property', target: layer.id, path: 'position.x', value: 1110, mode: 'static', preserveHandEdits: false }, { origin: 'generated-ui' });
  assert.equal(result.ok, true);
  assert.equal(property.v, 1110);
});

test('scene parameter edits preserve generated control bindings', () => {
  const { PM } = editor();
  const parameter = PM.proj.params.Angle = {
    name: 'Angle', label: 'Gradient Angle', control: 'num', value: 90,
  };

  PM.Edit.apply({ type: 'set_scene_parameter', name: 'Angle', value: 45 }, {
    label: 'Gradient Angle', origin: 'generated-ui',
  });

  assert.equal(PM.proj.params.Angle, parameter);
  assert.equal(parameter.value, 45);
});

test('a failed multi-command edit rolls the entire source back', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const before = JSON.stringify(PM.proj);
  const result = PM.Edit.apply([
    { type: 'set_property', target: layer.id, path: 'opacity', value: 40, mode: 'static', preserveHandEdits: false },
    { type: 'set_property', target: layer.id, path: 'not-a-property', value: 2 },
  ], { label: 'Broken batch', origin: 'generated-ui' });
  assert.equal(result.ok, false);
  assert.match(result.message, /not-a-property/);
  assert.equal(JSON.stringify(PM.proj), before);
  assert.equal(PM.hist.canUndo(), false);
});

test('hand-authored property intent is preserved unless overwrite is explicit', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const human = PM.Edit.apply({
    type: 'set_property', target: layer.id, path: 'position.x', value: 1000,
    mode: 'static', preserveHandEdits: false, markIntent: 'human',
  }, { label: 'Canvas move', origin: 'canvas' });
  assert.equal(human.ok, true);
  const preserved = PM.Edit.apply({
    type: 'set_property', target: layer.id, path: 'position.x', value: 1200, mode: 'static',
  }, { label: 'Agent move', origin: 'agent' });
  assert.equal(preserved.ok, false);
  assert.equal(PM.L(layer.id).p['position.x'].v, 1000);
  const explicit = PM.Edit.apply({
    type: 'set_property', target: layer.id, path: 'position.x', value: 1200,
    mode: 'static', preserveHandEdits: false,
  }, { label: 'Explicit move', origin: 'agent' });
  assert.equal(explicit.ok, true);
  assert.equal(PM.L(layer.id).p['position.x'].v, 1200);
});

test('all editing surfaces are wired to the shared source command boundary', () => {
  const index = source('index.html');
  const viewer = source('js/ui/viewer.js');
  const timeline = source('js/ui/timeline.js');
  const inspector = source('js/ui/inspector.js');
  const controls = source('js/ui/controls.js');
  const workspace = source('js/core/workspace.js');
  const agent = source('js/agent/tools.js');
  assert.match(index, /js\/core\/editing\.js/);
  assert.match(viewer, /PM\.Edit\.dispatch/);
  assert.match(timeline, /origin: 'timeline'/);
  assert.match(inspector, /origin: 'inspector'/);
  assert.match(controls, /opt\.command/);
  assert.match(workspace, /origin: 'generated-ui'/);
  assert.match(workspace, /path\.startsWith\('properties\.'\)/);
  assert.match(workspace, /PM\.proj\.params\[param\.name\]/);
  assert.match(workspace, /\['draw:ui', 'project', 'history'\]/);
  assert.match(agent, /reg\('edit_source'/);
});
