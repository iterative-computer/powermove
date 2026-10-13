// Runs the parity cases and the Math sweep through pm-expr compiled to wasm32
// (examples/wasm_parity.rs) and compares them with the V8 results a parity
// run wrote. Plain JavaScript, so it runs in bun and in Electron as Node:
//
//   wasm.mjs <wasm_parity.wasm> <parity dir> [--js-pow]
//
// <parity dir> holds cases.jsonl, expected.jsonl, math.json, and math-*.bin.
// With --js-pow the module was built with the `js-pow` feature and imports
// this runtime's Math.pow; under V8 every result must then match, pow
// included. Without it, pow mismatches are reported but allowed (the
// correctly rounded fallback differs from macOS's pow on about 0.09% of
// inputs).

import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { MATH_FUNCTIONS, mulberry32, sample } from './sweep.mjs';

const args = process.argv.slice(2);
const jsPow = args.includes('--js-pow');
const [wasmPath, dir] = args.filter((a) => !a.startsWith('--'));
if (!wasmPath || !dir) throw new Error('usage: wasm.mjs <wasm> <parity dir> [--js-pow]');

const imports = jsPow ? { pm_expr: { pow: Math.pow } } : {};
const { instance } = await WebAssembly.instantiate(readFileSync(wasmPath), imports);
const wasm = instance.exports;

const canonical = (v) => {
  if (Array.isArray(v)) return `[${v.map(canonical).join(',')}]`;
  if (v && typeof v === 'object') return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canonical(v[k])}`).join(',')}}`;
  return JSON.stringify(v);
};

const usesPow = (source) => /\*\*|pow/.test(source);

let failures = 0;

// Cases.
const casesBytes = readFileSync(join(dir, 'cases.jsonl'));
// Take the pointer first: allocating may grow (and detach) the memory buffer.
const inputPtr = wasm.input(casesBytes.length);
new Uint8Array(wasm.memory.buffer, inputPtr, casesBytes.length).set(casesBytes);
const length = wasm.run_cases();
const outputPtr = wasm.output();
const got = new TextDecoder().decode(new Uint8Array(wasm.memory.buffer, outputPtr, length)).split('\n');
const expected = readFileSync(join(dir, 'expected.jsonl'), 'utf8').split('\n').filter(Boolean);
const sources = readFileSync(join(dir, 'cases.jsonl'), 'utf8').split('\n').slice(1).filter(Boolean).map((l) => JSON.parse(l).source);
let caseFailures = 0;
let powCases = 0;
for (let i = 0; i < expected.length; i++) {
  if (canonical(JSON.parse(got[i])) === canonical(JSON.parse(expected[i]))) continue;
  if (!jsPow && usesPow(sources[i])) { powCases += 1; continue; }
  if (caseFailures < 20) console.log(`wasm case ${i} ${JSON.stringify(sources[i])}: expected ${expected[i]}\n  got ${got[i]}`);
  caseFailures += 1;
}
console.log(`wasm: ${expected.length - caseFailures - powCases} of ${expected.length} cases match V8` +
  (powCases ? ` (${powCases} more differ only through pow)` : ''));
failures += caseFailures;

// Math sweep.
const meta = JSON.parse(readFileSync(join(dir, 'math.json'), 'utf8'));
for (const [index, fn] of MATH_FUNCTIONS.entries()) {
  const bytes = readFileSync(join(dir, `math-${fn}.bin`));
  const v8 = new Float64Array(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
  const next = mulberry32(meta.seed ^ index);
  let bad = 0;
  for (let i = 0; i < meta.count; i++) {
    const [a, b] = sample(fn, i, next);
    const r = wasm.math(index, a, b);
    if (!Object.is(r, v8[i]) && !(Number.isNaN(r) && Number.isNaN(v8[i]))) {
      if (bad < 3) console.log(`wasm ${fn}(${a}, ${b}): V8 ${v8[i]}, wasm ${r}`);
      bad += 1;
    }
  }
  console.log(`wasm math: ${fn}: ${meta.count - bad} of ${meta.count} samples identical`);
  if (fn !== 'pow' || jsPow) failures += bad;
}
if (failures) {
  console.log(`wasm: ${failures} mismatches`);
  process.exit(1);
}
