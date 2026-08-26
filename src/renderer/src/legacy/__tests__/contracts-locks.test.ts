// @ts-nocheck -- faithful behavioral transplant of the frozen lock-policy oracle.
// Documents the locked-layer policy matrix (Phase 3a).
import assert from 'node:assert/strict';
import { it } from 'vitest';

import { makePM } from './make-pm';

function fixture() {
  const PM = makePM(
    'core/easing',
    'core/model',
    'core/selection',
    'core/anim',
    'core/history',
    'core/editing',
    'core/capabilities',
    'gl/shaders',
  );
  PM.proj = PM.mkProject({ name: 'Lock fixture', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 1;
  PM.syncShaderUniforms = () => {};

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
  set_composition: () => ({ type: 'set_composition', patch: { name: 'Allowed composition edit' } }),
  add_layer: () => ({ type: 'add_layer', id: 'allowed-layer', layerType: 'solid', name: 'Allowed layer', select: false }),
  set_scene_parameter: () => ({ type: 'set_scene_parameter', name: 'Allowed parameter', value: 42 }),
  add_marker: () => ({ type: 'add_marker', id: 'allowed-marker', time: 2, name: 'Allowed marker' }),
  create_section: ({ layer }) => ({
    type: 'create_section',
    section: { id: 'allowed-section', layers: [JSON.parse(JSON.stringify(layer))], versions: [] },
  }),
  update_section: ({ PM, layer }) => {
    PM.proj.library = { sections: [{ id: 'existing-section', layers: [], versions: [] }], looks: [] };
    return { type: 'update_section', sectionId: 'existing-section', layers: [JSON.parse(JSON.stringify(layer))] };
  },
  transform_layers: ({ PM, layer }) => {
    PM.Capabilities.compile = () => ({
      ok: true,
      commands: [{
        type: 'set_property', target: layer.id, path: 'opacity', value: 60,
        mode: 'static', preserveHandEdits: false,
      }],
    });
    return { type: 'transform_layers', transform: { version: 1 } };
  },
};

it('the Phase 3a locked-layer operation matrix is frozen for all 18 operations', () => {
  const outcomes = {
    set_property: 'blocked',
    replace_keyframes: 'blocked',
    set_easing: 'blocked',
    set_expression: 'blocked',
    set_content: 'blocked',
    set_layer: 'blocked',
    set_composition: 'allowed',
    add_layer: 'allowed',
    delete_layers: 'blocked',
    reorder_layer: 'blocked',
    add_effect: 'blocked',
    remove_effect: 'blocked',
    set_effect: 'blocked',
    set_scene_parameter: 'allowed',
    add_marker: 'allowed',
    create_section: 'allowed',
    update_section: 'allowed',
    transform_layers: 'blocked',
  };
  assert.deepEqual(Object.keys(outcomes).sort(), Object.keys(fixture().PM.Edit.operations).sort());

  for (const [operation, expected] of Object.entries(outcomes)) {
    const state = fixture();
    const result = state.PM.Edit.apply([lockedCases[operation](state)]);
    if (expected === 'blocked') {
      assert.equal(result.ok, false, `${operation} unexpectedly succeeded`);
      assert.match(result.message, /locked/i, operation);
      assert.match(result.message, /Locked title/, operation);
    } else {
      assert.equal(result.ok, true, `${operation}: ${result.message}`);
    }
  }
});

it('transform_layers rolls back unlocked edits when a compiled property edit reaches a locked layer', () => {
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

it('set_layer allows the explicit locked:false unlock for HUMAN origins only', () => {
  const { PM, layer } = fixture();
  /* Agent unlock is blocked — otherwise a two-command batch [unlock, edit]
     defeats the entire lock matrix. */
  const agentUnlock = PM.Edit.apply([{
    type: 'set_layer', target: layer.id, patch: { locked: false },
  }], { origin: 'agent' });
  assert.equal(agentUnlock.ok, false);
  assert.equal(PM.L(layer.id).lock, true);

  const humanUnlock = PM.Edit.apply([{
    type: 'set_layer', target: layer.id, patch: { locked: false },
  }], { origin: 'interface' });
  assert.equal(humanUnlock.ok, true, humanUnlock.message);
  assert.equal(PM.L(layer.id).lock, false);

  PM.L(layer.id).lock = true;
  const combined = PM.Edit.apply([{
    type: 'set_layer', target: layer.id, patch: { locked: false, name: 'Also rename' },
  }], { origin: 'interface' });
  assert.equal(combined.ok, false);
  assert.equal(combined.message, 'Layer “Locked title” is locked');
  assert.equal(PM.L(layer.id).lock, true);
  assert.equal(PM.L(layer.id).name, 'Locked title');
});

it('an agent batch cannot unlock-then-edit a locked layer', () => {
  const { PM, layer } = fixture();
  const batch = PM.Edit.apply([
    { type: 'set_layer', target: layer.id, patch: { locked: false } },
    { type: 'set_property', target: layer.id, path: 'opacity', value: 5, mode: 'static' },
    { type: 'delete_layers', targets: [layer.id] },
  ], { origin: 'agent' });
  assert.equal(batch.ok, false);
  assert.equal(PM.L(layer.id).lock, true, 'the layer stays locked');
  assert.notEqual(PM.L(layer.id).p.opacity.v, 5, 'the property edit did not land');
  assert.ok(PM.L(layer.id), 'the layer was not deleted');
});

it('the live-transaction path enforces the same lock policy', () => {
  const { PM, layer } = fixture();
  PM.Edit.begin('Agent drag', { origin: 'agent' });
  const blocked = PM.Edit.dispatch({ type: 'set_property', target: layer.id, path: 'opacity', value: 9, mode: 'static' });
  PM.Edit.cancel();
  assert.equal(blocked.ok, false);
  assert.match(blocked.message, /locked/);
});

it('overrideLock requires an explicitly trusted human origin', () => {
  const { PM, layer } = fixture();
  /* Unset origin fails closed even though provenance defaults to interface. */
  const unset = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'opacity', value: 7, mode: 'static', overrideLock: true,
  }]);
  assert.equal(unset.ok, false);
  /* canvas and timeline are real human surfaces and keep the privilege. */
  for (const origin of ['canvas', 'timeline', 'interface', 'inspector']) {
    const state = fixture();
    const allowed = state.PM.Edit.apply([{
      type: 'set_property', target: state.layer.id, path: 'opacity', value: 7, mode: 'static', overrideLock: true,
    }], { origin });
    assert.equal(allowed.ok, true, `${origin}: ${allowed.message}`);
    assert.equal(state.PM.L(state.layer.id).p.opacity.v, 7);
  }
});

it('delete_layers skips locked targets, lists them, and fails when all targets are locked', () => {
  const { PM, layer, other } = fixture();
  const partial = PM.Edit.apply([{
    type: 'delete_layers', targets: [layer.id, other.id],
  }], { origin: 'agent' });
  assert.equal(partial.ok, true, partial.message);
  assert.match(partial.message, /Skipped locked layers: “Locked title”/);
  assert.ok(PM.L(layer.id));
  assert.equal(PM.L(other.id), null);
  assert.deepEqual(partial.data.results[0].data.skippedLocked, [layer.id]);

  const allLocked = PM.Edit.apply([{
    type: 'delete_layers', targets: [layer.id],
  }], { origin: 'agent' });
  assert.equal(allLocked.ok, false);
  assert.equal(allLocked.message, 'All targeted layers are locked: “Locked title”');
  assert.ok(PM.L(layer.id));
});

it('overrideLock is honored only for interface and inspector origins', () => {
  for (const origin of ['interface', 'inspector']) {
    const { PM, layer } = fixture();
    const result = PM.Edit.apply([{
      type: 'set_property', target: layer.id, path: 'position.x', value: 333,
      mode: 'static', preserveHandEdits: false, overrideLock: true,
    }], { origin });
    assert.equal(result.ok, true, `${origin}: ${result.message}`);
    assert.equal(layer.p['position.x'].v, 333);
    assert.equal(PM.proj.edits.at(-1).operations[0].overrideLock, true);
  }

  for (const origin of ['agent', 'generated-ui', 'generated-tool', 'generated-script']) {
    const { PM, layer, other } = fixture();
    const blocked = PM.Edit.apply([{
      type: 'set_property', target: layer.id, path: 'position.x', value: 333,
      mode: 'static', preserveHandEdits: false, overrideLock: true,
    }], { origin });
    assert.equal(blocked.ok, false, `${origin} unexpectedly bypassed the lock`);
    assert.equal(blocked.message, 'Layer “Locked title” is locked');

    const sanitized = PM.Edit.apply([{
      type: 'set_property', target: other.id, path: 'position.x', value: 222,
      mode: 'static', preserveHandEdits: false, overrideLock: true,
    }], { origin });
    assert.equal(sanitized.ok, true, `${origin}: ${sanitized.message}`);
    const recorded = PM.proj.edits.at(-1).operations[0];
    assert.equal(Object.hasOwn(recorded, 'overrideLock'), false);
    assert.equal(recorded.preserveHandEdits, false, 'provenance keeps the submitted overwrite request');
    assert.equal(sanitized.data.results[0].command.preserveHandEdits, true, 'dispatch uses the forced policy');
  }
});

it('agent edits preserve the exact human-edited channel but not its sibling', () => {
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

  const rejectedOverride = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 250,
    mode: 'static', preserveHandEdits: false,
  }], { origin: 'agent' });
  assert.equal(rejectedOverride.ok, false);
  assert.equal(rejectedOverride.message, 'Preserved hand-edited position.x; explicitly allow overwrite to change it');

  const sibling = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.y', value: 300, mode: 'static',
  }], { origin: 'agent' });
  assert.equal(sibling.ok, true);
  // A rejected apply restores from its snapshot, so reacquire the live layer.
  assert.equal(PM.L(layer.id).p['position.y'].v, 300);
});

it('a generated-ui human gesture can overwrite locked_intent but cannot override layer.lock', () => {
  const unlockedState = fixture();
  unlockedState.layer.lock = false;
  unlockedState.layer.locked_intent['position.x'] = { by: 'human', at: 1, t: 0 };
  const overwrite = unlockedState.PM.Edit.apply([{
    type: 'set_property', target: unlockedState.layer.id, path: 'position.x', value: 500,
    mode: 'static', preserveHandEdits: false, markIntent: 'human', overrideLock: true,
  }], { origin: 'generated-ui' });
  assert.equal(overwrite.ok, true, overwrite.message);
  assert.equal(unlockedState.layer.p['position.x'].v, 500);
  const recorded = unlockedState.PM.proj.edits.at(-1).operations[0];
  assert.equal(recorded.preserveHandEdits, false);
  assert.equal(recorded.markIntent, 'human');
  assert.equal(Object.hasOwn(recorded, 'overrideLock'), false);

  const lockedState = fixture();
  const blocked = lockedState.PM.Edit.apply([{
    type: 'set_property', target: lockedState.layer.id, path: 'position.x', value: 500,
    mode: 'static', preserveHandEdits: false, markIntent: 'human', overrideLock: true,
  }], { origin: 'generated-ui' });
  assert.equal(blocked.ok, false);
  assert.equal(blocked.message, 'Layer “Locked title” is locked');
});

it('a root locked_intent entry blocks a child property channel', () => {
  const { PM, layer } = fixture();
  layer.lock = false;
  layer.locked_intent.position = { by: 'human', at: 1, t: 0 };
  const result = PM.Edit.apply([{
    type: 'set_property', target: layer.id, path: 'position.x', value: 444, mode: 'static',
  }], { origin: 'agent' });
  assert.equal(result.ok, false);
  assert.equal(result.message, 'Preserved hand-edited position.x; explicitly allow overwrite to change it');
});

it('agent, interface, and default origins are recorded as exact provenance strings', () => {
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

