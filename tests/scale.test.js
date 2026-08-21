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
    scope: [],
    L: (id) => PM.proj.layers.find(l => l.id === id) || null,
    curComp: () => PM.proj,
  };
  const context = vm.createContext({ window: { PM }, console });
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/easing.js'), 'utf8'), context);
  vm.runInContext(fs.readFileSync(path.join(root, 'js/core/anim.js'), 'utf8'), context);
  return PM;
}

test('evaluation stays interactive-scale with 1000 layers × 60 keys', () => {
  const PM = animModel();
  const mkL = (i) => {
    const p = {};
    for (const k of ['position.x', 'position.y', 'scale.x', 'scale.y', 'rotation', 'opacity']) {
      p[k] = { v: i, kf: Array.from({ length: 60 }, (_, j) => ({ t: j * .1, v: Math.sin(i + j) * 100, eo: [.5, 0], ei: [.5, 1], i: 'k' + j })) };
    }
    return { id: 'L' + i, parent: i ? 'L' + (i - 1) : null, from: 0, dur: 1e9, p };
  };
  PM.proj = { layers: Array.from({ length: 1000 }, (_, i) => mkL(i)), params: {}, fps: 30 };

  const t0 = performance.now();
  let acc = 0;
  for (let f = 0; f < 30; f++) {
    const T = f / 30;
    PM.beginEval(T); // the render contract: one eval window per frame
    for (let i = 0; i < 1000; i++) {
      acc += PM.ev(PM.proj.layers[i], 'position.x', T);
      acc += PM.worldOpacity(PM.proj.layers[i], T);
    }
    // deep chain: worldMatrix walks parents — sample every 10th layer to keep this linear-ish
    for (const L of PM.proj.layers.filter((_, i) => i % 10 === 0)) acc += PM.worldMatrix(L, T)[4];
  }
  const ms = performance.now() - t0;
  assert.ok(Number.isFinite(acc));
  assert.ok(ms < 4000, `full-project evaluation took ${ms.toFixed(0)}ms (budget 4s)`);
});
