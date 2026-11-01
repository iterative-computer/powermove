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
export function diff(before: any, after: any, path: Path = []): Patch[] {
  if (equal(before, after)) return [];
  if (
    !before ||
    !after ||
    typeof before !== "object" ||
    typeof after !== "object" ||
    Array.isArray(before) !== Array.isArray(after) ||
    (Array.isArray(before) &&
      (before.length !== after.length ||
        before.some((v, i) => identity(v) !== identity(after[i]))))
  )
    return [{ path, exists: true, value: copy(after) }];
  const changes: Patch[] = [];
  for (const key of new Set([...Object.keys(before), ...Object.keys(after)])) {
    const nextPath = [...path, Array.isArray(after) ? Number(key) : key];
    if (!Object.hasOwn(after, key))
      changes.push({ path: nextPath, exists: false });
    else if (!Object.hasOwn(before, key))
      changes.push({ path: nextPath, exists: true, value: copy(after[key]) });
    else changes.push(...diff(before[key], after[key], nextPath));
  }
  return changes;
}
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
export class DocumentMirror {
  document: Data;
  private base: Data;
  constructor(document: Data) {
    this.base = copy(document);
    this.document = copy(document);
  }
  pending(): Patch[] {
    copy(this.document);
    return diff(this.base, this.document);
  }
  snapshot(): Data {
    return copy(this.document);
  }
  restore(snapshot: Data): void {
    reconcile(this.document, snapshot);
  }
  replace(document: Data): void {
    this.document = document;
  }
  accept(document: Data): void {
    this.base = copy(document);
    reconcile(this.document, document);
  }
  /** Native writes received before our flush win only where we did not write. */
  receive(document: Data): void {
    const previous = this.base;
    try {
      const local = this.pending();
      const authoritative = copy(document);
      reconcile(
        this.document,
        patch(authoritative, rebase(local, previous, authoritative)),
      );
      this.base = authoritative;
    } catch (error) {
      this.base = copy(document);
      reconcile(this.document, document);
      throw error;
    }
  }
}
