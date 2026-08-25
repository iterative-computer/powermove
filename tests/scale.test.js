const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const root = path.resolve(__dirname, '..');
const baselinePath = path.join(root, 'tests/fixtures/perf-baseline.json');
const CALIBRATION_ITERATIONS = 2_000_000;
const calibrationValues = Float64Array.from({ length: 1024 }, (_value, index) => (index % 97) + .25);

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

function scaleScene(PM, expression = null) {
  const mkL = (i) => {
    const p = {};
    for (const k of ['position.x', 'position.y', 'scale.x', 'scale.y', 'rotation', 'opacity']) {
      p[k] = {
        v: i,
        kf: Array.from({ length: 60 }, (_, j) => ({ t: j * .1, v: Math.sin(i + j) * 100, eo: [.5, 0], ei: [.5, 1], i: 'k' + j })),
        expr: k === 'position.x' ? expression : null,
      };
    }
    return { id: 'L' + i, parent: i ? 'L' + (i - 1) : null, from: 0, dur: 1e9, p };
  };
  PM.proj = { layers: Array.from({ length: 1000 }, (_, i) => mkL(i)), params: {}, fps: 30 };
}

function evaluateSecond(PM) {
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
  return { acc, ms: performance.now() - t0 };
}

function medianBenchmark(PM) {
  for (let i = 0; i < 3; i++) assert.ok(Number.isFinite(evaluateSecond(PM).acc));
  const samples = Array.from({ length: 7 }, () => evaluateSecond(PM).ms).sort((a, b) => a - b);
  return samples[3];
}

function calibrateOnce() {
  const t0 = performance.now();
  let acc = 0;
  for (let index = 0; index < CALIBRATION_ITERATIONS; index++) {
    acc += calibrationValues[index & 1023] * 1.000001 + (index & 7);
  }
  return { acc, ms: performance.now() - t0 };
}

function calibrationMedian() {
  for (let index = 0; index < 2; index++) assert.ok(Number.isFinite(calibrateOnce().acc));
  const samples = Array.from({ length: 5 }, () => calibrateOnce().ms).sort((a, b) => a - b);
  return samples[2];
}

test('evaluation stays interactive-scale with 1000 layers × 60 keys', () => {
  const PM = animModel();
  scaleScene(PM);
  const { acc, ms } = evaluateSecond(PM);
  assert.ok(Number.isFinite(acc));
  assert.ok(ms < 4000, `full-project evaluation took ${ms.toFixed(0)}ms (budget 4s)`);
});

test('evaluation medians stay within the recorded legacy performance envelope', () => {
  const plain = animModel();
  scaleScene(plain);
  const evalMedianMs = medianBenchmark(plain);

  const expressionHeavy = animModel();
  scaleScene(expressionHeavy, 'value + Math.sin(t) * 10');
  const exprMedianMs = medianBenchmark(expressionHeavy);
  const calibrationMs = calibrationMedian();
  const evalRatio = evalMedianMs / calibrationMs;
  const exprRatio = exprMedianMs / calibrationMs;
  const current = {
    calibrationMs,
    evalMedianMs,
    exprMedianMs,
    evalRatio,
    exprRatio,
    recordedAt: new Date().toISOString(),
    node: process.version,
  };

  if (process.env.PM_RECORD_BASELINE === '1') {
    fs.mkdirSync(path.dirname(baselinePath), { recursive: true });
    fs.writeFileSync(baselinePath, `${JSON.stringify(current, null, 2)}\n`);
    return;
  }

  assert.ok(fs.existsSync(baselinePath),
    `Performance baseline is missing at ${baselinePath}; run PM_RECORD_BASELINE=1 npm test to record it intentionally`);
  const baseline = JSON.parse(fs.readFileSync(baselinePath, 'utf8'));
  for (const field of ['calibrationMs', 'evalMedianMs', 'exprMedianMs', 'evalRatio', 'exprRatio']) {
    assert.ok(Number.isFinite(baseline[field]) && baseline[field] > 0,
      `Performance baseline field ${field} must be a positive finite number; run PM_RECORD_BASELINE=1 npm test to re-record it`);
  }
  assert.ok(evalRatio <= baseline.evalRatio * 1.5,
    `plain evaluation ratio ${evalRatio.toFixed(2)} exceeded baseline ${baseline.evalRatio.toFixed(2)} × 1.5 ` +
    `(raw ${evalMedianMs.toFixed(2)}ms / calibration ${calibrationMs.toFixed(2)}ms)`);
  assert.ok(exprRatio <= baseline.exprRatio * 1.5,
    `expression evaluation ratio ${exprRatio.toFixed(2)} exceeded baseline ${baseline.exprRatio.toFixed(2)} × 1.5 ` +
    `(raw ${exprMedianMs.toFixed(2)}ms / calibration ${calibrationMs.toFixed(2)}ms)`);
});
