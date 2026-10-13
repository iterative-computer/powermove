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
export function diffJsonOps(before: any, after: any, path: readonly PathPart[] = [], out: PatchOp[] = []): PatchOp[] {
  const value = jsonValue(after, path.length ? String(path[path.length - 1]) : '');
  if (value === ABSENT) {
    if (before !== undefined) out.push({ path: path.slice(), exists: true });
    return out;
  }
  walk(before, value, path.slice(), out);
  return out;
}

function walk(before: any, after: any, path: PathPart[], out: PatchOp[]): void {
  if (Object.is(before, after)) return;
  if (Array.isArray(before) && Array.isArray(after)) return walkArrays(before, elements(after), path, out);
  if (isRecord(before) && isRecord(after)) {
    let segments: [string, number, number][] | undefined;
    const member = (key: string, inBefore: boolean, value: any) => {
      const start = out.length;
      path.push(key);
      if (inBefore && value !== ABSENT) walk(before[key], value, path, out);
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

function walkArrays(before: readonly any[], after: readonly any[], path: PathPart[], out: PatchOp[]): void {
  const identityOrderChanged = before.length === after.length && before.some((item, index) => {
    const a = idOf(item), b = idOf(after[index]);
    return a !== undefined && b !== undefined && a !== b;
  });
  if (before.length === after.length && !identityOrderChanged) {
    for (let index = 0; index < before.length; index++) {
      path.push(index);
      walk(before[index], after[index], path, out);
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
    if (from === undefined) return;
    path.push(index);
    walk(before[from], item, path, out);
    path.pop();
  });
}
