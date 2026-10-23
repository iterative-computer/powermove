/*
 * `diffOps` (shared/patch-ids.ts) with the live side read as `JSON.stringify`
 * reads it.
 *
 * The barrier diffs its shadow (plain JSON, exactly what pm-wasm holds)
 * against the live document, which is not plain JSON: keyframes carry
 * non-enumerable accessor views (temporal-bridge.ts), members can be
 * `undefined`, numbers can be `NaN` or `-0`, and values can have `toJSON`.
 * `diffOps` tests membership with `Object.hasOwn`, so an accessor that
 * replaced a stored `eo` would never be deleted from the mirror. This walker
 * sees exactly the JSON the live document serializes to: enumerable own string
 * keys, `toJSON` and primitive wrappers unwrapped, `undefined`, functions, and
 * symbols absent in objects and `null` in arrays, non-finite numbers `null`,
 * `-0` as `0`. Everything else is `diffOps`: the same id-keyed operations for
 * arrays, the same visiting and code-unit ordering, so on plain JSON both emit
 * identical lists (tested).
 */
import type { PathPart } from '../../../shared/patch';
import { isIdKeyed, type PatchOp } from '../../../shared/patch-ids';

/** No JSON form: the member is absent (`undefined`, a function, a symbol). */
const ABSENT = Symbol('absent');

/** A live value as JSON sees it in member position `key`. */
export function jsonValue(value: any, key: string): any {
  if (value !== null && typeof value === 'object' && typeof value.toJSON === 'function') value = value.toJSON(key);
  else if (typeof value === 'bigint') throw new TypeError('Do not know how to serialize a BigInt');
  if (value instanceof Number || value instanceof String || value instanceof Boolean) value = value.valueOf();
  switch (typeof value) {
    case 'number': return Number.isFinite(value) ? (value === 0 ? 0 : value) : null;
    case 'undefined': case 'function': case 'symbol': return ABSENT;
    default: return value;
  }
}

const isRecord = (v: any): v is Record<string, any> => !!v && typeof v === 'object' && !Array.isArray(v);
const byCodeUnits = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

/** A JSON copy of a live value (`clonePatchValue`). */
const clone = (value: any) => (value === undefined ? undefined : JSON.parse(JSON.stringify(value)));

const isEnumerable = Object.prototype.propertyIsEnumerable;

/** The JSON elements of a live array. */
function elements(arr: readonly any[]): any[] {
  const out = new Array(arr.length);
  for (let i = 0; i < arr.length; i++) {
    const v = jsonValue(arr[i], String(i));
    out[i] = v === ABSENT ? null : v;
  }
  return out;
}

/**
 * Operations turning `before` (plain JSON) into the JSON form of the live
 * value `after`, at `path`.
 */
export function diffJsonOps(before: any, after: any, path: readonly PathPart[] = [], out: PatchOp[] = [], memo?: JsonMemo): PatchOp[] {
  const value = jsonValue(after, path.length ? String(path[path.length - 1]) : '');
  if (value === ABSENT) {
    if (before !== undefined) out.push({ path: path.slice(), exists: true });
    return out;
  }
  walk(before, value, path.slice(), out, memo);
  return out;
}

function walk(before: any, after: any, path: PathPart[], out: PatchOp[], memo?: JsonMemo): void {
  if (Object.is(before, after)) return;
  if (Array.isArray(before) && Array.isArray(after)) return walkArrays(before, elements(after), path, out, memo);
  if (isRecord(before) && isRecord(after)) {
    let segments: [string, number, number][] | undefined;
    const member = (key: string, inBefore: boolean, value: any) => {
      const start = out.length;
      path.push(key);
      if (inBefore && value !== ABSENT) walk(before[key], value, path, out, memo);
      else if (value !== ABSENT) out.push({ path: path.slice(), exists: true, value: clone(value) });
      else out.push({ path: path.slice(), exists: false });
      path.pop();
      if (out.length > start) (segments ??= []).push([key, start, out.length]);
    };
    // A live member is present when it is enumerable and has a JSON form.
    const liveValue = (key: string) => (isEnumerable.call(after, key) ? jsonValue(after[key], key) : ABSENT);
    for (const key of Object.keys(before)) member(key, true, liveValue(key));
    for (const key of Object.keys(after)) {
      if (Object.hasOwn(before, key)) continue;
      const value = jsonValue(after[key], key);
      if (value !== ABSENT) member(key, false, value);
    }
    if (segments && segments.length > 1) {
      const base = segments[0]![1];
      const sorted = [...segments].sort((a, b) => byCodeUnits(a[0], b[0]));
      if (sorted.some((segment, index) => segment !== segments![index])) {
        const ops = out.slice(base);
        out.length = base;
        for (const [, start, end] of sorted) for (let i = start; i < end; i++) out.push(ops[i - base]!);
      }
    }
    return;
  }
  out.push({ path: path.slice(), exists: true, value: clone(after) });
}

const idOf = (v: any): string | undefined => (isRecord(v) && typeof v.id === 'string' ? v.id : undefined);

/**
 * Whether the live value `after` serializes exactly to the plain JSON
 * `before` (as `walk` would find no operation), without building operations:
 * the fast test for the retained elements of an id-keyed array, which are
 * almost always unchanged after a reorder, insert, or delete.
 */
export function sameJson(before: any, after: any): boolean {
  if (before === after) return true;
  return plainPrototypes() ? fastSame(before, after) : slowSame(before, after);
}

const OBJECT_PROTO = Object.prototype;
const ARRAY_PROTO = Array.prototype;

/** Whether the shared prototypes are untouched: no `toJSON`, nothing enumerable. */
function plainPrototypes(): boolean {
  if ((OBJECT_PROTO as any).toJSON !== undefined || (ARRAY_PROTO as any).toJSON !== undefined) return false;
  for (const _ in OBJECT_PROTO) return false;
  return true;
}

/**
 * `slowSame` for the shapes a document is made of (plain objects and arrays,
 * whatever their members), without allocating: `for…in` reads exactly the
 * enumerable own string keys of an object whose prototype is
 * `Object.prototype` or `null` (checked by `plainPrototypes`), and the shadow
 * is plain JSON, where a missing member reads `undefined` or an inherited
 * function, neither of which equals a JSON value. Values with other
 * prototypes (primitive wrappers, dates, class instances, array-likes) take
 * `slowSame`, and so does every difference found on a value with a `toJSON`
 * of its own. Not seen: a non-enumerable own `toJSON` on a plain object that
 * otherwise equals the shadow (no writer defines one; the auditor compares
 * serializations and would report it); looking `toJSON` up on every object
 * would cost a quarter of the time. On the 884-layer project's layers
 * (94,824 values) this takes 2.4 ms where `slowSame` takes 7.4 ms and
 * `JSON.stringify` 3.1 ms (Node, loaded machine).
 */
function fastSame(b: any, a: any): boolean {
  if (b === a) return true;
  if (typeof a !== 'object') return typeof a === 'number' && !Number.isFinite(a) && b === null;
  if (a === null) return false;
  const proto = Object.getPrototypeOf(a);
  const array = proto === ARRAY_PROTO && Array.isArray(a);
  if (!array && proto !== OBJECT_PROTO && proto !== null) return slowSame(b, a);
  if (b === null || typeof b !== 'object' || Array.isArray(b) !== array) return differs(b, a);
  if (array) {
    if (b.length !== a.length) return differs(b, a);
    for (let i = 0; i < a.length; i++) {
      const v = a[i];
      if (v === undefined || typeof v === 'function' || typeof v === 'symbol') { if (b[i] !== null) return differs(b, a); }
      else if (!fastSame(b[i], v)) return differs(b, a);
    }
    return true;
  }
  let n = 0;
  for (const key in a) {
    const v = a[key];
    if (v === undefined || typeof v === 'symbol') continue;
    if (typeof v === 'function') {
      if (key === 'toJSON') return slowSame(b, a);
      continue;
    }
    const bv = b[key];
    // Missing from the shadow: a difference, unless the live member has no
    // JSON form after all (a `toJSON` answering undefined); the slow path knows.
    if (bv === undefined) return slowSame(b, a);
    if (!fastSame(bv, v)) return differs(b, a);
    n++;
  }
  for (const _ in b) n--;
  return n === 0 || differs(b, a);
}

/** A difference the fast path found, unless the live value's own `toJSON` replaces it. */
const differs = (b: any, a: any): boolean => a.toJSON !== undefined && slowSame(b, a);

function slowSame(before: any, after: any): boolean {
  if (before === after) return true;
  switch (typeof after) {
    case 'number': return Number.isFinite(after) ? false /* === failed; 0 and -0 are === */ : before === null;
    case 'string': case 'boolean': return false;
    case 'object': break;
    default: return false;
  }
  if (after === null) return false;
  if (typeof after.toJSON === 'function' || after instanceof Number || after instanceof String || after instanceof Boolean) {
    const value = jsonValue(after, '');
    return value !== ABSENT && value !== after && sameJson(before, value);
  }
  if (before === null || typeof before !== 'object') return false;
  if (Array.isArray(after)) {
    if (!Array.isArray(before) || before.length !== after.length) return false;
    for (let i = 0; i < after.length; i++) {
      const v = after[i];
      if (v === undefined || typeof v === 'function' || typeof v === 'symbol') { if (before[i] !== null) return false; }
      else if (!sameJson(before[i], v)) return false;
    }
    return true;
  }
  if (Array.isArray(before)) return false;
  const keys = Object.keys(after);
  // Plain JSON never holds undefined, so a missing member reads undefined
  // (functions inherited from Object.prototype are not JSON either).
  if (keys.length === Object.keys(before).length) {
    let i = 0;
    for (; i < keys.length; i++) {
      const key = keys[i]!;
      const v = after[key];
      if (v === undefined || typeof v === 'function' || typeof v === 'symbol') break;
      const b = before[key];
      if (b === undefined || typeof b === 'function' || !Object.hasOwn(before, key)) return false;
      if (!sameJson(b, v)) return false;
    }
    if (i === keys.length) return true;
  }
  // Members without a JSON form on the live side: count what JSON keeps.
  let count = 0;
  for (const key of keys) {
    const v = after[key];
    if (v === undefined || typeof v === 'function' || typeof v === 'symbol') continue;
    if (typeof v === 'object' && v !== null && typeof v.toJSON === 'function' && jsonValue(v, key) === ABSENT) continue;
    if (!Object.hasOwn(before, key) || !sameJson(before[key], v)) return false;
    count++;
  }
  return count === Object.keys(before).length;
}

function walkArrays(before: readonly any[], after: readonly any[], path: PathPart[], out: PatchOp[], memo?: JsonMemo): void {
  const same = memo ? (b: any, a: any) => memo.same(b, a) : sameJson;
  const identityOrderChanged = before.length === after.length && before.some((item, index) => {
    const a = idOf(item), b = idOf(after[index]);
    return a !== undefined && b !== undefined && a !== b;
  });
  if (before.length === after.length && !identityOrderChanged) {
    for (let index = 0; index < before.length; index++) {
      if (same(before[index], after[index])) continue;
      path.push(index);
      walk(before[index], after[index], path, out, memo);
      path.pop();
    }
    return;
  }
  const replace = () => { out.push({ path: path.slice(), exists: true, value: clone(after) }); };
  if (!isIdKeyed(before) || !isIdKeyed(after)) return replace();
  const oldIndex = new Map<string, number>(before.map((item, index) => [item.id, index]));
  const newIndex = new Map<string, number>(after.map((item, index) => [item.id, index]));
  if (!before.some((item) => newIndex.has(item.id))) return replace();
  for (const item of before) if (!newIndex.has(item.id)) out.push({ op: 'delete', path: path.slice(), id: item.id });
  const survivorsOld = before.filter((item) => newIndex.has(item.id)).map((item) => item.id as string);
  const survivorsNew = after.filter((item) => oldIndex.has(item.id)).map((item) => item.id as string);
  if (survivorsOld.some((id, index) => id !== survivorsNew[index])) out.push({ op: 'order', path: path.slice(), ids: survivorsNew });
  after.forEach((item, index) => {
    if (!oldIndex.has(item.id)) out.push({ op: 'insert', path: path.slice(), index, value: clone(item) });
  });
  after.forEach((item, index) => {
    const from = oldIndex.get(item.id);
    if (from === undefined || same(before[from], item)) return;
    path.push(index);
    walk(before[from], item, path, out, memo);
    path.pop();
  });
}

/**
 * Remembers, per element of the shadow's id-keyed arrays, the JSON-visible
 * content of the live value it was last found equal to (`sameJson`), as one
 * flat list: keys and primitive values in the order `for…in` and indices visit
 * them, with markers for objects and arrays. While that shadow element is
 * unchanged, a live value whose walk reproduces the list serializes to the same
 * JSON, so it is unchanged without reading the shadow. A `touch()` without a
 * path compares every layer of the open composition; this reads only the live
 * side, in one pass, without keyed lookups on the shadow.
 *
 * The owner must call `forgetAlong` for every operation before applying it to
 * the shadow, which mutates shadow objects in place. Anything the list cannot
 * vouch for falls back to `sameJson`: values with a `toJSON` member, other
 * prototypes, `NaN`, and changed prototypes (`plainPrototypes`).
 */
export class JsonMemo {
  private seen = new WeakMap<object, unknown[]>();

  /** `sameJson(before, after)`, remembering an equal element's content. */
  same(before: any, after: any): boolean {
    if (before === after) return true;
    const plain = plainPrototypes();
    if (plain && before !== null && typeof before === 'object') {
      const flat = this.seen.get(before);
      if (flat) {
        cursor = 0;
        if (matches(after, flat) && cursor === flat.length) return true;
        this.seen.delete(before);
      }
    }
    if (!sameJson(before, after)) return false;
    if (plain && before !== null && typeof before === 'object') {
      const flat: unknown[] = [];
      if (flatten(after, flat)) this.seen.set(before, flat);
    }
    return true;
  }

  /** Forgets every shadow object an operation at `path` may change. */
  forgetAlong(root: any, path: readonly PathPart[]): void {
    let node = root;
    for (let i = 0; ; i++) {
      if (node === null || typeof node !== 'object') return;
      this.seen.delete(node);
      if (i === path.length) return;
      node = node[path[i] as any];
    }
  }
}

const OBJECT_START = Symbol('object'), ARRAY_START = Symbol('array'), OBJECT_END = Symbol('end');

/** The live value's JSON-visible content into `out`; false when the list cannot vouch for it. */
function flatten(value: any, out: unknown[]): boolean {
  const proto = Object.getPrototypeOf(value);
  if (proto === ARRAY_PROTO && Array.isArray(value)) {
    out.push(ARRAY_START, value.length);
    for (let i = 0; i < value.length; i++) {
      const v = value[i];
      if (v !== null && typeof v === 'object') { if (!flatten(v, out)) return false; }
      else out.push(v);
    }
    return true;
  }
  if (proto !== OBJECT_PROTO && proto !== null) return false;
  out.push(OBJECT_START);
  for (const key in value) {
    if (key === 'toJSON') return false;
    const v = value[key];
    out.push(key);
    if (v !== null && typeof v === 'object') { if (!flatten(v, out)) return false; }
    else out.push(v);
  }
  out.push(OBJECT_END);
  return true;
}

/** Read position in the list `matches` walks (one walk at a time: synchronous). */
let cursor = 0;

/** Whether the live value walks exactly as the remembered list from `cursor`. */
function matches(value: any, flat: readonly unknown[]): boolean {
  if (value === null || typeof value !== 'object') return false;
  const proto = Object.getPrototypeOf(value);
  if (proto === ARRAY_PROTO && Array.isArray(value)) {
    if (flat[cursor++] !== ARRAY_START || flat[cursor++] !== value.length) return false;
    for (let i = 0; i < value.length; i++) {
      const v = value[i];
      if (v !== null && typeof v === 'object') { if (!matches(v, flat)) return false; }
      else if (flat[cursor++] !== v) return false;
    }
    return true;
  }
  if ((proto !== OBJECT_PROTO && proto !== null) || flat[cursor++] !== OBJECT_START) return false;
  for (const key in value) {
    if (flat[cursor++] !== key) return false;
    const v = value[key];
    if (v !== null && typeof v === 'object') { if (!matches(v, flat)) return false; }
    else if (flat[cursor++] !== v) return false;
  }
  return flat[cursor++] === OBJECT_END;
}
