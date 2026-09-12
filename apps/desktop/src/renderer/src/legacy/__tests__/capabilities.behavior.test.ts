// @ts-nocheck -- faithful behavioral transplant of the surviving generated-tool oracle.
import assert from 'node:assert/strict';
import { it } from 'vitest';

import { makePM } from './make-pm';

function runtime() {
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
  PM.proj = PM.mkProject({ name: 'Transforms', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.time = 2;
  PM.syncShaderUniforms = () => {};
  return PM;
}

function addLayers(PM, starts = [1, 1, 1]) {
  const layers = starts.map((from, index) => {
    const layer = PM.mkLayer('text', { name: `Layer ${index + 1}`, from, dur: 4 });
    layer.id = `layer-${index + 1}`; layer.from = from; layer.dur = 4;
    PM.addLayer(layer, PM.proj.layers.length); return layer;
  });
  PM.selectLayers(layers.map(layer => layer.id));
  return layers;
}

function staggerAction(PM, mode = 'apply') {
  return PM.Capabilities.panelRecipe('layer-stagger').controls
    .find(control => control.action?.mode === mode).action;
}

it('Layer Stagger previews exact source changes without mutating the project', () => {
  const PM = runtime();
  const layers = addLayers(PM);
  const preview = PM.Capabilities.preview(staggerAction(PM, 'preview').transform, {
    offsetFrames: 2, order: 'stack', anchor: 'earliest',
  });
  assert.equal(preview.ok, true);
  assert.deepEqual(layers.map(layer => layer.from), [1, 1, 1], 'preview is read-only');
  assert.deepEqual([...preview.changes.map(change => Math.round(change.after * 30))], [32, 34]);
  assert.deepEqual([...preview.changes.map(change => change.target)], ['layer-2', 'layer-3']);
});

it('a collection transform applies as one source transaction and one undo step', () => {
  const PM = runtime();
  addLayers(PM);
  const action = staggerAction(PM, 'apply');
  const result = PM.Capabilities.apply(action.transform, {
    offsetFrames: 2, order: 'stack', anchor: 'earliest',
  }, { label: 'Apply Stagger' });
  assert.equal(result.ok, true);
  assert.deepEqual([...PM.proj.layers.map(layer => Math.round(layer.from * 30))], [30, 32, 34]);
  assert.equal(PM.proj.revision, 1);
  assert.equal(PM.hist.list().at(-1), 'Apply Stagger');
  assert.equal(PM.hist.undo(), true);
  assert.deepEqual([...PM.proj.layers.map(layer => Math.round(layer.from * 30))], [30, 30, 30]);
  assert.equal(PM.hist.redo(), true);
  assert.deepEqual([...PM.proj.layers.map(layer => Math.round(layer.from * 30))], [30, 32, 34]);
});

it('transform_layers is a higher-order PM.Edit command that expands to primitive source edits', () => {
  const PM = runtime();
  addLayers(PM);
  const action = staggerAction(PM, 'apply');
  const result = PM.Edit.apply({
    type: 'transform_layers', transform: action.transform,
    state: { offsetFrames: 3, order: 'reverseStack', anchor: 'playhead' },
  }, { label: 'Reverse stagger', origin: 'agent' });
  assert.equal(result.ok, true);
  assert.deepEqual([...PM.proj.layers.map(layer => Math.round(layer.from * 30))], [66, 63, 60]);
  assert.equal(PM.proj.edits[0].operations.every(operation => operation.type === 'set_layer'), true,
    'history records the real primitive edits, not an opaque tool program');
});

it('generated transforms skip locked layers and report live selection state', () => {
  const PM = runtime();
  const layers = addLayers(PM); layers[1].lock = true;
  const action = staggerAction(PM, 'preview');
  const preview = PM.Capabilities.preview(action.transform, {
    offsetFrames: 2, order: 'stack', anchor: 'earliest',
  });
  assert.equal(preview.targets.length, 2);
  assert.equal(PM.Capabilities.selectionSummary(), '2 of 3 layers editable');
  assert.deepEqual([...preview.changes.map(change => change.target)], ['layer-3']);
});

it('the transform language supports state, current values, aggregates, and frame math', () => {
  const PM = runtime();
  addLayers(PM, [1, 2, 3]);
  const transform = {
    label: 'Compress timing', selector: { scope: 'selection' }, order: 'stack', edits: [{
      path: 'layer.from', value: { op: 'add', args: [
        { aggregate: 'min', path: 'layer.from' },
        { op: 'multiply', args: [{ ref: 'index' }, { op: 'frames', args: [{ state: 'spacing' }] }] },
      ] },
    }, {
      path: 'layer.duration', value: { op: 'multiply', args: [{ ref: 'current' }, { state: 'durationScale' }] },
    }],
  };
  const result = PM.Capabilities.apply(transform, { spacing: 4, durationScale: .5 });
  assert.equal(result.ok, true);
  assert.deepEqual([...PM.proj.layers.map(layer => Math.round(layer.from * 30))], [30, 34, 38]);
  assert.deepEqual([...PM.proj.layers.map(layer => layer.dur)], [2, 2, 2]);
});

it('the reusable Layer Stagger recipe exposes selection, settings, preview, apply, and undo', () => {
  const PM = runtime();
  const panel = PM.Capabilities.panelRecipe('layer-stagger');
  assert.equal(panel.state.offsetFrames, 2);
  assert.deepEqual([...panel.controls.map(control => control.label)], [
    'Selection', 'Offset', 'Order', 'Anchor', 'Preview', 'Apply Stagger', 'Undo last edit',
  ]);
  assert.equal(panel.controls.find(control => control.label === 'Apply Stagger').primary, true);
  assert.equal(panel.controls.find(control => control.label === 'Apply Stagger').action.type, 'transform');
});

it('Flow-style generated tools get a visual curve and apply it through source history', () => {
  const PM = runtime();
  const [layer] = addLayers(PM, [0]);
  const first = PM.setKeyOn(layer.p.opacity, 0, 0, 'linear', PM.proj.fps);
  const second = PM.setKeyOn(layer.p.opacity, 1, 100, 'linear', PM.proj.fps);
  PM.sel.keys = [first.i, second.i];
  const panel = PM.Capabilities.panelRecipe('easing-flow');
  const curve = panel.controls.find(control => control.type === 'curve');
  const apply = panel.controls.find(control => control.action?.type === 'easing' && control.action.mode === 'apply');
  const original = [...first.eo, ...first.ei];

  assert.equal(curve.stateKey, 'curve');
  assert.ok(curve.presets.includes('easeInOut'));
  assert.equal(PM.Capabilities.keyframeSummary(), '2 keyframes selected');
  const preview = PM.Capabilities.previewEasing(apply.action, { curve: [.2, .8, .3, 1] });
  assert.equal(preview.ok, true);
  assert.deepEqual([...first.eo, ...first.ei], original, 'preview never mutates keyframes');
  const result = PM.Capabilities.applyEasing(apply.action, { curve: [.2, .8, .3, 1] });
  assert.equal(result.ok, true);
  assert.deepEqual([...first.eo, ...first.ei], [.2, .8, .3, 1]);
  assert.equal(PM.proj.edits.at(-1).origin, 'generated-tool');
  assert.equal(PM.hist.undo(), true);
  assert.deepEqual([...PM.L(layer.id).p.opacity.kf[0].eo, ...PM.L(layer.id).p.opacity.kf[0].ei], original);
});

it('Decompose Text is a reusable sandboxed tool rather than a panel of generic shortcuts', () => {
  const PM = runtime();
  const panel = PM.Capabilities.panelRecipe('decompose-text');
  assert.deepEqual([...panel.controls.map(control => control.label)], [
    'Selection', 'Split into', 'Original', 'Preview', 'Decompose', 'Undo last edit',
  ]);
  const action = panel.controls.find(control => control.label === 'Decompose').action;
  assert.equal(action.type, 'script');
  assert.equal(action.mode, 'apply');
  assert.deepEqual([...action.requiredTypes], ['text']);
  assert.match(action.code, /source\.textLayout/);
  assert.match(action.code, /PM\.addLayer/);
  assert.doesNotMatch(action.code, /document|window|fetch/);
  const sanitized = PM.Capabilities.sanitizeControlAction(action);
  assert.equal(sanitized.type, 'script');
  assert.equal(PM.Capabilities.sanitizeControlAction({ type: 'script', mode: 'apply', code: '' }), null);
});

