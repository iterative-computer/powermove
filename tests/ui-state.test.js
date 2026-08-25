const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = (name) => fs.readFileSync(path.join(root, name), 'utf8');

function fixture() {
  let id = 0;
  const listeners = new Map();
  const PM = {
    version: 'test',
    uid: prefix => `${prefix}-${++id}`,
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    bus: {
      on(name, fn) {
        if (!listeners.has(name)) listeners.set(name, new Set());
        listeners.get(name).add(fn);
      },
      emit(name) {
        for (const fn of listeners.get(name) || []) fn();
      },
    },
    invalidate() {},
    toast() {},
    touch() {},
  };
  const context = vm.createContext({ window: { PM }, console, JSON, Object, Map, Set });
  vm.runInContext(source('js/core/ui-state.js'), context, { filename: 'js/core/ui-state.js' });
  vm.runInContext(source('js/core/model.js'), context, { filename: 'js/core/model.js' });
  PM.proj = PM.mkProject({ name: 'UI state fixture', w: 1920, h: 1080, fps: 30, dur: 5 });
  const layer = PM.mkLayer('text', { name: 'Title' });
  layer.id = 'layer-a';
  layer.collapsed = false;
  PM.addLayer(layer, 0);
  const prop = layer.p.opacity;
  const key = { i: 'key-a', t: 0, v: 100, eo: [.33, 0], ei: [.67, 1] };
  prop.kf.push(key);
  PM.UIState.prune(PM.proj);
  vm.runInContext(source('js/core/history.js'), context, { filename: 'js/core/history.js' });
  return { PM, key, layer };
}

test('graph handle interaction does not mutate project JSON or create a phantom undo entry', () => {
  const { PM, key } = fixture();
  PM.hist.clear();
  const before = JSON.stringify(PM.proj);
  const serializedBefore = PM.serialize();
  const canUndoBefore = PM.hist.canUndo();

  PM.hist.begin('Click keyframe');
  PM.UIState.setKeyHandles(key, { ho: [120, 45], hi: [80, 45], pt: [100, 60] });
  PM.hist.commit('Click keyframe');

  assert.equal(JSON.stringify(PM.proj), before);
  assert.equal(PM.serialize(), serializedBefore);
  assert.equal(PM.hist.canUndo(), canUndoBefore);
  assert.deepEqual(JSON.parse(JSON.stringify(PM.UIState.getKeyHandles(key))), {
    ho: [120, 45], hi: [80, 45], pt: [100, 60],
  });
});

test('side tables are pruned when the active project changes', () => {
  const { PM, key, layer } = fixture();
  const effect = { id: 'fx-prune', type: 'blur', on: true, p: {} };
  layer.fx.push(effect);
  PM.UIState.setKeyHandles(key, { pt: [10, 20] });
  PM.UIState.setReveal(layer, ['opacity']);
  PM.UIState.setShaderMeta(layer, { shaderKey: 'shader:old' });
  PM.UIState.setFxOpen(effect, true);
  assert.equal(PM.UIState.getLayerCollapsed(layer), false, 'collapsed defaults from the persistent layer field');
  PM.UIState.setLayerCollapsed(layer, true);

  const next = PM.mkProject({ name: 'Next project', w: 1280, h: 720, fps: 24, dur: 3 });
  PM.proj = next;
  PM.bus.emit('project');

  assert.equal(PM.UIState.keyHandles.size, 0);
  assert.equal(PM.UIState.reveal.size, 0);
  assert.equal(PM.UIState.shaderMeta.size, 0);
  assert.equal(PM.UIState.fxOpen.size, 0);
  assert.equal(PM.UIState.layerCollapsed.size, 0);
});

test('shader metadata round-trips through the layer side table', () => {
  const { PM, layer } = fixture();
  const udefs = [{ name: 'amount', label: 'Amount', control: 'slider', min: 0, max: 1 }];
  PM.UIState.setShaderMeta(layer, { udefs, shaderKey: 'sh:layer-a:123' });

  const meta = PM.UIState.getShaderMeta(layer);
  assert.equal(meta.shaderKey, 'sh:layer-a:123');
  assert.deepEqual(JSON.parse(JSON.stringify(meta.udefs)), udefs);
  assert.equal(JSON.stringify(PM.proj).includes('_shaderKey'), false);
  assert.equal(JSON.stringify(PM.proj).includes('_udefs'), false);
});

test('legacy transient writers are redirected to non-enumerable side-table accessors', () => {
  const { PM, layer } = fixture();
  const effect = { id: 'fx-a', type: 'blur', on: true, open: true, p: {} };
  layer.fx.push(effect);
  PM.UIState.prune(PM.proj);

  layer._reveal = ['position.x'];
  layer._shaderKey = 'sh:compat';
  layer._udefs = [{ name: 'gain' }];
  effect.open = false;

  assert.deepEqual(Array.from(PM.UIState.getReveal(layer)), ['position.x']);
  assert.equal(PM.UIState.getShaderMeta(layer).shaderKey, 'sh:compat');
  assert.equal(PM.UIState.getShaderMeta(layer).udefs[0].name, 'gain');
  assert.equal(PM.UIState.getFxOpen(effect), false);
  assert.equal(JSON.stringify(PM.proj).includes('_reveal'), false);
  assert.equal(JSON.stringify(PM.proj).includes('_shaderKey'), false);
  assert.equal(JSON.stringify(PM.proj).includes('_udefs'), false);
  assert.equal(JSON.stringify(PM.proj).includes('"open"'), false);

  layer.collapsed = true;
  PM.UIState.prune(PM.proj);
  assert.equal(PM.UIState.getLayerCollapsed(layer), true);
  layer.collapsed = false;
  layer._reveal = ['opacity'];
  assert.equal(PM.UIState.getLayerCollapsed(layer), false, 'legacy reveal shortcuts still expand the side-table view');
});
