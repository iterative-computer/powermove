// Case generation for the pm-expr parity harness (run with bun).
//
// Sources: expressions found in the repository (tests, docs, fixtures), the
// private corpus when it is present (never written into the repository), a
// curated list of edge cases, a loose-equality and coercion matrix, and a
// seeded grammar fuzzer.

import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

export type ContextSpec = {
  t: number;
  T: number;
  fps: number;
  value?: unknown;
  layer?: unknown;
  layerIndex?: number;
  comp: Record<string, unknown>;
  ch: string;
  idx: number;
};

export type Case = { source: string; ctx: number; origin: string };

export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ── contexts ───────────────────────────────────────────────────────────────

const titleLayer = {
  id: 'l1',
  name: 'Title',
  type: 'text',
  from: 0.5,
  dur: 6,
  group: null,
  shy: false,
  p: {
    'position.x': { v: 320, kf: [{ t: 0, v: 100 }, { t: 2, v: 540 }] },
    opacity: { v: 1 },
    'c.color': { v: '#ff3366' },
    'c.text': { v: 'Hello 😀 world' },
    amount: 4,
  },
  tags: ['a', 'b', 'c'],
  nested: { list: [[1, 2], [3, [4, 5]], [], [null], [{}]], empty: {}, zero: 0, blank: '', no: false },
};

const mainComp = {
  id: 'c1',
  name: 'Main',
  w: 1920,
  h: 1080,
  fps: 30,
  dur: 8,
  layers: [
    { id: 'l0', name: 'Background', type: 'solid', from: 0, dur: 8, p: {} },
    titleLayer,
    { id: 'l2', name: '7', type: 'shape', from: -1, dur: 3, p: { x: { v: '12' } } },
  ],
  params: {
    gain: { value: 3 },
    spread: { value: 120 },
    label: { value: 'hi' },
    flag: { value: true },
    zero: { value: 0 },
    list: { value: [1, 2] },
    none: { value: null },
    falsy: 0,
    word: 'text',
    '': { value: 'empty name' },
    '0': { value: 'zero name' },
    'Unicode 😀': { value: 9 },
  },
  markers: [{ t: 1, name: 'beat' }],
  empty: [],
  single: [5],
  pair: [1, 2],
  nested: [[1, 2], [3]],
  text: ' 42 ',
};

export const CONTEXTS: ContextSpec[] = [
  // 0: the context of expression.test.ts.
  {
    t: 0.5, T: 2, fps: 30, value: 10,
    layer: { name: 'Shape', p: { amount: 4 } },
    comp: { dur: 8, layers: [{}, {}, {}], params: { gain: { value: 3 } } },
    ch: 'position.x', idx: 2,
  },
  // 1: a realistic composition; `layer` is comp.layers[1] (identity holds).
  { t: 1.25, T: 1.75, fps: 30, value: 320, layerIndex: 1, comp: mainComp, ch: 'position.x', idx: 1 },
  // 2: string value, negative local time, fractional rate.
  { t: -0.75, T: 0.25, fps: 29.97, value: '#ff3366', layerIndex: 1, comp: mainComp, ch: 'c.color', idx: 1 },
  // 3: array value and a large time.
  { t: 123456.789, T: 123457.289, fps: 60, value: [1, 2, 3], layerIndex: 2, comp: mainComp, ch: 'scale', idx: 2 },
  // 4: an own `toString` on the layer (string coercion throws), object value.
  {
    t: 3.5, T: 4, fps: 24, value: { x: 1, y: 2 },
    layer: { name: 'Odd', toString: 1, valueOf: 'v', p: { constructor: 5, __proto__x: 1 }, length: 3 },
    comp: { layers: [], params: [{ value: 'array params' }, 0] },
    ch: 'rotation', idx: 0,
  },
  // 5: no params, boolean value, zero times.
  { t: 0, T: 0, fps: 25, value: true, layer: { name: '', from: 0 }, comp: { name: 'Bare', layers: [] }, ch: '', idx: -1 },
  // 6: params is a string; value missing (undefined).
  { t: 7.25, T: 9, fps: 120, layer: { name: 'x', p: {} }, comp: { params: 'abc', layers: [{ id: 'q' }] }, ch: 'opacity', idx: 0 },
];

// ── repository harvest ─────────────────────────────────────────────────────

const SKIP = new Set(['node_modules', 'target', '.git', 'dist', 'out', '.claude', 'build']);
// The harness itself is not a source of project expressions.
const SKIP_PATHS = ['engine/crates/pm-expr'];

function walk(dir: string, files: string[], root: string): void {
  for (const name of readdirSync(dir)) {
    if (SKIP.has(name)) continue;
    const path = join(dir, name);
    if (SKIP_PATHS.includes(relative(root, path))) continue;
    const stat = statSync(path);
    if (stat.isDirectory()) walk(path, files, root);
    else if (/\.(ts|svelte|md|json|mjs|js)$/.test(name) && stat.size < 4_000_000) files.push(path);
  }
}

function unescapeLiteral(body: string, quote: string): string | null {
  if (quote === '"') {
    try { return JSON.parse(`"${body}"`); } catch { /* fall through */ }
  }
  let out = '';
  for (let i = 0; i < body.length; i++) {
    const c = body[i]!;
    if (c !== '\\') { out += c; continue; }
    const e = body[++i];
    if (e === undefined) return null;
    if (e === 'u') {
      const hex = body.slice(i + 1, i + 5);
      if (!/^[0-9a-fA-F]{4}$/.test(hex)) return null;
      out += String.fromCharCode(parseInt(hex, 16));
      i += 4;
    } else out += ({ n: '\n', r: '\r', t: '\t', b: '\b', f: '\f', v: '\v' } as Record<string, string>)[e] ?? e;
  }
  return out;
}

/** Expression-like string literals: values of `expr`/`expression` keys and
 * arguments of the expression entry points and test helpers. */
export function harvestRepository(root: string): Map<string, string> {
  const files: string[] = [];
  for (const top of ['apps', 'packages', 'docs', 'engine']) {
    try { walk(join(root, top), files, root); } catch { /* missing directory */ }
  }
  const found = new Map<string, string>();
  const pattern = /(?:\bexpr|\bexpression|compileExpression\(|expressionDiagnostic\(|\brun\()\s*['"]?\s*[:,(]?\s*(['"])((?:\\.|(?!\1)[^\\\n])*)\1/g;
  for (const file of files) {
    const text = readFileSync(file, 'utf8');
    if (!/expr|expression/.test(text)) continue;
    for (const m of text.matchAll(pattern)) {
      const value = unescapeLiteral(m[2]!, m[1]!);
      if (value === null || value.length > 2100) continue;
      if (!found.has(value)) found.set(value, relative(root, file));
    }
  }
  return found;
}

// ── corpus harvest (private; never written into the repository) ────────────

export function harvestCorpus(dir: string): { contexts: ContextSpec[]; sources: [string, number][] } {
  const contexts: ContextSpec[] = [];
  const sources: [string, number][] = [];
  const files: string[] = [];
  const visit = (d: string) => {
    for (const name of readdirSync(d)) {
      const p = join(d, name);
      if (statSync(p).isDirectory()) visit(p);
      else if (name === 'document.json') files.push(p);
    }
  };
  visit(dir);
  for (const file of files) {
    let doc: any;
    try { doc = JSON.parse(readFileSync(file, 'utf8')); } catch { continue; }
    const comps: any[] = [doc, ...(Array.isArray(doc.comps) ? doc.comps : [])];
    for (const comp of comps) {
      if (!Array.isArray(comp?.layers)) continue;
      comp.layers.forEach((layer: any, layerIndex: number) => {
        for (const [ch, prop] of Object.entries<any>(layer?.p ?? {})) {
          if (!prop || typeof prop.expr !== 'string' || !prop.expr) continue;
          const value = Array.isArray(prop.kf) && prop.kf.length ? prop.kf[0].v : prop.v;
          for (const T of [0, 0.5, 1.25, 2.5, 7.3]) {
            const ctx = contexts.length;
            const from = typeof layer.from === 'number' ? layer.from : 0;
            const fps = comp.fps || doc.fps;
            contexts.push({ t: T - from, T, fps, value, comp, layerIndex, ch, idx: layerIndex });
            sources.push([prop.expr, ctx]);
          }
        }
      });
    }
  }
  return { contexts, sources };
}

// ── curated edge cases ─────────────────────────────────────────────────────

export const EDGE_CASES: string[] = [
  // Limits.
  'value;' + 'x'.repeat(2_100),
  '1+'.repeat(999) + '1',
  '1+'.repeat(999) + '12',
  '1+'.repeat(80) + '1',
  '1+'.repeat(79) + '1',
  '('.repeat(12) + '1' + ')'.repeat(12),
  '('.repeat(13) + '1' + ')'.repeat(13),
  '-'.repeat(23) + '1',
  '-'.repeat(24) + '1',
  '-'.repeat(25) + '1',
  '('.repeat(2000),
  '-'.repeat(2000),
  '['.repeat(1000) + '1',
  'max(1,2,3,4,5,6,7,8,9,10,11,12)',
  'max(1,2,3,4,5,6,7,8,9,10,11,12,13)',
  'Math.max(1,2,3,4,5,6,7,8,9,10,11,12)',
  'clamp(1,2,3,4,5,6,7,8,9,10,11,12,13)',
  'a'.repeat(2000),
  'a'.repeat(2001),
  '😀'.repeat(1000),
  '"' + '😀'.repeat(999) + '"',
  '"' + 'é'.repeat(1998) + '"',
  // Syntax errors and their positions.
  '', ' ', '\n', '1 +', '+', '(1', '1)', '[1]', 'a.', 'a.1', 'a[1', 'f(1,', 'f(1 2)', '1 ? 2', '1 ? 2 :', '? 1 : 2',
  'value = 4', '(() => 7)()', '1 & 2', '1 | 2', '#', '`x`', '{}', '"abc', "'abc", '"a\nb"', '"a\\', '"\\', '1e999',
  '1e-999', '1.5.5', '1..toString', '.5', '5.', '1.e3', '0x10', '0b1', '1_000', '1e', '1e+', '1E5', '00012', '08',
  'a b', '1 2', 'Math .sin(1)', 'Math. sin(1)', 'Math.\nsin(1)', '😀', 'é', '"é" + t', 'x @ y', '1 = 2', '1 !== 2 === false',
  ' 1　+﻿2 ', '1​+2', '\t\v\f1',
  // Identifiers and names.
  'time', 't', 'T', 'fps', 'value', 'layer', 'comp', 'ch', 'idx', 'PI', 'Math', 'frame', 'undefined', 'NaN', 'Infinity',
  'this', 'globalThis', 'window', 'eval', 'Function', 'Object', 'constructor', 'toString', '__proto__', 'prototype',
  '$', '_', '$x', 'true', 'false', 'null', 'True', 'idx2', 'clamp', 'sin', 'param', 'Math.E', 'Math.LN2', 'Math.LN10',
  'Math.LOG2E', 'Math.LOG10E', 'Math.PI', 'Math.SQRT1_2', 'Math.SQRT2', 'Math.sin', 'Math.max', 'Math.random',
  'Math.constructor', 'Math.__proto__', 'Math.prototype', 'Math.toString', 'Math.hasOwnProperty', 'Math["PI"]',
  'Math["E"] + Math.E', 'Math[0]', 'Math[null]',
  // Calls.
  'sin(1)', 'Math.sin(1)', 'Math["sin"](1)', "Math['cos'](1)", 'Math[sin](1)', 'Math["s" + "in"](1)', '(Math).sin(1)',
  'Math.PI(1)', 'Math.random()', 'Math.floor(-1.5)', 'Math.ceil(-0.5)', 'Math.round(-0.5)', 'Math.round(0.49999999999999994)',
  'Math.round(-2.5)', 'Math.round(2.5)', 'Math.sign(-0)', 'Math.abs(-0)', 'Math.max()', 'Math.min()', 'max(-0, 0)', 'min(0, -0)',
  'Math.max(1, "3", true)', 'Math.min(1, NaN)', 'Math.pow(2, 0.5)', 'Math.pow(-8, 1/3)', 'Math.pow(1, Infinity)',
  'pow(2, 1/0)', 'pow(-1, -1/0)', 'pow(0, -1)', 'pow(-0, -3)', 'pow(-0, 0.5)', 'pow(-1/0, 0.5)', 'pow(-1/0, -0.5)',
  'pow(4, -0.5)', 'pow(2, -1074)', 'pow(2, 1023.9999)', 'pow(10, 308.3)', 'pow(10, -323.4)', 'pow(-2, 3)', 'pow(-2, 0.5)',
  '2 ** 3 ** 2', '(-2) ** 2', '2 ** -1', '(-8) ** (1/3)', 'atan2(1)', 'atan2(0, -0)', 'atan2(-0, -0)', 'atan2(1/0, 1/0)',
  'atan(1e-9)', 'atan(-1e-300)', 'atan(1e300)', 'atan(1/0)', 'asin(2)', 'acos(-1)', 'log(0)', 'log(-1)', 'exp(710)',
  'exp(-746)', 'sqrt(-0)', 'sqrt(-1)', 'sin(1e300)', 'cos(1e22)', 'tan(1.5707963267948966)', 'sin()', 'abs()', 'floor("2.7")',
  't(1)', '(1)(2)', 'layer.name(1)', 'value()', 'Math.sin.call(1)', 'f()()', 'clamp()', 'clamp(5)', 'clamp(5, 6)',
  'clamp(5, 6, 4)', 'clamp(NaN, 0, 1)', 'clamp(undefined, 0, 1)', 'lerp()', 'lerp(0, 10, 0.5)', 'linear()', 'linear(t)',
  'linear(t, 0, 0, 5, 6)', 'linear(t, 0, 0)', 'linear(t, 0, 1, 5)', 'ease(t, 0, 0, 1, 2)', 'ease()', 'ease(0.5, 0, 1, 0, 1)',
  'random()', 'random(0)', 'random(-1)', 'random(1e300)', 'random("7")', 'random(null)', 'random(undefined)',
  'wiggle()', 'wiggle(2)', 'wiggle(2, 120)', 'wiggle(2, 120, 3)', 'wiggle(0.5, 1, -4)', 'bounce()', 'bounce(0.2)',
  'bounce(0.5)', 'bounce(0.8)', 'bounce(0.95)', 'bounce(2)', 'bounce(NaN)', 'loop()', 'loop(0, 5)', 'loop(2, 5)',
  'loop(-1)', 'loop(2)', 'pingpong()', 'pingpong(2, 5)', 'pingpong(0, 5)', 'pingpong(-1)', 'pingpong(2, -1)',
  'param("gain")', 'param("label")', 'param("flag")', 'param("zero")', 'param("list")', 'param("none")',
  'param("falsy")', 'param("word")', 'param("")', 'param()', 'param(null)', 'param(0)', 'param("0")', 'param(["gain"])',
  'param("Unicode 😀")', 'param("constructor")', 'param("toString")', 'param("__proto__")', 'param("hasOwnProperty")',
  'param("missing")', 'param("length")', 'param("1")', 'param("2")', 'param("map")', 'param("charAt")', 'param(layer)',
  // Inherited Object.prototype lookups.
  'constructor(5) + 1', 'constructor()', 'constructor() + ""', 'constructor(5) == 5', 'constructor(5) === 5', '!constructor(0)',
  '"" + constructor(7)', 'toString()', 'valueOf()', 'hasOwnProperty(1)', 'isPrototypeOf(1)', 'propertyIsEnumerable(1)',
  '__proto__(1)', '__defineGetter__()', '__lookupSetter__()', 'toLocaleString()', 'Math.constructor(1)', 'Math.toString()',
  'Math.valueOf()', 'Math.__proto__(1)', 'Math.toLocaleString()', 'Math.isPrototypeOf()', 'Math["constructor"](3) * 2',
  'Math.constructor("return globalThis")()', 'layer.constructor', 'layer.__proto__', 'layer.prototype', 'layer["con" + "structor"]',
  'layer.hasOwnProperty', 'comp.layers.map', 'comp.layers.constructor',
  // Operators spelled as strings and prototype names as operators.
  '1 "+" 2', '1 "**" 2 "**" 3', '"-" 3', '"!" 0', 'sin "(" 1 ")"', 'Math "." sin(1)', '1 "?" 2 ":" 3', 'f("(")', '"(" 1',
  '1 constructor 2', '1 toString 2 ? 3 : 4', 'value valueOf t toString 3', 'value toString t valueOf 3', '1 __proto__ 2',
  '1 __proto__ 2 __proto__ 3', '1 constructor 2 + 3', '1 + 2 constructor 3 * 4', '1 "constructor" 2', '1 hasOwnProperty 2 || 3',
  '1 toLocaleString 2 isPrototypeOf 3 propertyIsEnumerable 4', '(1 valueOf 2) ? 5 : 6', 'f(1 toString 2)',
  '1 __defineGetter__ 2 __defineSetter__ 3 __lookupGetter__ 4 __lookupSetter__ 5',
  // Members and indexes.
  'layer.name', 'layer.name.length', 'layer["name"]', 'layer[\'name\']', 'layer.p', 'layer.p.amount', 'layer.p.opacity.v',
  'layer.p["position.x"].kf[1].v', 'layer.tags', 'layer.tags.length', 'layer.tags[0]', 'layer.tags["1"]', 'layer.tags[1.0]',
  'layer.tags["01"]', 'layer.tags[" 1 "]', 'layer.tags["1e0"]', 'layer.tags["0x1"]', 'layer.tags[""]', 'layer.tags[-0]',
  'layer.tags[-1]', 'layer.tags[1.5]', 'layer.tags[3]', 'layer.tags[null]', 'layer.tags[true]', 'layer.tags[[1]]',
  'layer.tags[layer.tags]', 'layer.tags[NaN]', 'layer.tags[1/0]', 'layer.tags[9007199254740991]', 'layer.tags[9007199254740992]',
  'layer.tags.join', 'layer.nested.list[1][1][0]', 'layer.nested.list + ""', 'layer.nested.empty + ""', 'layer.nested.zero',
  'layer.nested.blank.length', 'layer.missing', 'layer.missing.deeper', 'layer[0]', 'layer[1e21]', 'layer[-0]', 'layer[layer]',
  'comp.layers.length', 'comp.layers[idx]', 'comp.layers[idx] == layer', 'comp.layers[idx] === layer', 'comp.layers[0] == layer',
  'comp.layers[1].p === layer.p', 'comp == comp', 'comp === comp', 'layer == comp', 'value == value', 'value === value',
  'comp.empty + 1', 'comp.single + 1', 'comp.single * 2', 'comp.pair + 1', 'comp.pair * 1', 'comp.nested + ""', 'comp.text * 2',
  'comp.text + 1', 'comp.empty == 0', 'comp.empty == ""', 'comp.single == 5', 'comp.single == "5"', 'comp.pair == "1,2"',
  'comp.empty == false', 'comp.markers[0].t', 'comp.params.gain.value', 'comp.w * 0.1', 'ch.length', 'ch[0]', 'ch + idx',
  'value.length', 'value[0]', 'value.x', 'value + 1', 'value * 2', '"" + value', 'value == "1,2,3"', 'value.callable',
  'clamp.callable', 'sin.callable', 'clamp.callable.length', '"abc".length', '"abc"[0]', '"😀".length', '(1).x', 'true.x', 'null.x',
  'undefined.x', 'Math.sin.name', 'Math.sin.length', 'layer.p.constructor', 'layer.length', 'layer.valueOf', 'layer.toString',
  // Coercion and equality.
  'layer + 1', '"" + layer', 'layer == "[object Object]"', 'layer == 1', '!layer', '-layer', '+comp', 'comp + ""', 'comp * 1',
  '"" + Math', '"" + Math.sin', '"" + Math.max', '"" + clamp', 'Math.sin + 1', 'Math == Math', 'Math.sin == Math.sin',
  'Math.sin === Math["sin"]', 'Math.sin == Math.cos', 'clamp == clamp', 'sin == Math.sin', '"" + t', '"" + -0', '"" + 1e21',
  '"" + 1e-7', '"" + 123456789012345680000', '"" + 0.1', '"" + (0.1 + 0.2)', '"" + 5e-324', '"" + 1/0', '"" + -1/0', '"" + 0/0',
  '"" + null', '"" + true', '"" + undefined', '"1" + 2', '1 + "2"', '"3" * "4"', '"3" - 1', '" 12 " * 1', '"0x1F" * 1',
  '"0b101" * 1', '"0o17" * 1', '"-0x1" * 1', '"1e3" * 1', '"Infinity" * 1', '"-Infinity" * 1', '"infinity" * 1', '"" * 1',
  '"\\t\\n" * 1', '"1_0" * 1', '".5" * 1', '"5." * 1', '"." * 1', '"+.5e1" * 1', '"0x" * 1', '"1e" * 1', 'null + 1',
  'true + true', 'undefined + 1', '"2" == 2', '"" == 0', '" " == 0', '"0" == false', 'null == undefined', 'null == 0',
  'undefined == 0', 'null === null', 'NaN == NaN', '0 == -0', '0 === -0', '"a" < "b"', '"10" < "9"', '"a" < 1', 'null < 1',
  'undefined < 1', 'true > false', '1 < 2 < 3', '3 > 2 > 1', '!""', '!"0"', '!0', '!-0', '!NaN', '!!layer', '-"3"', '+"3"',
  '+""', '-null', '+true', '-undefined', '7 % 3', '-7 % 3', '7 % -3', '-7 % -3', '5.5 % 2', '1 % 0', '0 % 5', '-0 % 5',
  '1 / 0', '-1 / 0', '0 / 0', '1 / -0', '0.1 + 0.2', '1e308 * 10', '2 ** 1024', '1 && 2', '0 && 2', '1 || 2', '0 || 2',
  '"" || "x"', 'null || undefined', 'layer && comp', 't > 1 ? "late" : "early"', '1 ? 2 ? 3 : 4 : 5', '0 ? 1 : 0 ? 2 : 3',
  'value > 5 && value < 20', 'idx === 2 ? layer.name + " " + comp.dur : "no"', 'value + Math.sin(t) * 10', 'clamp(param("gain") * 5, 0, 12)',
  // The new name (native dialect only; undefined in the legacy language).
  'frame', 'frame + 1', 'frame == undefined', 'Math.floor(time * fps)',
];

// ── the coercion matrix ────────────────────────────────────────────────────

const ATOMS = [
  'undefined', 'null', 'true', 'false', '0', '-0', '1', '0/0', '1/0', '""', '"0"', '"1"', '" 1 "', '"a"', '"[object Object]"',
  '"1,2"', 'layer', 'comp', 'comp.layers', 'comp.single', 'comp.empty', 'comp.pair', 'Math', 'Math.sin', 'clamp',
  'constructor(1)', 'constructor()', 'value', 'layer.name', 'ch',
];
const MATRIX_OPS = ['==', '!=', '===', '!==', '<', '>=', '+', '-', '*', '%', '**', '&&', '||'];

export function matrixCases(): string[] {
  const out: string[] = [];
  for (const a of ATOMS) for (const b of ATOMS) for (const op of MATRIX_OPS) out.push(`(${a}) ${op} (${b})`);
  return out;
}

// ── the grammar fuzzer ─────────────────────────────────────────────────────

const NUMBERS = ['0', '1', '2', '0.5', '.25', '3.', '1e3', '2.5e-3', '1E2', '127.1', '43758.5453', '9007199254740993',
  '1e21', '1e-7', '0.1', '360', '1000000', '5e-324', '1.7976931348623157e308'];
const STRINGS = ['""', '"a"', "'b'", '"0"', '" 7 "', '"1e2"', '"0x10"', '"Infinity"', '"-0"', '"name"', '"gain"', '"label"',
  '"position.x"', '"😀"', '"\\n"', "'\\''", '"\\u0041"', '"length"', '"callable"', '"constructor"', '"1"', '"[object Object]"'];
const IDENTS = ['time', 't', 'T', 'fps', 'value', 'layer', 'comp', 'ch', 'idx', 'PI', 'Math', 'frame', 'foo', 'undefined',
  'NaN', 'true', 'false', 'null', 'clamp', 'sin', 'constructor', 'toString'];
const PATHS = ['layer.name', 'layer.from', 'layer.dur', 'layer.p', 'layer.p.amount', 'layer.p.opacity.v', 'layer.tags',
  'layer.tags[1]', 'layer.nested.list', 'comp.layers', 'comp.layers.length', 'comp.layers[idx]', 'comp.dur', 'comp.fps',
  'comp.w', 'comp.params', 'comp.params.gain', 'comp.single', 'comp.pair', 'comp.text', 'value.length', 'value[0]',
  'Math.PI', 'Math.E', 'Math.SQRT2', 'ch.length', 'layer.missing', 'comp.markers[0].t'];
const HELPERS = ['clamp', 'lerp', 'linear', 'ease', 'random', 'wiggle', 'bounce', 'loop', 'pingpong', 'param'];
const MATH = ['abs', 'acos', 'asin', 'atan', 'atan2', 'ceil', 'cos', 'exp', 'floor', 'log', 'max', 'min', 'pow', 'round',
  'sign', 'sin', 'sqrt', 'tan'];
const BINARY = ['+', '-', '*', '/', '%', '**', '<', '<=', '>', '>=', '==', '!=', '===', '!==', '&&', '||'];
const SPACES = ['', ' ', ' ', ' ', '  ', '\t', '\n', ' ', '　'];

export function fuzzCases(count: number, seed: number): string[] {
  const rand = mulberry32(seed);
  const pick = <T>(list: readonly T[]): T => list[Math.floor(rand() * list.length)]!;
  const sp = () => (rand() < 0.85 ? (rand() < 0.7 ? ' ' : '') : pick(SPACES));
  const expr = (depth: number): string => {
    const r = rand();
    if (depth <= 0 || r < 0.25) {
      const k = rand();
      if (k < 0.3) return pick(NUMBERS);
      if (k < 0.45) return pick(STRINGS);
      if (k < 0.7) return pick(IDENTS);
      return pick(PATHS);
    }
    if (r < 0.5) return `${expr(depth - 1)}${sp()}${pick(BINARY)}${sp()}${expr(depth - 1)}`;
    if (r < 0.58) return `${pick(['-', '+', '!'])}${expr(depth - 1)}`;
    if (r < 0.66) return `(${expr(depth - 1)})`;
    if (r < 0.72) return `${expr(depth - 1)} ? ${expr(depth - 1)} : ${expr(depth - 1)}`;
    if (r < 0.84) {
      const n = Math.floor(rand() * 6);
      const args = Array.from({ length: n }, () => expr(depth - 2));
      return `${pick(HELPERS)}(${args.join(', ')})`;
    }
    if (r < 0.95) {
      const n = Math.floor(rand() * 4);
      const args = Array.from({ length: n }, () => expr(depth - 2));
      return `${rand() < 0.5 ? 'Math.' : ''}${pick(MATH)}(${args.join(',' + sp())})`;
    }
    return `${pick(PATHS)}[${expr(depth - 1)}]`;
  };
  const out: string[] = [];
  while (out.length < count) {
    let source = expr(1 + Math.floor(rand() * 5));
    // Occasionally damage the source to exercise errors.
    if (rand() < 0.06) {
      const at = Math.floor(rand() * (source.length + 1));
      source = source.slice(0, at) + pick(['(', ')', '"', '@', '=', '.', ',', '?', '[', ':', '&']) + source.slice(at);
    }
    out.push(source);
  }
  return out;
}
