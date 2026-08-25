const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const source = name => fs.readFileSync(path.join(root, name), 'utf8');

function runtime() {
  let id = 0;
  const events = [];
  const PM = {
    uid: prefix => `${prefix}${++id}`,
    clamp: (value, min, max) => Math.max(min, Math.min(max, value)),
    round: (value, places = 0) => Number(Number(value).toFixed(places)),
    snapF: (time, fps) => Math.round(time * fps) / fps,
    Ease: {
      PRESETS: {}, handles: () => ({ eo: [.33, 0], ei: [.67, 1] }),
      bezier: () => value => value, spring: value => value, nameOf: () => 'power',
    },
    SHADER_TEMPLATE: 'void main(){}', FX: {}, GL: { dropProgram() {} },
    bus: { emit: name => events.push(name) }, invalidate() {}, toast() {},
    store: { get: (_key, fallback) => fallback, set() {} },
  };
  const context = vm.createContext({ window: { PM }, console, Date, JSON, Object, Set, Map, Math });
  for (const file of ['js/core/model.js', 'js/core/selection.js', 'js/core/anim.js']) {
    vm.runInContext(source(file), context, { filename: file });
  }
  PM.proj = PM.mkProject({ name: 'Selection ids', dur: 10, fps: 30 });
  PM.time = 1;
  PM.rasterClears = 0;
  PM.rasterClear = () => { PM.rasterClears++; };
  PM.syncShaderUniforms = () => {};
  PM.mkEffect = () => null;
  vm.runInContext(source('js/core/history.js'), context, { filename: 'js/core/history.js' });
  vm.runInContext(source('js/core/editing.js'), context, { filename: 'js/core/editing.js' });
  vm.runInContext(source('js/core/capabilities.js'), context, { filename: 'js/core/capabilities.js' });
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

test('keyframe ids survive undo and redo prunes ids missing from the restored graph', () => {
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

test('set_easing resolves an id-based selection to live keyframe objects', () => {
  const { PM } = runtime();
  const { first, second } = keyedLayer(PM);
  const curve = [.2, .1, .8, .9];
  const result = PM.Edit.apply({ type: 'set_easing', keyframes: PM.sel.keys, curve }, { label: 'Ease ids' });

  assert.equal(result.ok, true);
  assert.deepEqual([...first.eo, ...first.ei], curve);
  assert.deepEqual([...second.eo, ...second.ei], curve);
  assert.deepEqual([...PM.Capabilities.selectedKeyframes().map(key => key.i)], PM.sel.keys);
});

test('legacy object selections degrade to empty ids and replacement clamps and invalidates in legacy event order', () => {
  const { PM, events } = runtime();
  const { layer, first } = keyedLayer(PM);
  PM.time = 9;
  PM.exprCache.set('value + 1', {});
  events.length = 0;
  const generation = PM.projGeneration;
  const shorter = JSON.parse(JSON.stringify(PM.proj));
  shorter.dur = 2;

  PM.replaceProject(shorter, { selection: { layers: [layer.id, 'dead'], keys: [first, 'dead'], chan: 'opacity' } });

  assert.deepEqual([...PM.sel.layers], [layer.id]);
  assert.deepEqual([...PM.sel.keys], []);
  assert.equal(PM.time, 2);
  assert.equal(PM.exprCache.size, 0);
  assert.equal(PM.rasterClears, 1);
  assert.equal(PM.projGeneration, generation + 1);
  assert.deepEqual(events, ['layers', 'sel', 'assets', 'project']);
});

test('expression cache survives touches, caps at 256, and evaluates after 300 distinct sources', () => {
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
