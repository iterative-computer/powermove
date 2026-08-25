// Documents CURRENT behavior as of the legacy app; the policy matrix will be revised deliberately in Phase 3a.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = (name) => fs.readFileSync(path.join(root, name), 'utf8');

function fixture() {
  let id = 0;
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
    bus: { emit() {} },
    invalidate() {},
    toast() {},
    store: { get: (_key, fallback) => fallback, set() {} },
    FX: {},
    GL: { dropProgram() {} },
  };
  const context = vm.createContext({ window: { PM }, console, Date, JSON, Object, Set, Map });
  for (const file of ['js/core/model.js', 'js/core/anim.js', 'js/gl/shaders.js']) {
    vm.runInContext(source(file), context, { filename: file });
  }
  PM.proj = PM.mkProject({ name: 'Lock fixture', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 1;
  PM.syncShaderUniforms = () => {};
  vm.runInContext(source('js/core/history.js'), context, { filename: 'js/core/history.js' });
  vm.runInContext(source('js/core/editing.js'), context, { filename: 'js/core/editing.js' });
  vm.runInContext(source('js/core/capabilities.js'), context, { filename: 'js/core/capabilities.js' });

  const layer = PM.mkLayer('text', { name: 'Locked title' });
  layer.id = 'locked-title';
  layer.lock = true;
  const other = PM.mkLayer('solid', { name: 'Other' });
  other.id = 'other';
  PM.addLayer(layer, 0);
  PM.addLayer(other, 1);
  PM.selectLayers(layer.id);
  PM.setKeyOn(layer.p.opacity, 0, 0, 'linear', PM.proj.fps);
  PM.setKeyOn(layer.p.opacity, 1, 100, 'linear', PM.proj.fps);
  const effect = PM.mkEffect('blur');
  effect.id = 'existing-effect';
  layer.fx.push(effect);
  return { PM, layer, other, effect };
}

const lockedCases = {
  set_property: ({ layer }) => ({
    type: 'set_property', target: layer.id, path: 'position.x', value: 100,
    mode: 'static', preserveHandEdits: false,
  }),
  replace_keyframes: ({ layer }) => ({
    type: 'replace_keyframes', target: layer.id, path: 'opacity',
    keyframes: [{ time: 0, value: 10 }, { time: 1, value: 90 }], preserveHandEdits: false,
  }),
  set_easing: ({ layer }) => ({
    type: 'set_easing', keyframes: layer.p.opacity.kf.map(key => key.i), curve: [.2, .1, .8, .9],
  }),
  set_expression: ({ layer }) => ({ type: 'set_expression', target: layer.id, path: 'opacity', expression: 'value + 1' }),
  set_content: ({ layer }) => ({ type: 'set_content', target: layer.id, patch: { text: 'Changed' } }),
  set_layer: ({ layer }) => ({ type: 'set_layer', target: layer.id, patch: { name: 'Renamed while locked' } }),
  delete_layers: ({ layer }) => ({ type: 'delete_layers', targets: [layer.id] }),
  reorder_layer: ({ layer }) => ({ type: 'reorder_layer', target: layer.id, index: 1 }),
  add_effect: ({ layer }) => ({ type: 'add_effect', target: layer.id, effect: 'blur' }),
  remove_effect: ({ layer, effect }) => ({ type: 'remove_effect', target: layer.id, effect: effect.id }),
  set_effect: ({ layer, effect }) => ({ type: 'set_effect', target: layer.id, effect: effect.id, patch: { enabled: false } }),
};

const successfulMutation = {
  set_expression: ({ layer }) => assert.equal(layer.p.opacity.expr, 'value + 1'),
  set_layer: ({ layer }) => assert.equal(layer.name, 'Renamed while locked'),
  delete_layers: ({ PM, layer }) => assert.equal(PM.proj.layers.some(item => item.id === layer.id), false),
  reorder_layer: ({ PM, layer }) => assert.equal(PM.proj.layers.findIndex(item => item.id === layer.id), 1),
  add_effect: ({ PM, layer }) => {
    assert.equal(layer.fx.length, 2);
    assert.deepEqual(Object.keys(layer.fx.at(-1).p), Array.from(PM.FX.blur.params, parameter => parameter.k));
  },
  remove_effect: ({ layer }) => assert.equal(layer.fx.length, 0),
  set_effect: ({ PM, effect }) => {
    assert.deepEqual(Object.keys(effect.p), Array.from(PM.FX.blur.params, parameter => parameter.k));
    assert.equal(effect.on, false);
  },
  set_easing: ({ layer }) => {
    for (const key of layer.p.opacity.kf) {
      assert.deepEqual([...key.eo, ...key.ei], [.2, .1, .8, .9]);
      assert.equal(key.hold, false);
    }
  },
};

test('the legacy locked-layer operation matrix is frozen', () => {
  const outcomes = {
    set_property: 'throws',
    replace_keyframes: 'throws',
    set_easing: 'succeeds',
    set_expression: 'succeeds',
    set_content: 'throws',
    set_layer: 'succeeds',
    delete_layers: 'succeeds',
    reorder_layer: 'succeeds',
    add_effect: 'succeeds',
    remove_effect: 'succeeds',
    set_effect: 'succeeds',
  };

  for (const [operation, expected] of Object.entries(outcomes)) {
    const state = fixture();
    const result = state.PM.Edit.apply([lockedCases[operation](state)]);
    if (expected === 'throws') {
      assert.equal(result.ok, false, `${operation} unexpectedly succeeded`);
      assert.match(result.message, /Layer “Locked title” is locked/);
    } else {
      assert.equal(result.ok, true, `${operation}: ${result.message}`);
      successfulMutation[operation](state);
    }
  }
});

test('transform_layers rolls back unlocked edits when a compiled property edit reaches a locked layer', () => {
  const { PM, layer, other } = fixture();
  const lockedBefore = layer.p.opacity.v;
  const otherBefore = other.p.opacity.v;
  /* Capabilities currently filters locked targets before compiling. Inject the
     representative expansion here to freeze Edit's lock and rollback boundary. */
  PM.Capabilities.compile = () => ({
    ok: true,
    commands: [
      { type: 'set_property', target: other.id, path: 'opacity', value: 40, mode: 'static', preserveHandEdits: false },
      { type: 'set_property', target: layer.id, path: 'opacity', value: 60, mode: 'static', preserveHandEdits: false },
    ],
  });

  const result = PM.Edit.apply([{ type: 'transform_layers', transform: { version: 1 } }]);
  assert.equal(result.ok, false);
  assert.match(result.message, /Layer “Locked title” is locked/);
  assert.equal(PM.L(layer.id).p.opacity.v, lockedBefore);
  assert.equal(PM.L(other.id).p.opacity.v, otherBefore, 'the earlier unlocked edit is rolled back atomically');
});

test('overrideLock true permits a property edit on a locked layer', () => {
  const { PM, layer } = fixture();
  const result = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 333,
    mode: 'static', preserveHandEdits: false, overrideLock: true,
  }]);
  assert.equal(result.ok, true);
  assert.equal(layer.p['position.x'].v, 333);
});

test('agent edits preserve the exact human-edited channel but not its sibling', () => {
  const { PM, layer } = fixture();
  layer.lock = false;
  const human = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 100,
    mode: 'static', preserveHandEdits: false, markIntent: 'human',
  }], { origin: 'interface' });
  assert.equal(human.ok, true);

  const blocked = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 200, mode: 'static',
  }], { origin: 'agent' });
  assert.equal(blocked.ok, false);
  assert.equal(blocked.message, 'Preserved hand-edited position.x; explicitly allow overwrite to change it');

  const explicitOverwrite = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 250,
    mode: 'static', preserveHandEdits: false,
  }], { origin: 'agent' });
  assert.equal(explicitOverwrite.ok, true);
  assert.equal(PM.L(layer.id).p['position.x'].v, 250);

  const sibling = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.y', value: 300, mode: 'static',
  }], { origin: 'agent' });
  assert.equal(sibling.ok, true);
  // A rejected apply restores from its snapshot, so reacquire the live layer.
  assert.equal(PM.L(layer.id).p['position.y'].v, 300);
});

test('a root locked_intent entry blocks a child property channel', () => {
  const { PM, layer } = fixture();
  layer.lock = false;
  layer.locked_intent.position = { by: 'human', at: 1, t: 0 };
  const result = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 444, mode: 'static',
  }], { origin: 'agent' });
  assert.equal(result.ok, false);
  assert.equal(result.message, 'Preserved hand-edited position.x; explicitly allow overwrite to change it');
});

test('agent, interface, and default origins are recorded as exact provenance strings', () => {
  const { PM, layer } = fixture();
  layer.lock = false;
  assert.equal(PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 10,
    mode: 'static', preserveHandEdits: false,
  }], { origin: 'agent' }).ok, true);
  assert.equal(PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.y', value: 20,
    mode: 'static', preserveHandEdits: false,
  }], { origin: 'interface' }).ok, true);
  assert.equal(PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'rotation', value: 30,
    mode: 'static', preserveHandEdits: false,
  }]).ok, true);
  assert.deepEqual([...PM.proj.edits.map((edit) => edit.origin)], ['agent', 'interface', 'interface']);
});
