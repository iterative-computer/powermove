// @ts-nocheck -- faithful behavioral transplant of the frozen JS oracle contract.
import assert from 'node:assert/strict';
import { it } from 'vitest';

import { makePM } from './make-pm';

const OPERATIONS = [
  'set_property', 'replace_keyframes', 'set_easing', 'set_expression',
  'set_content', 'set_layer', 'set_composition', 'add_layer',
  'delete_layers', 'reorder_layer', 'add_effect', 'remove_effect',
  'set_effect', 'set_scene_parameter', 'add_marker', 'create_section',
  'update_section', 'transform_layers',
];

const OPERATION_CONTRACT = {
  set_property: { target: 'layer', fields: ['path', 'value', 'time', 'mode', 'ease'] },
  replace_keyframes: { target: 'layer', fields: ['path', 'keyframes', 'replace', 'expression'] },
  set_easing: { target: 'keyframes', fields: ['keyframes', 'curve'] },
  set_expression: { target: 'layer', fields: ['path', 'expression'] },
  set_content: { target: 'layer', fields: ['patch'] },
  set_layer: { target: 'layer', fields: ['patch'] },
  set_composition: { target: 'project', fields: ['patch'] },
  add_layer: { target: 'project', fields: ['layerType', 'name', 'content', 'properties', 'parent', 'blend', 'motionBlur', 'visible', 'solo', 'shy', 'collapsed'] },
  delete_layers: { target: 'project', fields: ['targets'] },
  reorder_layer: { target: 'layer', fields: ['index'] },
  add_effect: { target: 'layer', fields: ['effect', 'parameters'] },
  remove_effect: { target: 'layer', fields: ['effect'] },
  set_effect: { target: 'layer', fields: ['effect', 'patch'] },
  set_scene_parameter: { target: 'project', fields: ['name', 'value'] },
  add_marker: { target: 'project', fields: ['time', 'name'] },
  create_section: { target: 'project', fields: ['section'] },
  update_section: { target: 'project', fields: ['sectionId', 'layers', 'thumb', 'version', 'at'] },
  transform_layers: { target: 'layer-collection', fields: ['transform', 'state'] },
};

function editor() {
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
  PM.proj = PM.mkProject({ name: 'Contract fixture', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 1;
  PM.syncShaderUniforms = () => {};
  return PM;
}

function fixture() {
  const PM = editor();
  const text = PM.mkLayer('text', { name: 'Title' });
  text.id = 'title';
  const solid = PM.mkLayer('solid', { name: 'Backdrop', d: { color: '#101010' } });
  solid.id = 'backdrop';
  const shape = PM.mkLayer('shape', { name: 'Badge' });
  shape.id = 'badge';
  PM.addLayer(text, 0);
  PM.addLayer(solid, 1);
  PM.addLayer(shape, 2);
  PM.selectLayers(text.id);
  PM.setKeyOn(text.p.opacity, 0, 0, 'linear', PM.proj.fps);
  PM.setKeyOn(text.p.opacity, 1, 100, 'linear', PM.proj.fps);
  return { PM, text, solid, shape };
}

function applyOne(PM, command, label = command.type) {
  const result = PM.Edit.apply([command], { label, origin: 'contract-test' });
  assert.equal(result.ok, true, `${command.type}: ${result.message}`);
  return result;
}

it('Edit.operations freezes the complete source-edit vocabulary', () => {
  const { PM } = fixture();
  assert.deepEqual(JSON.parse(JSON.stringify(PM.Edit.operations)), OPERATION_CONTRACT);
  assert.deepEqual(Object.keys(OPERATION_CONTRACT).sort(), [...OPERATIONS].sort());
});

it('all 18 source operations apply successfully and have an observable effect', () => {
  const { PM, text, solid } = fixture();

  applyOne(PM, {
    type: 'set_property', target: text.id, path: 'position.x', value: 111,
    mode: 'static', preserveHandEdits: false,
  });
  assert.equal(text.p['position.x'].v, 111);

  applyOne(PM, {
    type: 'replace_keyframes', target: text.id, path: 'opacity',
    keyframes: [{ time: .25, value: 20 }, { time: .75, value: 80 }],
    preserveHandEdits: false,
  });
  assert.deepEqual(Array.from(text.p.opacity.kf, key => ({ t: key.t, v: key.v })), [
    { t: 8 / 30, v: 20 },
    { t: 23 / 30, v: 80 },
  ]);
  assert.equal(text.p.opacity.kf.some(key => key.t === 0 || key.t === 1), false,
    'the previous keyframes are removed');

  const keyframeIds = text.p.opacity.kf.map((key) => key.i);
  text.p.opacity.kf.forEach(key => { key.hold = true; });
  applyOne(PM, { type: 'set_easing', keyframes: keyframeIds, curve: [.2, .1, .8, .9] });
  for (const key of text.p.opacity.kf) {
    assert.deepEqual([...key.eo, ...key.ei], [.2, .1, .8, .9]);
    assert.equal(key.hold, false);
  }

  applyOne(PM, { type: 'set_expression', target: text.id, path: 'opacity', expression: 'value + 1' });
  assert.equal(text.p.opacity.expr, 'value + 1');

  applyOne(PM, { type: 'set_content', target: text.id, patch: { text: 'Frozen contract' } });
  assert.equal(text.d.text, 'Frozen contract');

  applyOne(PM, { type: 'set_layer', target: text.id, patch: { name: 'Hero title' } });
  assert.equal(text.name, 'Hero title');

  applyOne(PM, { type: 'set_composition', patch: { name: 'Edited composition' } });
  assert.equal(PM.proj.name, 'Edited composition');

  applyOne(PM, {
    type: 'add_layer', id: 'added-solid', layerType: 'solid', name: 'Added solid',
    content: { color: '#223344' }, properties: { opacity: 75 }, select: false,
  });
  assert.equal(PM.L('added-solid').p.opacity.v, 75);

  applyOne(PM, { type: 'delete_layers', targets: ['added-solid'] });
  assert.equal(PM.L('added-solid'), null);

  applyOne(PM, { type: 'reorder_layer', target: solid.id, index: 0 });
  assert.equal(PM.proj.layers[0].id, solid.id);

  applyOne(PM, { type: 'add_effect', target: text.id, effect: 'blur', parameters: { amount: 12 } });
  const blur = text.fx[0];
  assert.deepEqual(Object.keys(blur.p), Array.from(PM.FX.blur.params, parameter => parameter.k));
  assert.equal(blur.p.amount.v, 12);

  applyOne(PM, { type: 'remove_effect', target: text.id, effect: blur.id });
  assert.equal(text.fx.length, 0);

  const toggleEffect = PM.mkEffect('blur');
  text.fx.push(toggleEffect);
  applyOne(PM, { type: 'set_effect', target: text.id, effect: toggleEffect.id, patch: { enabled: false } });
  assert.deepEqual(Object.keys(toggleEffect.p), Array.from(PM.FX.blur.params, parameter => parameter.k));
  assert.equal(toggleEffect.on, false);

  const parameter = PM.proj.params.Intensity = {
    name: 'Intensity', label: 'Intensity', control: 'num', value: 10,
  };
  applyOne(PM, { type: 'set_scene_parameter', name: 'Intensity', value: 42 });
  assert.equal(parameter.value, 42);

  applyOne(PM, { type: 'add_marker', id: 'marker-contract', time: 2.5, name: 'Beat' });
  assert.deepEqual(JSON.parse(JSON.stringify(PM.proj.markers.at(-1))), {
    id: 'marker-contract', t: 2.5, name: 'Beat',
  });

  PM.proj.library = {
    sections: Array.from({ length: 23 }, (_, index) => ({ id: `old-section-${index}` })),
    looks: [],
  };
  const firstSection = {
    id: 'section-contract-first', name: 'First contract section',
    layers: [JSON.parse(JSON.stringify(text))], versions: [], tags: ['text'], schemaVersion: 1,
  };
  const secondSection = {
    ...firstSection,
    id: 'section-contract-second', name: 'Second contract section',
  };
  applyOne(PM, { type: 'create_section', section: firstSection });
  applyOne(PM, { type: 'create_section', section: secondSection });
  assert.deepEqual(Array.from(PM.proj.library.sections.slice(0, 2), section => section.id), [
    secondSection.id, firstSection.id,
  ]);
  assert.equal(PM.proj.library.sections.length, 24);
  assert.equal(PM.proj.library.sections.some(section => section.id === 'old-section-22'), false);

  const replacement = JSON.parse(JSON.stringify(text));
  replacement.name = 'Section update';
  applyOne(PM, {
    type: 'update_section', sectionId: firstSection.id, layers: [replacement],
    version: { id: 'section-version', layers: [replacement], at: 1 },
  });
  assert.equal(PM.proj.library.sections.find(section => section.id === firstSection.id).layers[0].name, 'Section update');

  applyOne(PM, {
    type: 'transform_layers',
    transform: {
      version: 1, selector: { scope: 'all' },
      edits: [{ path: 'properties.opacity', value: 50 }],
    },
  });
  assert.equal(solid.p.opacity.v, 50);
});

it('scene parameter edits preserve the existing parameter object identity', () => {
  const { PM } = fixture();
  const parameter = PM.proj.params.Speed = {
    name: 'Speed', label: 'Playback speed', control: 'num', value: 1,
  };
  const before = PM.proj.params.Speed;
  const result = PM.Edit.apply([
    { type: 'set_scene_parameter', name: 'Speed', value: 2 },
  ]);
  assert.equal(result.ok, true);
  assert.equal(PM.proj.params.Speed, before);
  assert.equal(PM.proj.params.Speed, parameter);
  assert.equal(parameter.value, 2);
});

it('baseRevision mismatch rejects the edit with the project-changed contract', () => {
  const { PM, text } = fixture();
  const before = text.p['position.x'].v;
  const result = PM.Edit.apply([{
    type: 'set_property', target: text.id, path: 'position.x', value: before + 1,
    mode: 'static', preserveHandEdits: false,
  }], { baseRevision: PM.proj.revision + 1 });
  assert.equal(result.ok, false);
  assert.match(result.message, /Project changed/);
  assert.equal(text.p['position.x'].v, before);
});

it('a five-dispatch live edit coalesces to one edit and one revision', () => {
  const { PM, text } = fixture();
  const revision = PM.proj.revision;
  const editCount = PM.proj.edits.length;
  PM.Edit.begin('Drag position', { origin: 'interface' });
  for (const value of [100, 120, 140, 160, 180]) {
    const result = PM.Edit.dispatch({
      type: 'set_property', target: text.id, path: 'position.x', value,
      mode: 'static', preserveHandEdits: false,
    });
    assert.equal(result.ok, true);
  }
  const committed = PM.Edit.commit();
  assert.equal(committed.ok, true);
  assert.equal(PM.proj.revision, revision + 1);
  assert.equal(PM.proj.edits.length, editCount + 1);
  assert.equal(PM.proj.edits.at(-1).operations.length, 1);
  assert.equal(PM.proj.edits.at(-1).operations[0].value, 180);
});

it('cancelling an empty live edit preserves the project object reference', () => {
  const { PM } = fixture();
  const project = PM.proj;
  PM.Edit.begin('Empty gesture', { origin: 'interface' });
  assert.equal(PM.Edit.cancel(), true);
  assert.equal(PM.proj, project);
});

it('the source-edit provenance log is capped at 200 entries', () => {
  const { PM, text } = fixture();
  for (let index = 0; index < 205; index++) {
    const result = PM.Edit.apply([{
      type: 'set_property', target: text.id, path: 'position.x', value: index,
      mode: 'static', preserveHandEdits: false,
    }]);
    assert.equal(result.ok, true);
    assert.ok(PM.proj.edits.length <= 200);
  }
  assert.equal(PM.proj.edits.length, 200);
  assert.equal(PM.proj.edits.at(-1).revision, 205);
});

