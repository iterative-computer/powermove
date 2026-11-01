export const primitives = [
  "div",
  "text",
  "img",
  "input",
  "textarea",
  "button",
  "scroll",
  "virtual-list",
  "anchored",
  "path",
  "canvas",
  "frame",
  "markdown",
  "code",
  "Glass",
  "material",
  "webview",
] as const;
export type Primitive = (typeof primitives)[number];
export type Json =
  | null
  | boolean
  | number
  | string
  | Json[]
  | { [key: string]: Json };
export type Props = Record<string, Json>;
export interface ListenerFlags {
  capture: boolean;
  bubble: boolean;
  preventDefault: boolean;
  keys: string[];
}
export type Op =
  | { op: "create"; id: number; type: Primitive }
  | { op: "insertBefore"; parent: number; id: number; before: number | null }
  | { op: "remove"; id: number }
  | { op: "setProps"; id: number; patch: Props }
  | { op: "setText"; id: number; text: string }
  | { op: "setListeners"; id: number; flags: Record<string, ListenerFlags> };
export interface Commit {
  baseRevision: number;
  ops: Op[];
}
export interface Ack {
  accepted: boolean;
  revision: number;
  error?: unknown;
}
/** Must queue to the validation worker. A Promise is awaited only in the JS worker. */
export type Transport = (commit: Commit) => Promise<Ack>;
export interface EventPlan {
  revision: number;
  target: number;
  preventDefault: boolean;
  deliveries: { id: number; capture: boolean }[];
}
export interface SurfaceEvent {
  type: string;
  target: number;
  currentTarget: number;
  defaultPrevented: boolean;
  key?: string;
  value?: Json;
  shiftKey?: boolean;
  altKey?: boolean;
  pointerId?: number;
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}
export function json(value: unknown): Json {
  const seen = new Set<object>();
  function visit(v: unknown): Json {
    if (v === null || typeof v === "string" || typeof v === "boolean") return v;
    if (typeof v === "number" && Number.isFinite(v)) return v;
    if (typeof v !== "object" || v === null)
      throw new TypeError("Surface props must be finite JSON data");
    if (seen.has(v)) throw new TypeError("Cyclic surface prop");
    seen.add(v);
    let result: Json;
    if (Array.isArray(v)) result = v.map(visit);
    else {
      if (
        Object.getPrototypeOf(v) !== Object.prototype &&
        Object.getPrototypeOf(v) !== null
      )
        throw new TypeError("Surface props must be plain records");
      const record: Record<string, Json> = Object.create(null);
      for (const [key, descriptor] of Object.entries(
        Object.getOwnPropertyDescriptors(v),
      )) {
        if (!descriptor.enumerable) continue;
        if (!("value" in descriptor))
          throw new TypeError("Surface props cannot contain accessors");
        if (descriptor.value !== undefined)
          record[key] = visit(descriptor.value);
      }
      result = record;
    }
    seen.delete(v);
    return result;
  }
  return visit(value);
}
