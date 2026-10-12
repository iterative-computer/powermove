/**
 * Generates tests/vectors/patch-vectors.json by running the real
 * `apps/desktop/src/shared/patch.ts` under bun. The Rust tests replay every
 * case and must reproduce the document, the thrown error kind, and the index
 * of the throwing patch exactly.
 *
 *   bun engine/crates/pm-model/tests/vectors/generate-patch-vectors.ts
 *
 * A larger local run: PM_PATCH_CASES=50000 PM_PATCH_OUT=/tmp/patch.json, then
 * PM_PATCH_VECTORS=/tmp/patch.json cargo test -p pm-model --test patch_semantics.
 *
 * Deterministic: a seeded PRNG drives the random cases, so two runs write the
 * same bytes. Random cases stay inside the JSON-visible model and avoid the
 * documented limits in pm_model::patch: paths never navigate through inherited
 * members or non-index array properties (in JavaScript those writes land on
 * shared prototypes and would pollute this generator's own realm), and a
 * `__proto__` final key only receives non-null primitives or deletes, never an
 * object or null (a prototype change) or `undefined` (an own member holding
 * undefined).
 */
import { writeFileSync } from 'node:fs';
import path from 'node:path';
import { applyPatch, isPatchList, type Patch, type PathPart } from '../../../../../apps/desktop/src/shared/patch';

type Case = {
  name: string;
  /** JSON text of the starting document, or null for `undefined`. */
  doc: string | null;
  patches: Patch[];
  /** JSON.stringify of the result, or null for `undefined`. */
  result: string | null;
  error: { index: number; kind: string } | null;
};

function run(name: string, docText: string | null, patches: Patch[]): Case {
  let root: any = docText === null ? undefined : JSON.parse(docText);
  let error: Case['error'] = null;
  for (let index = 0; index < patches.length; index++) {
    try {
      root = applyPatch(root, [patches[index]!]);
    } catch (e) {
      error = { index, kind: (e as Error).constructor.name };
      break;
    }
  }
  const result = root === undefined ? null : JSON.stringify(root);
  return { name, doc: docText, patches, result: result ?? null, error };
}

const curated: Case[] = [];
const add = (name: string, doc: unknown, patches: Patch[]) =>
  curated.push(run(name, doc === undefined ? null : JSON.stringify(doc), patches));
const P = (path: PathPart[], value?: unknown): Patch => (value === undefined ? { path, exists: true } : { path, exists: true, value });
const D = (path: PathPart[]): Patch => ({ path, exists: false });

const layerDoc = {
  id: 'P', fps: 30, layers: [
    { id: 'a', name: 'A', p: { 'position.x': { v: 0, kf: [], expr: null } } },
    { id: 'b', name: 'B', p: {} },
    { id: 'c', name: 'C', p: {} },
  ], comps: { C2: { id: 'C2', layers: [] } }, assets: {},
};

// The shapes history.ts and remote-sync.ts produce.
add('leaf set on a layer channel', layerDoc, [P(['layers', 0, 'p', 'position.x', 'v'], 12.5)]);
add('string and number indices address the same element', layerDoc, [P(['layers', '1', 'name'], 'Bee'), P(['layers', 1, 'name'], 'B2')]);
add('whole array replacement (reorder)', layerDoc, [P(['layers'], [layerDoc.layers[2], layerDoc.layers[0], layerDoc.layers[1]])]);
add('member delete', layerDoc, [D(['layers', 0, 'p', 'position.x'])]);
add('numeric delete splices', layerDoc, [D(['layers', 1])]);
add('string delete leaves a hole', layerDoc, [D(['layers', '1'])]);
add('negative numeric delete splices from the end', layerDoc, [D(['layers', -1])]);
add('fractional numeric delete truncates', layerDoc, [D(['layers', 1.7])]);
add('out-of-range numeric delete is a no-op', layerDoc, [D(['layers', 9])]);
add('far negative numeric delete removes the first element', layerDoc, [D(['layers', -9])]);
add('set past the end leaves holes', layerDoc, [P(['layers', 5], { id: 'z' })]);
add('array length shrink', layerDoc, [P(['layers', 'length'], 1)]);
add('array length grow', layerDoc, [P(['layers', 'length'], 5)]);
add('array length from a numeric string', layerDoc, [P(['layers', 'length'], '2')]);
add('invalid array length throws RangeError', layerDoc, [P(['fps'], 24), P(['layers', 'length'], -1), P(['fps'], 60)]);
add('fractional array length throws RangeError', layerDoc, [P(['layers', 'length'], 1.5)]);
add('undefined array length throws RangeError', layerDoc, [P(['layers', 'length'])]);
add('deleting array length throws TypeError', layerDoc, [D(['layers', 'length'])]);
add('non-index array key is invisible', layerDoc, [P(['layers', 'foo'], 1), P(['layers', '01'], 2)]);
add('undefined member value removes it', layerDoc, [P(['fps'])]);
add('undefined element value becomes null', layerDoc, [P(['layers', 0])]);
add('missing parent is a no-op', layerDoc, [P(['nope', 'deeper'], 1), P(['layers', 7, 'name'], 'x')]);
add('null parent is a no-op', { a: null }, [P(['a', 'b'], 1)]);
add('creating a member on a number throws', layerDoc, [P(['fps', 'x'], 1)]);
add('creating a member on a string throws', layerDoc, [P(['id', 'x'], 1)]);
add('creating a member on a boolean throws', { on: true }, [P(['on', 'x'], 1)]);
add('assigning a string index throws', layerDoc, [P(['id', '0'], 'Q')]);
add('navigating into a string', { s: 'abc' }, [P(['s', 1, 'x'], 1)]);
add('navigating into string length', { s: 'abc' }, [P(['s', 'length', 'x'], 1)]);
add('navigating into array length', layerDoc, [P(['layers', 'length', 'x'], 1)]);
add('deleting a string index throws', { s: 'abc' }, [D(['s', 0])]);
add('deleting a string length throws', { s: 'abc' }, [D(['s', 'length'])]);
add('deleting past a string end is fine', { s: 'abc' }, [D(['s', 5]), D(['s', 'x'])]);
add('deleting on a number is fine', { n: 5 }, [D(['n', 'x'])]);
add('setting __proto__ calls the setter', { a: {} }, [P(['a', '__proto__'], { x: 1 }), P(['__proto__'], 5)]);
add('__proto__ on a primitive is ignored', { s: 'abc' }, [P(['s', '__proto__'], 1)]);
add('deleting a non-own __proto__ is a no-op', { a: 1 }, [D(['__proto__'])]);
add('root replacement', layerDoc, [P([], { fresh: true })]);
add('root delete', layerDoc, [D([])]);
add('patch on an undefined root is a no-op', undefined, [P(['a'], 1)]);
add('root replacement on an undefined root', undefined, [P([], [1, 2])]);
add('root replaced with a primitive, then patched', layerDoc, [P([], 'str'), P(['x'], 1)]);
add('patching a null root', null, [P(['x'], 1)]);
add('values are cloned through JSON', layerDoc, [P(['v'], { a: -0, b: [1, -0], c: 'x' })]);
add('later patches see earlier ones', layerDoc, [P(['layers', 3], { id: 'd', p: {} }), P(['layers', 3, 'p', 'x'], 1), D(['layers', 0])]);
add('error keeps earlier patches applied', layerDoc, [P(['fps'], 25), P(['fps', 'x'], 1), P(['fps'], 50)]);
add('emoji string indexing yields surrogate halves', { s: 'a😀' }, [P(['s', 1, 'x'], 1)]);
add('canonical composition paths', layerDoc, [P(['comps', 'C2', 'layers'], [{ id: 'n' }]), P(['comps', 'C2', 'layers', 0, 'name'], 'N')]);

// isPatchList cases.
const patchLists: Array<{ value: unknown; maxBytes: number; ok: boolean }> = [];
const checkList = (value: unknown, maxBytes = 32 * 1024 * 1024) => patchLists.push({ value, maxBytes, ok: isPatchList(value, maxBytes) });
checkList([]);
checkList({});
checkList([{ path: [], exists: true }]);
checkList([{ path: ['a', 0], exists: false }]);
checkList([{ path: ['a', -1], exists: false }]);
checkList([{ path: ['a', 1.5], exists: false }]);
checkList([{ path: ['a', 1e300], exists: false }]);
checkList([{ path: ['a', true], exists: false }]);
checkList([{ path: ['a'], exists: 1 }]);
checkList([{ path: 'a', exists: true }]);
checkList([null]);
checkList([{ path: ['a'], exists: true, value: 'x'.repeat(10) }], 12);
checkList([{ path: ['a'], exists: true, value: 'x'.repeat(10) }], 11);
checkList([{ path: ['a'], exists: true, value: '😀😀' }], 6);
checkList([{ path: ['a'], exists: true, value: '😀😀' }], 5);
checkList([{ path: ['a'], exists: true }], 4);
checkList([{ path: ['a'], exists: true }], 3);
checkList([{ path: ['a'], exists: true, value: { b: [1, 2, { c: null }] } }, { path: ['b'], exists: false, value: 'x'.repeat(100) }], 22);
checkList([{ path: ['a'], exists: true, value: { b: [1, 2, { c: null }] } }], 21);

// ── Random cases ─────────────────────────────────────────────────────────────
let seed = 0x5eed1234;
const rand = () => { seed |= 0; seed = (seed + 0x6d2b79f5) | 0; let t = Math.imul(seed ^ (seed >>> 15), 1 | seed); t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t; return ((t ^ (t >>> 14)) >>> 0) / 4294967296; };
const pick = <T>(items: readonly T[]): T => items[Math.floor(rand() * items.length)]!;
const KEYS = ['a', 'b', 'id', 'v', 'kf', 'layers', '0', '1', '10', '2', 'é', '😀', '__proto__'];

function randomValue(depth: number): unknown {
  const r = rand();
  if (depth <= 0 || r < 0.35) {
    return pick([null, true, false, 0, 1, -3.5, 1e21, 0.1, 'x', '', 'abc', 'a😀', 2.5e-7]);
  }
  if (r < 0.65) {
    const n = Math.floor(rand() * 4);
    return Array.from({ length: n }, (_, i) => rand() < 0.5 ? { id: pick(['a', 'b', 'c', 'd']), n: i, child: randomValue(depth - 1) } : randomValue(depth - 1));
  }
  const out: Record<string, unknown> = {};
  const n = Math.floor(rand() * 4);
  for (let i = 0; i < n; i++) {
    const key = pick(KEYS);
    if (key === '__proto__') continue; // added below through JSON text
    out[key] = randomValue(depth - 1);
  }
  return out;
}

/** JSON text, sometimes with an own "__proto__" member. */
function randomDocText(): string {
  const value = randomValue(3);
  let text = JSON.stringify(value);
  if (text.startsWith('{') && rand() < 0.15) text = text === '{}' ? '{"__proto__":{"q":1}}' : '{"__proto__":{"q":1},' + text.slice(1);
  return text;
}

/** JSON-visible own keys. Arrays may also hold non-index properties (a patch
 * wrote `layers.foo`); JSON never sees them and pm-model drops them, so paths
 * never navigate through them (see pm_model::patch, "documented limits"). */
const ownKeys = (node: object): string[] =>
  Array.isArray(node) ? Object.keys(node).filter(key => /^(0|[1-9][0-9]*)$/.test(key)) : Object.keys(node);

/** Paths through own members only, then a random final key. */
function randomPath(root: unknown): PathPart[] {
  const out: PathPart[] = [];
  let node: any = root;
  const depth = Math.floor(rand() * 4);
  for (let i = 0; i < depth; i++) {
    if (node === null || typeof node !== 'object') break;
    const keys = ownKeys(node);
    if (!keys.length || rand() < 0.2) break;
    const key = pick(keys);
    out.push(Array.isArray(node) && rand() < 0.6 ? Number(key) : key);
    node = node[key];
  }
  // Mostly write into containers; primitives as parents mostly throw.
  if ((node === null || typeof node !== 'object') && out.length && rand() < 0.8) {
    out.pop();
    node = out.reduce((n: any, k) => n?.[k], root);
  }
  const finals: PathPart[] = ['a', 'id', 'length', 0, 1, 2, '1', 5, -1, 1.5, '__proto__', 'new'];
  if (node !== null && typeof node === 'object' && rand() < 0.6) {
    const keys = ownKeys(node);
    if (keys.length) {
      const key = pick(keys);
      out.push(Array.isArray(node) && rand() < 0.6 ? Number(key) : key);
      return out;
    }
  }
  const last = pick(finals);
  // A non-own __proto__ may only be the final key (see the header).
  out.push(last);
  return out;
}

const random: Case[] = [];
const RANDOM_CASES = Number(process.env.PM_PATCH_CASES || 1500);
for (let i = 0; i < RANDOM_CASES; i++) {
  const docText = rand() < 0.03 ? null : randomDocText();
  let root: any = docText === null ? undefined : JSON.parse(docText);
  const patches: Patch[] = [];
  const count = 1 + Math.floor(rand() * 4);
  for (let j = 0; j < count; j++) {
    const p = randomPath(root);
    const r = rand();
    let patch: Patch = r < 0.55 ? { path: p, exists: true, value: randomValue(2) }
      : r < 0.6 ? { path: p, exists: true }
      : r < 0.65 ? { path: p, exists: true, value: pick([-1, 1.5, 3, '2', 'x', null]) }
      : { path: p, exists: false };
    if (p.at(-1) === '__proto__' && patch.exists) patch = { path: p, exists: true, value: pick([1, 'x', true]) };
    patches.push(patch);
    try { root = applyPatch(root, [patch]); } catch { break; }
  }
  random.push(run(`random ${i}`, docText, patches));
}

const out = { generatedBy: 'engine/crates/pm-model/tests/vectors/generate-patch-vectors.ts', cases: [...curated, ...random], patchLists };
writeFileSync(process.env.PM_PATCH_OUT || path.join(import.meta.dir, 'patch-vectors.json'), JSON.stringify(out) + '\n');
const errors = out.cases.filter(c => c.error).length;
console.log(`wrote ${out.cases.length} cases (${errors} throwing) and ${patchLists.length} isPatchList cases`);
