/*
 * Id-keyed structural patch operations (docs/rust-engine.md, section 26,
 * amendment 3; contract 5, docs/engine/rfcs/05-patches.md).
 *
 * diffPatches (patch.ts) replaces an array whole when its length or its `id`
 * order changes, so reordering, inserting, or deleting one layer re-sends the
 * whole `layers` array. S5 measured JSON.stringify of an 884-layer array alone
 * at 2.1 ms, over the 2 ms structural budget. These operations send only what
 * moved:
 *
 *   { op: 'order',  path, ids }          permute the array into the order of `ids`
 *   { op: 'insert', path, index, value } splice(index, 0, value); value has a new string id
 *   { op: 'delete', path, id }           remove the element whose id is `id`
 *
 * next to ordinary leaf patches ({ path, exists, value? }). An id-keyed
 * operation requires the array at `path` to be id-keyed: every element a plain
 * object with a string `id`, no two alike. `order` names every element exactly
 * once, `insert` adds an id the array lacks, `delete` names an id it has.
 * Anything else throws IdOpMismatch: the receiver has diverged and resyncs. As
 * with applyPatch, a throwing operation leaves the earlier ones applied.
 *
 * diffOps is diffPatches with id-keyed arrays. Where diffPatches would replace
 * an array whole and both arrays are id-keyed and share at least one id, it
 * emits, in order: a delete for each old id the new array lacks (old order);
 * an order of the surviving ids when their relative order changed; an insert
 * for each new id (ascending final index); then the ops that turn each
 * survivor into its new value, at its final index. Other arrays fall back to
 * diffPatches' leaf patch, so items without ids (or with duplicate ids) still
 * patch correctly. Object members are visited in UTF-16 code unit order (the
 * canonical key order), not insertion order, so the same edit always yields
 * the same operations; engine/crates/pm-model (src/ops.rs, diff_ops) is the
 * same generator in Rust and the cross-language vectors check both emit
 * identical lists.
 */
import { applyPatch, clonePatchValue, type Patch, type PathPart } from './patch';

export type IdOp =
  | { op: 'order'; path: PathPart[]; ids: string[] }
  | { op: 'insert'; path: PathPart[]; index: number; value: any }
  | { op: 'delete'; path: PathPart[]; id: string };

/** One operation of the patch stream: a leaf patch or an id-keyed operation. */
export type PatchOp = Patch | IdOp;

/** The receiver's array does not match an id-keyed operation. */
export class IdOpMismatch extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'IdOpMismatch';
  }
}

export const isIdOp = (op: PatchOp): op is IdOp => typeof (op as IdOp).op === 'string';

const isRecord = (value: any): value is Record<string, any> => !!value && typeof value === 'object' && !Array.isArray(value);

/** The string `id` of a plain object, if it has one. */
const recordId = (value: any): string | undefined => (isRecord(value) && typeof value.id === 'string' ? value.id : undefined);

/** Whether every element is a plain object with a string `id`, no two alike. */
export function isIdKeyed(items: readonly any[]): boolean {
  const seen = new Set<string>();
  for (const item of items) {
    const id = recordId(item);
    if (id === undefined || seen.has(id)) return false;
    seen.add(id);
  }
  return true;
}

// ── Applying ──────────────────────────────────────────────────────────────

function targetArray(root: any, path: PathPart[]): any[] {
  let node = root;
  for (const part of path) {
    if (!node || typeof node !== 'object' || !Object.hasOwn(node, part)) throw new IdOpMismatch(`no member '${String(part)}' on the path`);
    node = node[part as any];
  }
  if (!Array.isArray(node)) throw new IdOpMismatch('the path does not address an array');
  return node;
}

function elementId(item: any): string {
  const id = recordId(item);
  if (id === undefined) throw new IdOpMismatch('an element of the array has no string id');
  return id;
}

function applyIdOp(root: any, op: IdOp): void {
  if (root === undefined) throw new IdOpMismatch('the document is undefined');
  const items = targetArray(root, op.path);
  switch (op.op) {
    case 'order': {
      if (op.ids.length !== items.length) throw new IdOpMismatch(`order names ${op.ids.length} ids for ${items.length} elements`);
      const position = new Map<string, number>();
      items.forEach((item, index) => {
        const id = elementId(item);
        if (position.has(id)) throw new IdOpMismatch('the array has duplicate ids');
        position.set(id, index);
      });
      const source = op.ids.map((id) => {
        const index = position.get(id);
        if (index === undefined) throw new IdOpMismatch(`order names unknown or repeated id '${id}'`);
        position.delete(id);
        return index;
      });
      const moved = source.map((index) => items[index]);
      for (let index = 0; index < moved.length; index++) items[index] = moved[index];
      return;
    }
    case 'insert': {
      if (!Number.isInteger(op.index) || op.index < 0 || op.index > items.length) throw new IdOpMismatch(`insert at ${op.index} past the end (${items.length})`);
      const value = clonePatchValue(op.value);
      const id = recordId(value);
      if (id === undefined) throw new IdOpMismatch('an inserted element needs a string id');
      for (const item of items) if (elementId(item) === id) throw new IdOpMismatch(`insert of id '${id}' already present`);
      items.splice(op.index, 0, value);
      return;
    }
    case 'delete': {
      let found = -1;
      items.forEach((item, index) => {
        if (elementId(item) !== op.id) return;
        if (found >= 0) throw new IdOpMismatch('the array has duplicate ids');
        found = index;
      });
      if (found < 0) throw new IdOpMismatch(`delete of unknown id '${op.id}'`);
      items.splice(found, 1);
      return;
    }
    default:
      throw new IdOpMismatch(`unknown operation '${(op as { op: unknown }).op}'`);
  }
}

/**
 * Applies operations in place and returns the (possibly replaced) root, as
 * applyPatch does. Leaf patches keep applyPatch's semantics exactly.
 */
export function applyOps(root: any, ops: readonly PatchOp[]): any {
  let next = root;
  for (const op of ops) {
    if (isIdOp(op)) applyIdOp(next, op);
    else next = applyPatch(next, [op]);
  }
  return next;
}

// ── The generator ─────────────────────────────────────────────────────────

/** UTF-16 code unit order, the default Array.prototype.sort() order. */
const byCodeUnits = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

/**
 * Operations turning `before` into `after`: diffPatches' forward patches with
 * id-keyed arrays (see the header). The undo direction is
 * `diffOps(after, before)`.
 */
export function diffOps(before: any, after: any, path: readonly PathPart[] = [], out: PatchOp[] = []): PatchOp[] {
  walk(before, after, path.slice(), out);
  return out;
}

/**
 * Operations turning the array `before` at `path` into `after`: id-keyed
 * operations when both arrays are id-keyed and share an id, otherwise what
 * diffPatches emits for the pair.
 */
export function diffArrayOps(before: readonly any[], after: readonly any[], path: readonly PathPart[] = [], out: PatchOp[] = []): PatchOp[] {
  walkArrays(before, after, path.slice(), out);
  return out;
}

// `path` is one mutable stack for the whole walk; emitted operations copy it.
// Unchanged subtrees are compared without allocating.

function walk(before: any, after: any, path: PathPart[], out: PatchOp[]): void {
  if (Object.is(before, after)) return;
  if (Array.isArray(before) && Array.isArray(after)) return walkArrays(before, after, path, out);
  if (isRecord(before) && isRecord(after)) {
    // Members are visited in insertion order and their operations are put in
    // code unit order afterwards, so unchanged objects are never sorted.
    let segments: [string, number, number][] | undefined;
    const member = (key: string, inBefore: boolean, inAfter: boolean) => {
      const start = out.length;
      path.push(key);
      if (inBefore && inAfter) walk(before[key], after[key], path, out);
      else if (inAfter) out.push({ path: path.slice(), exists: true, value: clonePatchValue(after[key]) });
      else out.push({ path: path.slice(), exists: false });
      path.pop();
      if (out.length > start) (segments ??= []).push([key, start, out.length]);
    };
    for (const key of Object.keys(before)) member(key, true, Object.hasOwn(after, key));
    for (const key of Object.keys(after)) if (!Object.hasOwn(before, key)) member(key, false, true);
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
  out.push({ path: path.slice(), exists: true, value: clonePatchValue(after) });
}

function walkArrays(before: readonly any[], after: readonly any[], path: PathPart[], out: PatchOp[]): void {
  const identityOrderChanged = before.length === after.length && before.some((item, index) => {
    const a = recordId(item), b = recordId(after[index]);
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
  const replace = () => {
    out.push({ path: path.slice(), exists: true, value: clonePatchValue(after) });
  };
  if (!isIdKeyed(before) || !isIdKeyed(after)) return replace();
  const oldIndex = new Map<string, number>(before.map((item, index) => [item.id, index]));
  const newIndex = new Map<string, number>(after.map((item, index) => [item.id, index]));
  if (!before.some((item) => newIndex.has(item.id))) return replace();
  for (const item of before) if (!newIndex.has(item.id)) out.push({ op: 'delete', path: path.slice(), id: item.id });
  const survivorsOld = before.filter((item) => newIndex.has(item.id)).map((item) => item.id as string);
  const survivorsNew = after.filter((item) => oldIndex.has(item.id)).map((item) => item.id as string);
  if (survivorsOld.some((id, index) => id !== survivorsNew[index])) out.push({ op: 'order', path: path.slice(), ids: survivorsNew });
  after.forEach((item, index) => {
    if (!oldIndex.has(item.id)) out.push({ op: 'insert', path: path.slice(), index, value: clonePatchValue(item) });
  });
  after.forEach((item, index) => {
    const from = oldIndex.get(item.id);
    if (from === undefined) return;
    path.push(index);
    walk(before[from], item, path, out);
    path.pop();
  });
}
