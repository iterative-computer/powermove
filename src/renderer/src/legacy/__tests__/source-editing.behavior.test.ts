// @ts-nocheck -- faithful behavioral transplant of unique source-editing oracle cases.
import assert from 'node:assert/strict';
import { it } from 'vitest';

import { makePM } from './make-pm';

function editor() {
  const PM = makePM(
    'core/easing',
    'core/model',
    'core/selection',
    'core/anim',
    'core/history',
    'core/editing',
    'gl/shaders',
  );
  PM.proj = PM.mkProject({ name: 'Test', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 1;
  PM.syncShaderUniforms = () => {};
  return { PM, events: [] };
}

function addText(PM, id = 'title') {
  const layer = PM.mkLayer('text', { name: 'Title' });
  layer.id = id;
  PM.addLayer(layer, 0);
  PM.selectLayers(layer.id);
  return layer;
}

it('canvas, agent, and generated UI origins use the same source command', () => {
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

it('audio source edits enforce the canonical nonvisual schema', () => {
  const { PM } = editor();
  const audio = PM.mkLayer('audio', { name: 'Track', d: { asset: 'asset-1' } });
  audio.id = 'audio-1';
  PM.addLayer(audio, 0);

  const changed = PM.Edit.apply({
    type: 'set_content', target: audio.id,
    patch: { trim: 1.25, gain: 9, fadeIn: .1, fadeOut: .2 },
  }, { origin: 'inspector' });
  assert.equal(changed.ok, true);
  assert.deepEqual(JSON.parse(JSON.stringify(PM.L(audio.id).d)), {
    asset: 'asset-1', trim: 1.25, gain: 4, fadeIn: .1, fadeOut: .2,
  });

  assert.equal(PM.Edit.apply({ type: 'set_content', target: audio.id, patch: { width: 500 } }).ok, false);
  assert.equal(PM.Edit.apply({ type: 'set_layer', target: audio.id, patch: { motionBlur: true } }).ok, false);
  assert.equal(PM.Edit.apply({ type: 'set_layer', target: audio.id, patch: { parent: 'something' } }).ok, false);
  const paths = new Set(PM.Edit.sourceCatalog().layers.find(layer => layer.id === audio.id).controls.map(control => control.path));
  for (const path of ['layer.motionBlur', 'layer.blend', 'layer.parent']) assert.equal(paths.has(path), false);
  for (const path of ['content.asset', 'content.trim', 'content.gain', 'content.fadeIn', 'content.fadeOut']) assert.equal(paths.has(path), true);
});

it('procedural scripts can add parented styled layers atomically and undo them', () => {
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

it('an interrupted source gesture restores its edits and the next canvas gesture still works', () => {
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

it('Composition background color commits through source history and is undoable', () => {
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

it('the live editable-source catalog includes complete composition and selected-layer controls', () => {
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

it('Composition gradients preserve editable stops and undo as one source edit', () => {
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

it('Composition clear fill remains editable and undoable', () => {
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

it('generated visual easing applies to real keyframes as one undoable source edit', () => {
  const { PM } = editor();
  const layer = addText(PM);
  const property = layer.p.opacity;
  const first = PM.setKeyOn(property, 0, 0, 'linear', PM.proj.fps);
  const second = PM.setKeyOn(property, 1, 100, 'linear', PM.proj.fps);
  PM.sel.keys = [first.i, second.i];
  const before = first.eo.join(',') + '|' + first.ei.join(',');
  const curve = [.62, .05, 0, 1];

  const result = PM.Edit.apply({
    type: 'set_easing', keyframes: PM.sel.keys, curve,
  }, { label: 'Apply easing', origin: 'generated-tool' });

  assert.equal(result.ok, true);
  assert.deepEqual([...first.eo, ...first.ei], curve);
  assert.deepEqual([...second.eo, ...second.ei], curve);
  assert.equal(PM.proj.edits.at(-1).operations[0].type, 'set_easing');
  assert.equal(PM.hist.undo(), true);
  const restored = PM.L(layer.id).p.opacity.kf[0];
  assert.equal(restored.eo.join(',') + '|' + restored.ei.join(','), before);
});

it('effect addition is one source transaction and one undo step', () => {
  const { PM } = editor();
  const layer = addText(PM);

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

it('section creation and updates cross the same undoable source transaction boundary', () => {
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

it('a failed multi-command edit rolls the entire source back', () => {
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

