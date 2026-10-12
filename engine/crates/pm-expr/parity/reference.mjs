// Reference side of the pm-expr parity harness. Plain JavaScript so that it
// runs unchanged in V8 (Electron as Node, the runtime Powermove ships) and in
// JavaScriptCore (bun):
//
//   reference.mjs <expression.mjs> cases <cases.jsonl> <expected.jsonl>
//   reference.mjs <expression.mjs> math <out-dir> <count> <seed>
//
// `expression.mjs` is legacy/core/expression.ts bundled by run.ts. The input
// generator below is mirrored exactly by tests/parity.rs.

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const [modulePath, mode, ...rest] = process.argv.slice(2);
const { compileExpression, expressionDiagnostic } = await import(pathToFileURL(modulePath).href);

const view = new DataView(new ArrayBuffer(8));
const bitsHex = (x) => {
  view.setFloat64(0, x);
  return view.getBigUint64(0).toString(16).padStart(16, '0');
};

function encode(value) {
  switch (typeof value) {
    case 'undefined': return { type: 'undefined' };
    case 'boolean': return { type: 'boolean', value };
    case 'number': return { type: 'number', bits: Number.isNaN(value) ? 'nan' : bitsHex(value) };
    case 'string': return { type: 'string', value };
    default: {
      if (value === null) return { type: 'null' };
      let string = null;
      let error = null;
      try { string = String(value); } catch (e) { error = e instanceof Error ? e.message : String(e); }
      return { type: typeof value === 'function' ? 'function' : 'object', string, error };
    }
  }
}

function buildContext(spec) {
  const comp = spec.comp;
  const layer = spec.layerIndex === undefined ? spec.layer : comp.layers[spec.layerIndex];
  // paramGet in anim.ts, reading the context composition.
  const param = (name) => {
    const p = comp.params && comp.params[name];
    return p ? p.value : 0;
  };
  return { t: spec.t, T: spec.T, fps: spec.fps, value: spec.value, layer, comp, param, ch: spec.ch, idx: spec.idx };
}

function runCases(casesPath, outPath) {
  const lines = readFileSync(casesPath, 'utf8').split('\n').filter(Boolean);
  const header = JSON.parse(lines[0]);
  const contexts = header.contexts.map(buildContext);
  const out = [];
  for (const line of lines.slice(1)) {
    const { source, ctx } = JSON.parse(line);
    const expected = { diagnostic: expressionDiagnostic(source) };
    const compiled = compileExpression(source);
    expected.compiled = compiled !== null;
    if (compiled) {
      expected.needsIdx = compiled.needsIdx;
      const context = { ...contexts[ctx] };
      if (!compiled.needsIdx) context.idx = 0;
      try {
        const r = compiled.evaluate(context);
        expected.result = encode(r);
        expected.channel = (typeof r === 'number' && isFinite(r)) || typeof r === 'string' || typeof r === 'boolean'
          ? { value: encode(r), error: null }
          : { value: null, error: 'Expression did not return a finite value.' };
      } catch (e) {
        const message = e instanceof Error ? e.message : 'Expression failed.';
        expected.result = { type: 'throw', message };
        expected.channel = { value: null, error: message };
      }
    } else {
      expected.channel = { value: null, error: 'Expression syntax is invalid.' };
    }
    out.push(JSON.stringify(expected));
  }
  writeFileSync(outPath, out.join('\n') + '\n');
}

// ── math sweep ─────────────────────────────────────────────────────────────

export const MATH_FUNCTIONS = ['sin', 'cos', 'tan', 'asin', 'acos', 'atan', 'atan2', 'exp', 'log', 'pow', 'sqrt', 'random', 'wiggle'];

function mulberry32(seed) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return (t ^ (t >>> 14)) >>> 0;
  };
}

const fromBits = (hi, lo) => {
  view.setUint32(0, hi);
  view.setUint32(4, lo);
  return view.getFloat64(0);
};

const RANGE_A = { sin: 1e3, cos: 1e3, tan: 1e3, asin: 1, acos: 1, atan: 1e3, atan2: 1e3, exp: 745, log: 1e3, pow: 20, sqrt: 1e3, random: 1e3, wiggle: 1e3 };
const RANGE_B = { atan2: 1e3, pow: 40 };

/** The inputs of sample i of function fn; six draws per sample. */
export function sample(fn, i, next) {
  const d = [next(), next(), next(), next(), next(), next()];
  const unit = (k) => d[k] / 4294967296;
  const mode = i % 5;
  let a;
  if (mode === 0) a = fromBits(d[0], d[1]);
  else if (mode === 1) a = (unit(0) * 2 - 1) * RANGE_A[fn];
  else if (mode === 2) a = (Math.floor(unit(0) * 2001) - 1000 + Math.floor(unit(1) * 100) / 100) * 127.1;
  else if (mode === 3) a = unit(0) * 60 * (unit(1) * 8) * 6.2831 + unit(2) * 17.3 + Math.floor(unit(3) * 3) * 2.4;
  else a = (unit(0) * 2 - 1) * fromBits(((Math.floor(unit(1) * 80) - 60 + 1023) << 20) >>> 0, 0);
  let b;
  if (mode === 0) b = fromBits(d[2], d[3]);
  else if (mode === 2 && fn === 'pow') b = Math.floor((unit(4) * 2 - 1) * 40);
  else b = (unit(4) * 2 - 1) * (RANGE_B[fn] ?? 1);
  if (fn === 'log') a = Math.abs(a);
  if ((fn === 'asin' || fn === 'acos') && mode !== 0) a = unit(0) * 2 - 1;
  if (fn === 'pow' && mode === 1) a = unit(0) * 20;
  if (fn === 'wiggle') { a = unit(0) * 60; b = unit(1) * 8; }
  return [a, b];
}

function runMath(outDir, count, seed) {
  const random = compileExpression('random(value)');
  const wiggle = compileExpression('wiggle(value, 20, 7)');
  const base = { T: 0, fps: 30, layer: {}, comp: {}, param: () => 0, ch: 'x', idx: 0 };
  const apply = {
    sin: Math.sin, cos: Math.cos, tan: Math.tan, asin: Math.asin, acos: Math.acos, atan: Math.atan,
    atan2: Math.atan2, exp: Math.exp, log: Math.log, pow: Math.pow, sqrt: Math.sqrt,
    random: (a) => random.evaluate({ ...base, t: 0, value: a }),
    wiggle: (a, b) => wiggle.evaluate({ ...base, t: a, value: b }),
  };
  for (const fn of MATH_FUNCTIONS) {
    const next = mulberry32(seed ^ MATH_FUNCTIONS.indexOf(fn));
    const out = new Float64Array(count);
    for (let i = 0; i < count; i++) {
      const [a, b] = sample(fn, i, next);
      out[i] = apply[fn](a, b);
    }
    writeFileSync(join(outDir, `math-${fn}.bin`), new Uint8Array(out.buffer));
  }
  writeFileSync(join(outDir, 'math.json'), JSON.stringify({ count, seed, functions: MATH_FUNCTIONS, littleEndian: true }));
}

if (mode === 'cases') runCases(rest[0], rest[1]);
else if (mode === 'math') runMath(rest[0], Number(rest[1]), Number(rest[2]));
else throw new Error(`unknown mode ${mode}`);
