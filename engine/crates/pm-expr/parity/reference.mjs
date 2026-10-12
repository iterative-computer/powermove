// Reference side of the pm-expr parity harness. Plain JavaScript so that it
// runs unchanged in V8 (Electron as Node, the runtime Powermove ships) and in
// JavaScriptCore (bun):
//
//   reference.mjs <expression.mjs> cases <cases.jsonl> <expected.jsonl>
//   reference.mjs <expression.mjs> math <out-dir> <count> <seed>
//
// `expression.mjs` is legacy/core/expression.ts bundled by run.ts. The sweep's
// inputs come from sweep.mjs.

import { readFileSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

import { MATH_FUNCTIONS, mulberry32, sample } from './sweep.mjs';

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
