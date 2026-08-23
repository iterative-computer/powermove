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

test('procedural scripts can add parented styled layers atomically and undo them', () => {
  const { PM } = editor();
  const sourceLayer = addText(PM);
  sourceLayer.blend = 'screen';
  const result = PM.Edit.apply([{
    type: 'add_layer', id: 'piece-a', layerType: 'text', name: 'A',
    content: { ...sourceLayer.d, text: 'A', align: 'left' },
    properties: { 'position.x': -40, 'position.y': 0 },
    parent: sourceLayer.id, blend: sourceLayer.blend, motionBlur: true,
    from: sourceLayer.from, duration: sourceLayer.dur, select: false,
  }, {
    type: 'set_layer', target: sourceLayer.id, patch: { visible: false },
  }], { label: 'Decompose text', origin: 'generated-script' });
  assert.equal(result.ok, true);
  assert.equal(PM.L('piece-a').parent, sourceLayer.id);
  assert.equal(PM.L('piece-a').blend, 'screen');
  assert.equal(PM.L('piece-a').mblur, true);
  assert.equal(PM.L(sourceLayer.id).on, false);
  assert.equal(PM.proj.revision, 1);
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.L('piece-a'), null);
  assert.equal(PM.L(sourceLayer.id).on, true);
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

test('an interrupted source gesture restores its edits and the next canvas gesture still works', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const before = layer.p['position.x'].v;

  PM.Edit.begin('Interrupted resize', { origin: 'canvas' });
  PM.Edit.dispatch({ type: 'set_property', target: layer.id, path: 'position.x', value: before + 80, mode: 'static', preserveHandEdits: false });
  assert.equal(PM.Edit.cancel(), true);
  assert.equal(PM.L(layer.id).p['position.x'].v, before, 'the partial gesture is rolled back');

  PM.Edit.begin('Move after interruption', { origin: 'canvas' });
  const moved = PM.Edit.dispatch({ type: 'set_property', target: layer.id, path: 'position.x', value: before + 40, mode: 'static', preserveHandEdits: false });
  assert.equal(moved.ok, true);
  assert.equal(PM.Edit.commit().ok, true);
  assert.equal(PM.L(layer.id).p['position.x'].v, before + 40, 'later manipulation is not wedged');
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

test('Composition background color commits through source history and is undoable', () => {
  const { PM } = editor();
  const before = PM.proj.bg;
  const result = PM.Edit.apply({ type: 'set_composition', patch: { background: '#3366CC' } }, {
    label: 'Background', origin: 'inspector',
  });
  assert.equal(result.ok, true);
  assert.equal(PM.proj.bg, '#3366CC');
  assert.equal(PM.proj.edits.at(-1).operations[0].patch.background, '#3366CC');
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.proj.bg, before);
  assert.equal(PM.hist.redo(), true);
  assert.equal(PM.proj.bg, '#3366CC');
});

test('the live editable-source catalog includes complete composition and selected-layer controls', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const catalog = PM.Edit.sourceCatalog();
  const compositionPaths = new Set(catalog.composition.map(control => control.path));
  for (const path of ['composition.name', 'composition.width', 'composition.height', 'composition.fps', 'composition.duration', 'composition.workArea.start', 'composition.workArea.end', 'composition.backgroundFill']) {
    assert.equal(compositionPaths.has(path), true, `${path} is agent-bindable`);
  }
  const controls = new Set(catalog.layers.find(item => item.id === layer.id).controls.map(control => control.path));
  for (const path of ['layer.name', 'layer.visible', 'layer.locked', 'layer.solo', 'layer.blend', 'content.text', 'content.font', 'properties.position.x', 'properties.opacity']) {
    assert.equal(controls.has(path), true, `${path} is agent-bindable`);
  }
  assert.equal(PM.Edit.apply({ type: 'set_composition', patch: { width: 2560, height: 1440 } }, { origin: 'generated-ui' }).ok, true);
  assert.deepEqual([PM.proj.w, PM.proj.h], [2560, 1440]);
});

test('Composition gradients preserve editable stops and undo as one source edit', () => {
  const { PM } = editor();
  const before = JSON.stringify(PM.proj.backgroundFill);
  const gradient = PM.normalizeFill({ type: 'linear', angle: 35, stops: [
    { id: 'a', color: '#FF0000', position: 0 },
    { id: 'b', color: '#0000FF', position: 100 },
  ] });
  assert.equal(PM.Edit.apply({ type: 'set_composition', patch: { backgroundFill: gradient } }, {
    label: 'Background fill', origin: 'inspector',
  }).ok, true);
  assert.equal(PM.proj.backgroundFill.type, 'linear');
  assert.equal(PM.proj.backgroundFill.angle, 35);
  assert.deepEqual(PM.proj.backgroundFill.stops.map(stop => [stop.id, stop.color, stop.position]), [
    ['a', '#FF0000', 0], ['b', '#0000FF', 100],
  ]);
  assert.equal(PM.hist.undo(), true);
  assert.equal(JSON.stringify(PM.proj.backgroundFill), before);
  assert.equal(PM.hist.redo(), true);
  assert.equal(PM.proj.backgroundFill.stops[1].position, 100);
});

test('Composition clear fill remains editable and undoable', () => {
  const { PM } = editor();
  const before = JSON.stringify(PM.proj.backgroundFill);
  const clear = PM.normalizeFill({ type: 'none', stops: PM.proj.backgroundFill.stops });
  assert.equal(PM.Edit.apply({ type: 'set_composition', patch: { backgroundFill: clear } }, {
    label: 'Clear background fill', origin: 'inspector',
  }).ok, true);
  assert.equal(PM.proj.backgroundFill.type, 'none');
  assert.equal(PM.hist.undo(), true);
  assert.equal(JSON.stringify(PM.proj.backgroundFill), before);
});

test('effect addition is one source transaction and one undo step', () => {
  const { PM } = editor();
  const layer = addText(PM);
  PM.mkEffect = (type) => ({ id: PM.uid('f'), type, on: true, open: false, p: { amount: PM.P(24) } });

  const result = PM.Edit.apply({ type: 'add_effect', target: layer.id, effect: 'blur' }, {
    label: 'Add Gaussian Blur', origin: 'effects-panel',
  });

  assert.equal(result.ok, true);
  assert.equal(PM.L(layer.id).fx.length, 1);
  assert.equal(PM.hist.list().at(-1), 'Add Gaussian Blur');
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.L(layer.id).fx.length, 0);
  assert.equal(PM.hist.redo(), true);
  assert.equal(PM.L(layer.id).fx.length, 1);
});

test('section creation and updates cross the same undoable source transaction boundary', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const section = {
    id: 'section-1', name: 'Title section', layers: [JSON.parse(JSON.stringify(layer))],
    versions: [], tags: ['text'], schemaVersion: 1,
  };
  assert.equal(PM.Edit.apply({ type: 'create_section', section }, { label: 'Save section', origin: 'library' }).ok, true);
  assert.equal(PM.proj.library.sections[0].id, 'section-1');
  const replacement = JSON.parse(JSON.stringify(layer)); replacement.name = 'Updated title';
  assert.equal(PM.Edit.apply({
    type: 'update_section', sectionId: 'section-1', layers: [replacement],
    version: { id: 'version-2', layers: [replacement], at: 2 },
  }, { label: 'Update section', origin: 'library' }).ok, true);
  assert.equal(PM.proj.library.sections[0].layers[0].name, 'Updated title');
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.proj.library.sections[0].layers[0].name, 'Title');
  assert.equal(PM.hist.undo(), true);
  assert.equal(PM.proj.library?.sections?.length || 0, 0);
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
  const panels = source('js/ui/panels.js');
  const shortcuts = source('js/ui/shortcuts.js');
  const controls = source('js/ui/controls.js');
  const workspace = source('js/core/workspace.js');
  const capabilities = source('js/core/capabilities.js');
  const spatial = source('js/assistant/spatial.js');
  assert.match(index, /js\/core\/editing\.js/);
  assert.match(index, /js\/core\/capabilities\.js/);
  assert.match(capabilities, /PM\.Edit\.apply/);
  assert.match(capabilities, /sanitizeTransform/);
  assert.match(viewer, /PM\.Edit\.dispatch/);
  assert.match(timeline, /origin: 'timeline'/);
  assert.match(inspector, /origin: 'inspector'/);
  assert.match(panels, /if \(event\.detail > 1\) return;/, 'double-click follow-up cannot add duplicate effects');
  assert.doesNotMatch(panels, /row\.ondblclick = row\.onclick/, 'effect rows have only one activation path');
  assert.match(shortcuts, /origin: 'command-palette'/, 'palette effects use source transactions');
  assert.match(controls, /opt\.command/);
  assert.match(workspace, /origin: 'generated-ui'/);
  assert.match(workspace, /path\.startsWith\('properties\.'\)/);
  assert.match(workspace, /PM\.proj\.params\[param\.name\]/);
  assert.match(workspace, /\['draw:ui', 'project', 'history', 'sel', 'layers'\]/);
  assert.match(spatial, /workspace\.custom\.push/, 'generated sections enter the validated workspace model');
  assert.match(spatial, /PM\.WS\.mutate/, 'spatial changes use the shared workspace mutation boundary');
});
