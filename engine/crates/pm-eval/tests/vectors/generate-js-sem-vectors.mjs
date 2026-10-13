/**
 * Generates tests/vectors/js-sem-vectors.json from V8: run it with Node,
 *
 *   node engine/crates/pm-eval/tests/vectors/generate-js-sem-vectors.mjs
 *
 * `tests/js_sem_v8.rs` replays every case against `pm_eval::js_sem`. The
 * operators are JavaScript's own (`+`, `==`, `<`, `%`, `Math.round`,
 * `parseInt`, `toString(radix)`), applied to every pair of a value set that
 * covers the coercions projects depend on. Numbers are written as the hex of
 * their IEEE-754 bits. Deterministic.
 */
import { writeFileSync } from 'node:fs';

const OUT = process.env.PM_JS_SEM_OUT ?? new URL('./js-sem-vectors.json', import.meta.url).pathname;

let seed = 0x13579bdf;
function rand() {
  seed |= 0; seed = (seed + 0x6d2b79f5) | 0;
  let t = Math.imul(seed ^ (seed >>> 15), 1 | seed);
  t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
  return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
}

const view = new DataView(new ArrayBuffer(8));
const hex = x => { view.setFloat64(0, x); return view.getBigUint64(0).toString(16).padStart(16, '0'); };
function enc(v) {
  if (v === undefined) return { u: 1 };
  if (v === null) return { z: 1 };
  if (typeof v === 'number') return { n: hex(v) };
  if (typeof v === 'string') return { s: v };
  if (typeof v === 'boolean') return { b: v };
  return { j: JSON.stringify(v) };
}

const VALUES = [
  undefined, null, true, false, 0, -0, 1, -1, 0.1, 0.5, -2.5, 3, 7, 1e21, 1e-7, 123456789.125, NaN, Infinity, -Infinity,
  '', ' ', '5', ' 7 ', '-3.5', 'abc', '1e3', '0x10', '0b11', 'Infinity', '-Infinity', '+5', '.5', '5.', 'NaN', 'true', '\n12\t',
  [], [5], [1, 2], [null], ['7'], [[3]], {}, { a: 1 },
];

const binary = [];
for (const a of VALUES) for (const b of VALUES) {
  binary.push({
    a: enc(a), b: enc(b),
    add: enc(a + b), sub: enc(a - b), mul: enc(a * b), div: enc(a / b),
    // Objects are distinct here (fresh copies), as two JSON values are.
    eq: a == (b && typeof b === 'object' ? JSON.parse(JSON.stringify(b)) : b),
    seq: a === (b && typeof b === 'object' ? JSON.parse(JSON.stringify(b)) : b),
    lt: a < b, le: a <= b, gt: a > b, ge: a >= b,
  });
}
const unary = VALUES.map(v => ({ v: enc(v), number: enc(Number(v)), string: String(v), truthy: !!v, type: typeof v }));

const NUMS = [0, -0, 0.5, -0.5, 1.5, -1.5, 2.5, -2.5, 0.49999999999999994, -0.49999999999999994, 4503599627370495.5,
  4503599627370497, 9007199254740993, -4503599627370495.5, 1e300, -1e-300, 5e-324, 0.1, 254.5, 255.49999, NaN, Infinity, -Infinity];
for (let i = 0; i < 400; i++) NUMS.push((rand() - 0.5) * 10 ** Math.floor(rand() * 12 - 4));
for (let i = 0; i < 100; i++) NUMS.push(Math.floor((rand() - 0.5) * 2000) / 2);
const round = NUMS.map(x => [hex(x), hex(Math.round(x))]);
const rem = [];
const REM = [0, -0, 1, -1, 3, -3, 7.5, -7.5, 0.1, 1e300, NaN, Infinity, -Infinity, 5e-324, 6, -6];
for (const a of REM) for (const b of REM) rem.push([hex(a), hex(b), hex(a % b)]);
for (let i = 0; i < 300; i++) { const a = (rand() - 0.5) * 1000, b = (rand() - 0.5) * 20; rem.push([hex(a), hex(b), hex(a % b)]); }

const radix = [];
const RADIX_VALUES = [0, -0, 1, 15, 16, 255, -255, 256, 0.5, 0.1, -0.1, 1 / 3, 255.5, 1e21, 1.5e-7, 2 ** 53, 2 ** 53 + 2, 123.456, NaN, Infinity, -Infinity];
for (let i = 0; i < 200; i++) RADIX_VALUES.push((rand() - 0.5) * 10 ** Math.floor(rand() * 10 - 3));
for (const x of RADIX_VALUES) for (const r of [2, 8, 16, 36, 7]) radix.push([hex(x), r, x.toString(r)]);

const parse = [];
const PARSE = ['ff', 'FF', '0x1f', '0X1F', ' 7f ', '-a', '+b', 'g', '', '12px', '1e3', '0', '00ff', 'zz', '-0', '7fffffffffffffff', 'ffffffffffffffffffff', '  -0x10'];
for (const s of PARSE) for (const r of [0, 10, 16, 2, 36, 1, 37]) parse.push([s, r, hex(parseInt(s, r))]);

const pad = [['7', 2], ['ff', 2], ['NaN', 2], ['', 2], ['abc', 5]].map(([s, n]) => [s, n, s.padStart(n, '0')]);

writeFileSync(OUT, JSON.stringify({ engine: `node ${process.version}`, binary, unary, round, rem, radix, parse, pad }));
console.log(`wrote ${OUT}: ${binary.length} pairs`);
