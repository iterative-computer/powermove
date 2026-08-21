const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');

function animModel() {
  let nextId = 0;
  const PM = {
    clamp: (v, a, b) => Math.max(a, Math.min(b, v)),
    uid: (p) => `${p}${++nextId}`,
    exprCache: new Map(),
    bus: { emit() {} },
    L: (id) => PM.proj.layers.find(l => l.id === id) || null,
    curComp: () => PM.proj,
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/easing.js'), 'utf8'), context);
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/anim.js'), 'utf8'), context);
  return PM;
}

test('parenting cycles no longer freeze transforms after a depth cap', () => {
  const PM = animModel();
  const layer = (id, parent) => ({
    id, parent,
    p: { position: { x: 10, y: 0 } },
  });
  const L = (id, parent) => ({
    id, parent, from: 0,
    p: {
      'anchor.x': { v: 0, kf: [] }, 'anchor.y': { v: 0, kf: [] },
      'position.x': { v: 100, kf: [] }, 'position.y': { v: 0, kf: [] },
      'scale.x': { v: 100, kf: [] }, 'scale.y': { v: 100, kf: [] },
      rotation: { v: 0, kf: [] }, skew: { v: 0, kf: [] },
      opacity: { v: 100, kf: [] },
    },
  });
  const a = L('La', null), b = L('Lb', 'La'), c = L('Lc', 'Lb');
  PM.proj = { layers: [a, b, c], params: {}, fps: 30 };
  // a → c closes the loop
  a.parent = 'Lc';
  const m = PM.worldMatrix(a, 0);
  assert.ok(m.every(Number.isFinite), 'matrix stays finite in a cycle');
  assert.equal(PM.worldOpacity(a, 0), 1);
});

test('wouldCycle detects self, ancestors and unrelated layers', () => {
  const PM = animModel();
  const mk = (id, parent) => ({ id, parent });
  PM.proj = { layers: [mk('a', null), mk('b', 'a'), mk('c', 'b'), mk('z', null)] };
  assert.equal(PM.wouldCycle(PM.proj.layers[0], 'a'), true, 'self');
  assert.equal(PM.wouldCycle(PM.proj.layers[0], 'c'), true, 'parenting to own descendant cycles');
  assert.equal(PM.wouldCycle(PM.proj.layers[0], 'z'), false, 'unrelated is fine');
  assert.equal(PM.wouldCycle(PM.proj.layers[2], 'b'), false, 'keeping existing parent is fine');
  assert.equal(PM.wouldCycle(PM.proj.layers[0], null), false);
});

test('evalKfs tolerates unsorted input by clamping to endpoints rather than NaN', () => {
  const PM = animModel();
  const kf = [
    { t: .8, v: 30, eo: [0, 0], ei: [1, 1] },
    { t: .2, v: 10, eo: [0, 0], ei: [1, 1] },
  ];
  for (const t of [-1, 0, .3, .5, 1.5]) {
    const v = PM.evalKfs(kf, t);
    assert.ok(Number.isFinite(v), `finite at ${t}`);
  }
});

function projectModel() {
  let nextId = 0;
  const PM = {
    clamp: (v, a, b) => Math.max(a, Math.min(b, v)),
    uid: (p) => `${p}${++nextId}`,
    exprCache: new Map(),
    bus: { emit() {} },
    scope: [],
    invalidate: () => {},
    selectLayers: () => {},
    Ease: null,
    version: 'test',
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/easing.js'), 'utf8'), context);
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/anim.js'), 'utf8'), context);
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/model.js'), 'utf8'), context);
  return PM;
}

function baseProject(PM) {
  const p = PM.mkProject({ name: 'T', w: 1920, h: 1080, fps: 30, dur: 10 });
  PM.proj = p;
  return p;
}

test('precompose collapses selected layers into a nested comp with an identical span', () => {
  const PM = projectModel();
  const p = baseProject(PM);
  const a = PM.mkLayer('shape', { name: 'A' }, p); a.from = 1; a.dur = 3;
  const b = PM.mkLayer('text', { name: 'B' }, p); b.from = 2; b.dur = 3;
  const keep = PM.mkLayer('solid', { name: 'Keep' }, p);
  p.layers.push(a, b, keep);

  const L = PM.precompose([a.id, b.id], 'Group');
  assert.ok(L, 'precomp layer created');
  assert.equal(L.type, 'precomp');
  assert.ok(p.comps[L.d.comp], 'nested comp registered');
  const sub = p.comps[L.d.comp];
  assert.deepEqual([...sub.layers.map(l => l.name)], ['A', 'B'], 'layers moved in stack order');
  assert.deepEqual([...p.layers.map(l => l.name)], ['Group', 'Keep']);
  assert.equal(L.from, 1, 'span starts at earliest layer');
  assert.equal(Math.abs(L.dur - 4) < 1e-9, true, 'span ends at latest layer');
  assert.ok(PM.compOf(L) === sub, 'compOf resolves the nested comp');
});

test('precompose breaks parenting across the comp boundary', () => {
  const PM = projectModel();
  const p = baseProject(PM);
  const inner = PM.mkLayer('shape', { name: 'Inner' }, p);
  const outer = PM.mkLayer('null', { name: 'Outer' }, p);
  inner.parent = outer.id;          // parent outside the future group
  p.layers.push(outer, inner);
  const L = PM.precompose([inner.id], 'Group');
  const sub = p.comps[L.d.comp];
  assert.equal(sub.layers[0].parent, null, 'inbound parenting severed');
  assert.equal(outer.parent, null);
});

test('deleting a precomp layer garbage-collects its composition', () => {
  const PM = projectModel();
  const p = baseProject(PM);
  const a = PM.mkLayer('shape', { name: 'A' }, p);
  p.layers.push(a);
  const L = PM.precompose([a.id], 'Group');
  const cid = L.d.comp;
  assert.ok(p.comps[cid]);
  PM.removeLayers([L.id]);
  assert.ok(!p.comps[cid], 'orphaned comp removed');
});
