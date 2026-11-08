import type { Data, Patch, Path } from "./types";

const forbidden = new Set(["__proto__", "constructor", "prototype"]);
export function validatePath(path: Path): void {
  if (
    !path.every((p) =>
      typeof p === "string"
        ? !forbidden.has(p)
        : Number.isSafeInteger(p) && p >= 0,
    )
  )
    throw new TypeError("Invalid document path");
}
/** Reject lossy JSON rather than silently deleting authored values. */
export function copy<T>(value: T, omitUndefinedRecordFields = false): T {
  const seen = new Set<object>();
  function visit(v: any): any {
    if (v === null || typeof v === "string" || typeof v === "boolean") return v;
    if (typeof v === "number" && Number.isFinite(v)) return v;
    if (typeof v !== "object" || seen.has(v))
      throw new TypeError("Document must contain finite, acyclic JSON values");
    if (
      !Array.isArray(v) &&
      Object.getPrototypeOf(v) !== Object.prototype &&
      Object.getPrototypeOf(v) !== null
    )
      throw new TypeError("Document must contain plain records");
    seen.add(v);
    const result: any = Array.isArray(v) ? [] : {};
    for (const key of Object.keys(v)) {
      if (forbidden.has(key)) throw new TypeError("Invalid document key");
      const d = Object.getOwnPropertyDescriptor(v, key)!;
      if (!("value" in d))
        throw new TypeError("Document accessors are not supported");
      // Legacy workspace/layout helpers deliberately assign absent optional fields.
      // Documents keep strict validation; only workspace persistence opts into this.
      if (
        omitUndefinedRecordFields &&
        !Array.isArray(v) &&
        d.value === undefined
      )
        continue;
      result[key] = visit(d.value);
    }
    if (Array.isArray(v) && Object.keys(v).length !== v.length)
      throw new TypeError(
        "Sparse or decorated document arrays are not supported",
      );
    seen.delete(v);
    return result;
  }
  return visit(value);
}
const identity = (value: any): string | undefined =>
  value && typeof value === "object"
    ? typeof value.id === "string"
      ? `id:${value.id}`
      : typeof value.i === "string"
        ? `i:${value.i}`
        : undefined
    : undefined;

export const equal = (a: any, b: any): boolean =>
  JSON.stringify(a) === JSON.stringify(b);
/** One walk over both trees: a boundary diff stays linear in the document. */
export function diff(before: any, after: any, path: Path = []): Patch[] {
  const changes: Patch[] = [];
  walk(before, after, path, changes);
  return changes;
}
function walk(before: any, after: any, path: Path, changes: Patch[]): void {
  if (before === after) return;
  if (
    !before ||
    !after ||
    typeof before !== "object" ||
    typeof after !== "object" ||
    Array.isArray(before) !== Array.isArray(after) ||
    (Array.isArray(before) &&
      (before.length !== after.length ||
        before.some((v, i) => identity(v) !== identity(after[i]))))
  ) {
    if (!sameLeaf(before, after))
      changes.push({ path: [...path], exists: true, value: copy(after) });
    return;
  }
  if (Array.isArray(after)) {
    if (Object.keys(after).length !== after.length)
      throw new TypeError(
        "Sparse or decorated document arrays are not supported",
      );
    for (let index = 0; index < after.length; index++) {
      path.push(index);
      walk(before[index], after[index], path, changes);
      path.pop();
    }
    return;
  }
  for (const key of Object.keys(before))
    if (!Object.hasOwn(after, key))
      changes.push({ path: [...path, key], exists: false });
  for (const key of Object.keys(after)) {
    if (forbidden.has(key)) throw new TypeError("Invalid document key");
    const descriptor = Object.getOwnPropertyDescriptor(after, key)!;
    if (!("value" in descriptor))
      throw new TypeError("Document accessors are not supported");
    if (!Object.hasOwn(before, key)) {
      changes.push({
        path: [...path, key],
        exists: true,
        value: copy(descriptor.value),
      });
      continue;
    }
    path.push(key);
    walk(before[key], descriptor.value, path, changes);
    path.pop();
  }
}
/** Distinct non-identical values: only NaN/undefined forms can compare equal. */
const sameLeaf = (a: any, b: any): boolean =>
  JSON.stringify(a) === JSON.stringify(b);
export function patch(root: any, ops: Patch[]): any {
  let next = copy(root);
  for (const op of ops) {
    validatePath(op.path);
    if (!op.path.length) {
      if (!op.exists) throw new TypeError("Cannot delete document root");
      next = copy(op.value);
      continue;
    }
    let parent = next;
    for (const p of op.path.slice(0, -1)) {
      if (!parent || !Object.hasOwn(parent, p))
        throw new TypeError("Document path no longer exists");
      parent = parent[p];
    }
    const key = op.path.at(-1)!;
    if (!parent || typeof parent !== "object")
      throw new TypeError("Document path no longer exists");
    if (op.exists) parent[key] = copy(op.value);
    else delete parent[key];
  }
  return next;
}
/** Preserve retained layer/property references, including array reorder by id. */
export function reconcile(target: any, source: any): any {
  if (
    !target ||
    !source ||
    typeof target !== "object" ||
    typeof source !== "object" ||
    Array.isArray(target) !== Array.isArray(source)
  )
    return copy(source);
  if (Array.isArray(target)) {
    const old = target.slice(),
      ids = new Map(
        old
          .filter((v) => identity(v) !== undefined)
          .map((v) => [identity(v), v]),
      );
    const values = source.map((v: any, i: number) =>
      reconcile(identity(v) !== undefined ? ids.get(identity(v)) : old[i], v),
    );
    target.splice(0, target.length, ...values);
  } else {
    for (const key of Object.keys(target))
      if (!Object.hasOwn(source, key)) delete target[key];
    for (const key of Object.keys(source)) {
      const descriptor = Object.getOwnPropertyDescriptor(target, key);
      const current =
        descriptor && "value" in descriptor ? descriptor.value : undefined;
      const next = reconcile(current, source[key]);
      if (current === next && descriptor && "value" in descriptor) continue;
      if (descriptor && !descriptor.writable && descriptor.configurable)
        Object.defineProperty(target, key, { ...descriptor, value: next });
      else target[key] = next;
    }
  }
  return target;
}
/** Translate retained record paths when a native publication reorders an id list. */
export function rebase(ops: Patch[], before: Data, after: Data): Patch[] {
  return ops.map((op) => {
    let old: any = before,
      next: any = after;
    const path = op.path.map((part) => {
      let translated = part;
      if (
        typeof part === "number" &&
        Array.isArray(old) &&
        Array.isArray(next)
      ) {
        const id = identity(old[part]);
        if (id !== undefined) {
          const index = next.findIndex((value) => identity(value) === id);
          if (index < 0)
            throw new TypeError("Retained document record no longer exists");
          translated = index;
        }
      }
      old = old?.[part];
      next = next?.[translated];
      return translated;
    });
    return { ...op, path };
  });
}
/** A raw record or array of the live tree, and where it was last seen. */
type Link = { parent: Data; key: string | number };
const raws = new WeakMap<object, object>();
/** Whether `value` is a view of a record in the project (not a copy). */
export function isProjectView(value: unknown): boolean {
  return typeof value === "object" && value !== null && raws.has(value);
}
/**
 * `value` with project views replaced by the records they show, for natives
 * that refuse proxies (structured clone). Containers are copied only on the
 * way to a view; a view's own subtree holds no views.
 */
export function unwrapViews(value: unknown): unknown {
  const seen = new Map<object, unknown>();
  const visit = (v: any): any => {
    if (v === null || typeof v !== "object") return v;
    const raw = raws.get(v);
    if (raw) return raw;
    if (seen.has(v)) return seen.get(v);
    const proto = Object.getPrototypeOf(v);
    if (Array.isArray(v)) {
      const out: any[] = [];
      seen.set(v, out);
      let changed = false;
      for (let i = 0; i < v.length; i++) {
        out[i] = visit(v[i]);
        changed ||= out[i] !== v[i];
      }
      if (!changed) seen.set(v, v);
      return changed ? out : v;
    }
    if (v instanceof Map) {
      const out = new Map();
      seen.set(v, out);
      let changed = false;
      for (const [k, item] of v) {
        const key = visit(k),
          next = visit(item);
        changed ||= key !== k || next !== item;
        out.set(key, next);
      }
      if (!changed) seen.set(v, v);
      return changed ? out : v;
    }
    if (v instanceof Set) {
      const out = new Set();
      seen.set(v, out);
      let changed = false;
      for (const item of v) {
        const next = visit(item);
        changed ||= next !== item;
        out.add(next);
      }
      if (!changed) seen.set(v, v);
      return changed ? out : v;
    }
    if (proto !== Object.prototype && proto !== null) return v;
    const out: Data = Object.create(proto);
    seen.set(v, out);
    let changed = false;
    for (const key of Object.keys(v)) {
      out[key] = visit(v[key]);
      changed ||= out[key] !== v[key];
    }
    if (!changed) seen.set(v, v);
    return changed ? out : v;
  };
  return visit(value);
}
/** Lets the realm's `structuredClone` take project records as Electron did. */
export function cloneAcceptingViews(
  clone: (value: any, options?: any) => any,
): (value: any, options?: any) => any {
  return (value, options) => clone(unwrapViews(value), options);
}
const slots = (value: Data): unknown[] => {
  const out: unknown[] = [];
  for (const key of Object.keys(value)) out.push(key, value[key]);
  return out;
};
/**
 * The live project and its last acknowledged copy. Extensions see `document`,
 * a view that records which records they write, so a boundary costs what was
 * written rather than the size of the project: Electron let them mutate the
 * project in place, and a pro project holds millions of keyframes.
 */
export class DocumentMirror {
  private root: Data;
  private base: Data;
  private dirty = new Set<object>();
  private links = new WeakMap<object, Link>();
  private views = new WeakMap<object, any>();
  /** Records an extension built and put in the tree. It keeps the raw
   * reference and may write through it later, as in Electron, so each is
   * compared slot by slot with how it was last sent. */
  private adopted = new Map<object, unknown[]>();
  /** An extension replaced the whole project and holds it: every boundary
   * compares the whole tree, as before views. */
  private rawRoot = false;
  /** `owned`: the caller hands `document` over (a parsed publication), so it
   * becomes the live tree rather than being copied a second time. */
  constructor(document: Data, { owned = false } = {}) {
    this.base = copy(document);
    this.root = owned ? document : copy(document);
  }
  get document(): Data {
    return this.rawRoot ? this.root : this.view(this.root);
  }
  private link(raw: object, parent: Data, key: string | number): void {
    const link = this.links.get(raw);
    if (!link || link.parent !== parent || link.key !== key)
      this.links.set(raw, { parent, key });
  }
  /** A value entering the tree: views become the records they show, and the
   * extension's own records are adopted, keeping their identity. */
  private adopt(value: any, parent: Data, key: string | number): any {
    if (value === null || typeof value !== "object") return value;
    const raw = raws.get(value);
    if (raw) {
      this.link(raw, parent, key);
      return raw;
    }
    if (this.links.has(value) && !this.adopted.has(value)) {
      this.link(value, parent, key);
      return value;
    }
    this.link(value, parent, key);
    if (this.adopted.has(value)) return value;
    this.adopted.set(value, []);
    this.rescan(value);
    return value;
  }
  private rescan(value: Data): void {
    const array = Array.isArray(value);
    for (const key of Object.keys(value)) {
      const d = Object.getOwnPropertyDescriptor(value, key);
      if (!d || !("value" in d) || !d.value || typeof d.value !== "object")
        continue;
      const next = this.adopt(d.value, value, array ? Number(key) : key);
      if (next !== d.value && d.writable) value[key] = next;
    }
    this.adopted.set(value, slots(value));
  }
  private view(raw: any): any {
    let view = this.views.get(raw);
    if (view) return view;
    const mirror = this;
    view = new Proxy(raw, {
      get(target, key) {
        const value = target[key];
        if (value === null || typeof value !== "object") return value;
        const link = mirror.links.get(value);
        if (!link || link.parent !== target)
          mirror.links.set(value, {
            parent: target,
            key: Array.isArray(target) ? Number(key) : (key as string),
          });
        // The extension's own records read back as themselves, as in
        // Electron: they are tracked slot by slot, not through a view.
        return mirror.adopted.has(value) ? value : mirror.view(value);
      },
      set(target, key, value) {
        const slot = Array.isArray(target) ? Number(key) : (key as string);
        target[key] = mirror.adopt(value, target, slot);
        mirror.dirty.add(target);
        return true;
      },
      defineProperty(target, key, descriptor) {
        const slot = Array.isArray(target) ? Number(key) : (key as string);
        if ("value" in descriptor)
          descriptor = {
            ...descriptor,
            value: mirror.adopt(descriptor.value, target, slot),
          };
        mirror.dirty.add(target);
        return Reflect.defineProperty(target, key, descriptor);
      },
      deleteProperty(target, key) {
        mirror.dirty.add(target);
        return Reflect.deleteProperty(target, key);
      },
    });
    this.views.set(raw, view);
    raws.set(view, raw);
    return view;
  }
  /** Where a written record sits now; undefined once it left the tree. */
  private pathOf(raw: object): Path | undefined {
    const path: Path = [];
    for (let at: any = raw; at !== this.root; ) {
      const link = this.links.get(at);
      if (!link) return undefined;
      const parent = link.parent;
      if (parent[link.key] !== at) {
        const key = Array.isArray(parent)
          ? parent.indexOf(at)
          : Object.keys(parent).find((k) => parent[k] === at);
        if (key === undefined || key === -1) return undefined;
        link.key = key;
      }
      path.push(link.key);
      at = parent;
    }
    return path.reverse();
  }
  /** Written subtrees, none inside another. */
  private written(): Path[] {
    if (this.rawRoot) return [[]];
    const paths: Path[] = [];
    for (const raw of this.dirty) {
      const path = this.pathOf(raw);
      if (path) paths.push(path);
    }
    for (const [raw, last] of this.adopted) {
      const path = this.pathOf(raw);
      if (!path) {
        this.adopted.delete(raw);
        continue;
      }
      const now = slots(raw);
      if (now.length !== last.length || now.some((v, i) => v !== last[i])) {
        this.rescan(raw);
        this.adopted.set(raw, last);
        paths.push(path);
      }
    }
    return outermost(paths);
  }
  /** The adopted records as they are now are what was last sent. */
  private sent(): void {
    for (const raw of this.adopted.keys()) this.adopted.set(raw, slots(raw));
  }
  /** Every changed value is validated by `copy`; unchanged values equal `base`. */
  pending(): Patch[] {
    const changes: Patch[] = [];
    for (const path of this.written()) {
      const before = at(this.base, path),
        after = at(this.root, path);
      if (before.found && after.found)
        changes.push(...diff(before.value, after.value, [...path]));
      else if (after.found)
        changes.push({ path, exists: true, value: copy(after.value) });
      else if (before.found) changes.push({ path, exists: false });
    }
    return changes;
  }
  /** The pending writes, now in flight: later writes start a new set. */
  take(): Patch[] {
    const changes = this.pending();
    this.dirty.clear();
    this.sent();
    return changes;
  }
  /** Local writes not yet sent, including any through adopted records. */
  get writing(): boolean {
    return this.dirty.size > 0 || this.written().length > 0;
  }
  restore(snapshot: Data): void {
    reconcile(this.root, snapshot);
    this.dirty.add(this.root);
  }
  replace(document: Data): void {
    const raw = raws.get(document);
    this.root = raw ?? document;
    this.rawRoot ||= !raw;
    this.dirty.add(this.root);
  }
  accept(document: Data): void {
    this.base = copy(document);
    reconcile(this.root, document);
    this.dirty.clear();
    this.sent();
  }
  /** Drops local writes, and those of `sent`, where they were made. */
  discard(sent: Patch[] = []): void {
    this.settle(
      [...this.written(), ...sent.map((op) => op.path)].map((path) => ({
        path,
      })),
    );
    this.dirty.clear();
    this.sent();
  }
  /** Brings the live tree back to `base` wherever `ops` reached it, keeping
   * retained references. Whole-document publications reconcile everything. */
  settle(ops: { path: Path }[]): void {
    this.reconcileAt(outermost(ops.map((op) => op.path)));
    this.sent();
  }
  private reconcileAt(paths: Path[]): void {
    for (const path of paths) {
      if (!path.length) {
        reconcile(this.root, this.base);
        return;
      }
      const parentPath = path.slice(0, -1),
        key = path.at(-1)!;
      const parent = at(this.root, parentPath),
        source = at(this.base, parentPath);
      if (
        !parent.found ||
        !source.found ||
        !parent.value ||
        typeof parent.value !== "object" ||
        !source.value ||
        typeof source.value !== "object" ||
        Array.isArray(parent.value) !== Array.isArray(source.value)
      ) {
        reconcile(this.root, this.base);
        return;
      }
      if (Array.isArray(source.value)) {
        // One record replaced in place; anything structural walks the array.
        const same =
          typeof key === "number" &&
          parent.value.length === source.value.length &&
          identity(parent.value[key]) === identity(source.value[key]);
        if (!same) {
          reconcile(parent.value, source.value);
          continue;
        }
      }
      if (Object.hasOwn(source.value, key)) {
        const current = parent.value[key];
        const next = reconcile(current, source.value[key]);
        if (next !== current) parent.value[key] = next;
      } else delete parent.value[key];
    }
  }
  /** The last authoritative tree. Callers treat it as read-only. */
  get acknowledged(): Data {
    return this.base;
  }
  setAcknowledged(document: Data): void {
    this.base = copy(document);
  }
  /** Native operations against `base`, without rebuilding the whole tree. */
  applyOps(ops: Data[]): void {
    for (const op of ops) {
      if (!op.path.length) {
        this.base = copy(op.value);
        continue;
      }
      let parent = this.base;
      for (const part of op.op ? op.path : op.path.slice(0, -1))
        parent = parent[part];
      const key = op.path.at(-1)!;
      if (op.op === "order") {
        const items = new Map(parent.map((item: Data) => [item.id, item]));
        parent.splice(
          0,
          parent.length,
          ...op.ids.map((id: string) => items.get(id)),
        );
      } else if (op.op === "insert") parent.splice(op.index, 0, copy(op.value));
      else if (op.op === "delete")
        parent.splice(
          parent.findIndex((item: Data) => item.id === op.id),
          1,
        );
      else if (op.exists) parent[key] = copy(op.value);
      else delete parent[key];
    }
  }
  /** Native writes received before our flush win only where we did not write. */
  receive(document: Data, previous: Data = this.base): void {
    try {
      const local = diff(previous, this.root);
      const authoritative = copy(document);
      reconcile(
        this.root,
        patch(authoritative, rebase(local, previous, authoritative)),
      );
      this.base = authoritative;
    } catch (error) {
      this.base = copy(document);
      reconcile(this.root, document);
      this.dirty.clear();
      throw error;
    }
  }
  /** The live tree itself, for the realm's own whole-tree fallbacks. */
  get raw(): Data {
    return this.root;
  }
}
/** Paths with none of their ancestors in the list, shortest first. */
function outermost(paths: Path[]): Path[] {
  const seen = new Set<string>(),
    kept: Path[] = [];
  for (const path of [...paths].sort((a, b) => a.length - b.length)) {
    let key = "",
      inside = false;
    for (const part of path) {
      if (seen.has(key)) {
        inside = true;
        break;
      }
      key += "\u0000" + typeof part + ":" + part;
    }
    if (inside || seen.has(key)) continue;
    seen.add(key);
    kept.push(path);
  }
  return kept;
}
function at(root: any, path: Path): { found: boolean; value?: any } {
  let value = root;
  for (const part of path) {
    if (!value || typeof value !== "object" || !Object.hasOwn(value, part))
      return { found: false };
    value = value[part];
  }
  return { found: true, value };
}
