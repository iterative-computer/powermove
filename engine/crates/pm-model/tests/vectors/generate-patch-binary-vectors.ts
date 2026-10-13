/**
 * Generates tests/vectors/patch-binary-vectors.json: cross-language golden
 * vectors for the canonical binary patch format (contract 5, canonicalV1) and
 * the id-keyed operations, produced by the real renderer modules
 * (apps/desktop/src/shared/patch-ids.ts, patch-binary.ts, patch.ts).
 *
 *   bun engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors.ts
 *
 * The Rust tests (tests/patch_binary.rs) check, for every case, that
 * pm_model's decoder reads the TypeScript encoder's bytes as the same
 * operations, that its encoder writes the same bytes, that its own generator
 * (ops::diff_ops) emits the same operations, and that applying them gives the
 * same document as patch.ts's leaf patches. `bun test` in apps/desktop
 * (patch-binary.test.ts) regenerates the vectors in memory and fails if this
 * file is stale.
 *
 * Deterministic: a seeded PRNG drives the random cases. Documents avoid -0,
 * NaN, and `__proto__` keys, which JSON text does not carry (the Rust side
 * reads the documents from their JSON text).
 */
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { canonicalStringify } from '../../../../../apps/desktop/src/shared/canonical-json';
import { applyPatch, diffPatches } from '../../../../../apps/desktop/src/shared/patch';
import { encodePatchOps } from '../../../../../apps/desktop/src/shared/patch-binary';
import { applyOps, diffOps, IdOpMismatch, type PatchOp } from '../../../../../apps/desktop/src/shared/patch-ids';

export type DiffCase = {
  name: string;
  /** JSON text, or null for `undefined`. */
  before: string | null;
  after: string | null;
  /** diffOps(before, after) in JSON form. */
  ops: PatchOp[];
  /** encodePatchOps(ops) as lowercase hex. Applying the operations to `before` gives `after` (checked at generation). */
  hex: string;
};

export type ApplyCase = {
  name: string;
  doc: string | null;
  ops: PatchOp[];
  hex: string;
  result: string | null;
  /** The operation that threw and its kind ('Mismatch', 'TypeError', 'RangeError'). */
  error: { index: number; kind: string } | null;
};

export type InvalidCase = { name: string; hex: string };

export type PatchBinaryVectors = {
  about: string;
  format: 'canonicalV1';
  cases: DiffCase[];
  applyCases: ApplyCase[];
  invalid: InvalidCase[];
};

export const hex = (bytes: Uint8Array) => Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
const text = (value: unknown) => (value === undefined ? null : JSON.stringify(value));
const clone = (value: unknown) => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)));

/** One diff case: diffOps and diffPatches must agree, or generation fails. */
function diffCase(name: string, before: unknown, after: unknown): DiffCase {
  const ops = diffOps(before, after);
  const byOps = applyOps(clone(before), ops);
  const byLeaf = applyPatch(clone(before), diffPatches(before, after).forward);
  const expected = canonicalStringify(after);
  if (canonicalStringify(byOps) !== canonicalStringify(byLeaf) || canonicalStringify(byOps) !== expected) {
    throw new Error(`${name}: diffOps and diffPatches disagree`);
  }
  return { name, before: text(before), after: text(after), ops: clone(ops) as PatchOp[], hex: hex(encodePatchOps(ops)) };
}

function applyCase(name: string, doc: unknown, ops: PatchOp[]): ApplyCase {
  let root = clone(doc);
  let error: ApplyCase['error'] = null;
  for (let index = 0; index < ops.length; index++) {
    try {
      root = applyOps(root, [ops[index]!]);
    } catch (e) {
      const kind = e instanceof IdOpMismatch ? 'Mismatch' : (e as Error).constructor.name;
      error = { index, kind };
      break;
    }
  }
  return { name, doc: text(doc), ops, hex: hex(encodePatchOps(ops)), result: text(root), error };
}

// ── Documents ───────────────────────────────────────────────────────────────

const layer = (id: string, extra: Record<string, unknown> = {}) => ({ id, name: `Layer ${id}`, from: 0, p: { 'position.x': { v: 0, kf: [] } }, ...extra });
const project = (layers: unknown[]) => ({ id: 'P', compId: 'c1', comps: { c1: { id: 'c1', fps: 30, layers } }, assets: [] });
const ids = (n: number, prefix = 'L') => Array.from({ length: n }, (_, i) => `${prefix}${i}`);

function curated(): DiffCase[] {
  const out: DiffCase[] = [];
  const add = (name: string, before: unknown, after: unknown) => out.push(diffCase(name, before, after));
  const five = ids(5).map((id) => layer(id));
  add('reorder: swap two layers', project(five), project([five[0], five[3], five[2], five[1], five[4]]));
  add('reorder: move the last layer to the front', project(five), project([five[4], ...five.slice(0, 4)]));
  add('insert: one layer in the middle', project(five), project([...five.slice(0, 2), layer('new'), ...five.slice(2)]));
  add('insert: two layers at the ends', project(five), project([layer('a'), ...five, layer('z')]));
  add('delete: one layer', project(five), project(five.filter((_, i) => i !== 2)));
  add('delete: all but one layer', project(five), project([five[3]]));
  add('reorder, insert, delete, and edit together', project(five), project([
    five[4], layer('n1', { name: 'inserted' }), { ...five[0], name: 'renamed', p: { 'position.x': { v: 12, kf: [{ t: 0.5, v: 3 }] } } }, five[2],
  ]));
  add('edit only: no structural change', project(five), project(five.map((l, i) => (i === 1 ? { ...l, from: 1.5 } : l))));
  const nested = (kids: string[]) => ({ id: 'g', kids: kids.map((id) => ({ id, v: id.length })) });
  add('nested id-keyed arrays', project([nested(['a', 'b', 'c']), five[0]]), project([five[0], nested(['c', 'a', 'd'])]));
  add('fallback: items without ids', { list: [{ v: 1 }, { v: 2 }] }, { list: [{ v: 2 }] });
  add('fallback: one item lacks an id', { list: [{ id: 'a' }, { v: 2 }] }, { list: [{ v: 2 }, { id: 'a' }] });
  add('fallback: duplicate ids', { list: [{ id: 'a', v: 1 }, { id: 'a', v: 2 }] }, { list: [{ id: 'a', v: 2 }] });
  add('fallback: no shared id', { list: [{ id: 'a' }, { id: 'b' }] }, { list: [{ id: 'c' }] });
  add('fallback: empty to one item', { list: [] }, { list: [{ id: 'a' }] });
  add('fallback: primitives', { work: [0, 5] }, { work: [0, 5, 7] });
  add('fallback: numeric ids are not ids', { list: [{ id: 1 }, { id: 2 }] }, { list: [{ id: 2 }, { id: 1 }] });
  add('canonical key order', { b: 1, a: 1, 10: 1, 2: 1, '': 1, '😀': 1 }, { b: 2, a: 2, 10: 2, 2: 2, '': 2, '😀': 2 });
  add('lone surrogates in ids and keys', { list: [{ id: 'x\ud800' }, { id: '\udc00y' }, { id: '😀' }], ['k\udfff']: 1 }, { list: [{ id: '😀' }, { id: 'x\ud800' }], ['k\udfff']: 2 });
  add('type change: array to object', { a: [{ id: 'x' }] }, { a: { id: 'x' } });
  add('whole document from undefined', undefined, project(five));
  add('whole document to undefined', project(five), undefined);
  add('large reorder', { layers: ids(200).map((id) => ({ id, n: id })) }, { layers: ids(200).map((id) => ({ id, n: id })).reverse() });
  return out;
}

function curatedApply(): ApplyCase[] {
  const doc = project(ids(3).map((id) => layer(id)));
  const L = ['comps', 'c1', 'layers'];
  const out: ApplyCase[] = [];
  const add = (name: string, d: unknown, ops: PatchOp[]) => out.push(applyCase(name, d, ops));
  add('order', doc, [{ op: 'order', path: L, ids: ['L2', 'L0', 'L1'] }]);
  add('order that changes nothing', doc, [{ op: 'order', path: L, ids: ['L0', 'L1', 'L2'] }]);
  add('insert at the end', doc, [{ op: 'insert', path: L, index: 3, value: layer('N') }]);
  add('delete then insert the same id', doc, [{ op: 'delete', path: L, id: 'L1' }, { op: 'insert', path: L, index: 0, value: layer('L1', { name: 'back' }) }]);
  add('a leaf patch after an order addresses the new index', doc, [{ op: 'order', path: L, ids: ['L2', 'L0', 'L1'] }, { path: [...L, 0, 'name'], exists: true, value: 'first' }]);
  add('leaf set of undefined', { a: 1, b: [1, 2] }, [{ path: ['a'], exists: true }, { path: ['b', 0], exists: true }]);
  add('path numbers: -0 and 2^32', { l: [{ x: 1 }], o: { 4294967296: 1 } }, [{ path: ['l', -0, 'x'], exists: true, value: 2 }, { path: ['o', 4294967296], exists: false }]);
  add('mismatch: delete of an unknown id', doc, [{ op: 'delete', path: L, id: 'nope' }]);
  add('mismatch: order with too few ids', doc, [{ op: 'order', path: L, ids: ['L0', 'L1'] }]);
  add('mismatch: order with a repeated id', doc, [{ op: 'order', path: L, ids: ['L0', 'L0', 'L1'] }]);
  add('mismatch: order with an unknown id', doc, [{ op: 'order', path: L, ids: ['L0', 'L1', 'X'] }]);
  add('mismatch: insert of a present id', doc, [{ op: 'insert', path: L, index: 0, value: layer('L0') }]);
  add('mismatch: insert past the end', doc, [{ op: 'insert', path: L, index: 4, value: layer('N') }]);
  add('mismatch: missing path', doc, [{ op: 'delete', path: ['comps', 'c2', 'layers'], id: 'L0' }]);
  add('mismatch: path is not an array', doc, [{ op: 'delete', path: ['comps', 'c1'], id: 'L0' }]);
  add('mismatch: an element without an id', { l: [{ id: 'a' }, { v: 1 }] }, [{ op: 'delete', path: ['l'], id: 'a' }]);
  add('mismatch: duplicate ids in the array', { l: [{ id: 'a' }, { id: 'a' }] }, [{ op: 'order', path: ['l'], ids: ['a', 'a'] }]);
  add('mismatch: undefined document', undefined, [{ op: 'delete', path: [], id: 'a' }]);
  add('mismatch after a successful operation', doc, [{ op: 'delete', path: L, id: 'L0' }, { op: 'delete', path: L, id: 'L0' }]);
  add('TypeError after an id-keyed operation', doc, [{ op: 'delete', path: L, id: 'L0' }, { path: ['id', 'x'], exists: true, value: 1 }]);
  return out;
}

function invalid(): InvalidCase[] {
  const good = encodePatchOps([{ path: ['a'], exists: true, value: 1 }]);
  const u32 = (n: number) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, n, true); return Array.from(b); };
  const f64 = (n: number) => { const b = new Uint8Array(8); new DataView(b.buffer).setFloat64(0, n, true); return Array.from(b); };
  const body = (...ops: number[][]) => [0x50, 0x4d, 0x50, 0x01, ...u32(ops.length), ...ops.flat()];
  const str = (bytes: number[]) => [...u32(bytes.length), ...bytes];
  /** UTF-8 text with its length (JSON values are texts too). */
  const json = (s: string) => str(Array.from(new TextEncoder().encode(s)));
  const pathOf = (...parts: number[][]) => [...u32(parts.length), ...parts.flat()];
  const key = (s: string) => [0x00, ...json(s)];
  const cases: [string, number[]][] = [
    ['empty', []],
    ['bad magic', [0x50, 0x4d, 0x50, 0x02, ...u32(0)]],
    ['truncated count', [0x50, 0x4d, 0x50, 0x01, 0, 0]],
    ['count exceeds the bytes', [0x50, 0x4d, 0x50, 0x01, ...u32(1)]],
    ['huge count', [0x50, 0x4d, 0x50, 0x01, ...u32(0xffffffff)]],
    ['truncated value', Array.from(good.subarray(0, good.length - 1))],
    ['trailing bytes', [...Array.from(good), 0]],
    ['unknown operation tag', body([0x03, ...pathOf()])],
    ['unknown path part tag', body([0x00, ...pathOf([0x03])])],
    ['non-canonical number part', body([0x00, ...pathOf([0x02, ...f64(3)])])],
    ['fractional number part', body([0x00, ...pathOf([0x02, ...f64(1.5)])])],
    ['negative number part', body([0x00, ...pathOf([0x02, ...f64(-4294967297)])])],
    ['NaN number part', body([0x00, ...pathOf([0x02, ...f64(NaN)])])],
    ['infinite number part', body([0x00, ...pathOf([0x02, ...f64(Infinity)])])],
    ['invalid JSON value', body([0x01, ...pathOf(key('a')), ...json('{"a":}')])],
    ['JSON value with trailing data', body([0x01, ...pathOf(key('a')), ...json('1 2')])],
    ['JSON value with a byte order mark', body([0x01, ...pathOf(key('a')), ...str([0xef, 0xbb, 0xbf, 0x31])])],
    ['JSON value not UTF-8', body([0x01, ...pathOf(key('a')), ...str([0x22, 0xff, 0x22])])],
    ['JSON value with a raw surrogate', body([0x01, ...pathOf(key('a')), ...str([0x22, 0xed, 0xa0, 0x80, 0x22])])],
    ['insert of an array', body([0x11, ...pathOf(), ...u32(0), ...json('[1]')])],
    ['insert without an id', body([0x11, ...pathOf(), ...u32(0), ...json('{"v":1}')])],
    ['insert with a numeric id', body([0x11, ...pathOf(), ...u32(0), ...json('{"id":1}')])],
    ['key not WTF-8', body([0x00, ...pathOf([0x00, ...str([0xff])])])],
    ['key with an overlong sequence', body([0x00, ...pathOf([0x00, ...str([0xc0, 0x80])])])],
    ['key with a pair as two surrogates', body([0x00, ...pathOf([0x00, ...str([0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80])])])],
    ['id past U+10FFFF', body([0x12, ...pathOf(), ...str([0xf4, 0x90, 0x80, 0x80])])],
    ['truncated id', body([0x12, ...pathOf(), ...u32(5), 0x61])],
    ['order count exceeds the bytes', body([0x10, ...pathOf(), ...u32(1000)])],
    ['path count exceeds the bytes', body([0x00, ...u32(1000)])],
  ];
  return cases.map(([name, bytes]) => ({ name, hex: hex(Uint8Array.from(bytes)) }));
}

// ── Random documents ──────────────────────────────────────────────────────

function prng(seed: number) {
  let s = seed | 0;
  return () => {
    s = (s + 0x6d2b79f5) | 0;
    let t = Math.imul(s ^ (s >>> 15), 1 | s);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const KEYS = ['v', 'name', 'kf', 'p', 'x', '10', '2', '', '😀', 'k\ud800'];
const STRINGS = ['', 'a', 'b', 'é', '\udc00'];

export function randomPair(seed: number): [unknown, unknown] {
  const rand = prng(seed);
  const below = (n: number) => Math.floor(rand() * n);
  const chance = (p: number) => rand() < p;
  let nextId = 0;
  const scalar = (): unknown => [null, true, false, below(5), 0.5, 1e21, STRINGS[below(STRINGS.length)]][below(7)];
  const value = (depth: number): unknown => {
    if (depth === 0 || chance(0.45)) return scalar();
    switch (below(3)) {
      case 0: return Array.from({ length: below(4) }, () => value(depth - 1));
      case 1: return idArray(depth - 1);
      default: {
        const o: Record<string, unknown> = {};
        for (let i = below(4); i > 0; i--) o[KEYS[below(KEYS.length)]!] = value(depth - 1);
        return o;
      }
    }
  };
  const item = (depth: number, id: string) => {
    const v = value(depth);
    const o = v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, unknown>) : {};
    o.id = id;
    return o;
  };
  const idArray = (depth: number) => Array.from({ length: below(6) }, () => item(depth, `i${++nextId}${chance(0.05) ? '\ud800' : ''}`));
  const mutate = (v: any, depth: number): any => {
    if (Array.isArray(v)) {
      const keyed = v.every((x) => x && typeof x === 'object' && !Array.isArray(x) && typeof x.id === 'string');
      for (let n = below(3); n > 0; n--) {
        const k = below(9);
        if (k <= 1 && v.length) v.splice(below(v.length), 1);
        else if (k <= 3) v.splice(below(v.length + 1), 0, keyed || chance(0.5) ? item(depth, `n${++nextId}`) : value(depth));
        else if (k <= 5 && v.length > 1) { const i = below(v.length), j = below(v.length); [v[i], v[j]] = [v[j], v[i]]; }
        else if (k === 6 && v.length > 1) { const [moved] = v.splice(below(v.length), 1); v.splice(below(v.length + 1), 0, moved); }
        else if (k === 7 && v.length && chance(0.25)) {
          const target = v[below(v.length)], other = v[below(v.length)];
          if (target && typeof target === 'object' && !Array.isArray(target)) {
            if (chance(0.5) && other && typeof other.id === 'string') target.id = other.id;
            else delete target.id;
          }
        }
      }
      for (let i = 0; i < v.length; i++) if (chance(0.4)) v[i] = mutate(v[i], Math.max(0, depth - 1));
      return v;
    }
    if (v && typeof v === 'object') {
      for (const key of Object.keys(v)) {
        if (key === 'id') continue;
        const k = below(10);
        if (k === 0) delete v[key];
        else if (k === 1) v[key] = value(depth);
        else v[key] = mutate(v[key], Math.max(0, depth - 1));
      }
      if (chance(0.3)) v[KEYS[below(KEYS.length)]!] = value(depth);
      return v;
    }
    return chance(0.3) ? value(depth) : v;
  };
  const before = { layers: idArray(3), markers: idArray(1), misc: value(3) };
  const after = mutate(clone(before), 3);
  return [before, after];
}

export const RANDOM_CASES = 400;

export function buildPatchBinaryVectors(randomCases = RANDOM_CASES): PatchBinaryVectors {
  const cases = curated();
  for (let i = 0; i < randomCases; i++) {
    const [before, after] = randomPair(0x5eed0000 + i);
    cases.push(diffCase(`random ${i}`, before, after));
  }
  return {
    about: 'Generated by engine/crates/pm-model/tests/vectors/generate-patch-binary-vectors.ts from apps/desktop/src/shared/patch-ids.ts and patch-binary.ts. Do not edit.',
    format: 'canonicalV1',
    cases,
    applyCases: curatedApply(),
    invalid: invalid(),
  };
}

export const VECTORS_FILE = path.join(path.dirname(fileURLToPath(import.meta.url)), 'patch-binary-vectors.json');

/** The exact file contents. */
export const serializeVectors = (vectors: PatchBinaryVectors) => JSON.stringify(vectors) + '\n';

if ((import.meta as { main?: boolean }).main) {
  const vectors = buildPatchBinaryVectors(Number(process.env.PM_PATCH_BINARY_CASES || RANDOM_CASES));
  writeFileSync(process.env.PM_PATCH_BINARY_OUT || VECTORS_FILE, serializeVectors(vectors));
  console.log(`${vectors.cases.length} diff cases, ${vectors.applyCases.length} apply cases, ${vectors.invalid.length} invalid bodies`);
}
