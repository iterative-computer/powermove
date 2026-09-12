// @ts-nocheck -- faithful behavioral transplant of unique selection-identity oracle cases.
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
  );
  const events = [];
  PM.bus = {
    on() {},
    off() {},
    emit(name) { events.push(name); },
  };
  PM.proj = PM.mkProject({ name: 'Selection ids', dur: 10, fps: 30 });
  PM.time = 1;
  PM.rasterClears = 0;
  PM.rasterClear = () => { PM.rasterClears++; };
  PM.syncShaderUniforms = () => {};
  return { PM, events };
}

function keyedLayer(PM) {
  const layer = PM.mkLayer('text', { name: 'Title' });
  layer.id = 'title';
  PM.addLayer(layer, 0);
  const first = PM.setKeyOn(layer.p.opacity, 0, 0, 'linear', PM.proj.fps);
  const second = PM.setKeyOn(layer.p.opacity, 1, 100, 'linear', PM.proj.fps);
  PM.sel.layers = [layer.id];
  PM.sel.keys = [first.i, second.i];
  return { layer, first, second };
}

it('keyframe ids survive undo and redo prunes ids missing from the restored graph', () => {
  const { PM } = runtime();
  const { layer, first, second } = keyedLayer(PM);

  PM.hist.do('Remove selected key', () => {
    layer.p.opacity.kf = layer.p.opacity.kf.filter(key => key.i !== second.i);
    PM.touch();
  });
  assert.equal(PM.hist.undo(), true);
  assert.deepEqual([...PM.sel.keys], [first.i, second.i]);
  assert.deepEqual([...PM.resolveSelectedKeys().map(key => key.i)], [first.i, second.i]);
  assert.equal(PM.hist.redo(), true);
  assert.deepEqual([...PM.sel.keys], [first.i]);
});

it('set_easing resolves an id-based selection to live keyframe objects', () => {
  const { PM } = runtime();
  const { first, second } = keyedLayer(PM);
  const curve = [.2, .1, .8, .9];
  const result = PM.Edit.apply({ type: 'set_easing', keyframes: PM.sel.keys, curve }, { label: 'Ease ids' });

  assert.equal(result.ok, true);
  assert.deepEqual([...first.eo, ...first.ei], curve);
  assert.deepEqual([...second.eo, ...second.ei], curve);
  assert.deepEqual([...PM.Capabilities.selectedKeyframes().map(key => key.i)], PM.sel.keys);
});

it('expression cache survives touches, caps at 256, and evaluates after 300 distinct sources', () => {
  const { PM } = runtime();
  const { layer } = keyedLayer(PM);
  for (let index = 0; index < 300; index++) {
    layer.p.opacity.expr = `value + ${index}`;
    assert.equal(PM.ev(layer, 'opacity', 0), index);
    PM.touch();
  }
  assert.equal(PM.exprCache.size, 256);
  assert.equal(PM.exprCache.has('value + 0'), false);
  assert.equal(PM.exprCache.has('value + 299'), true);
  assert.equal(PM.ev(layer, 'opacity', 0), 299);
});

