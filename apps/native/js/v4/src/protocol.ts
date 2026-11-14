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
  /** The target's `id` prop when the host planned it; a render that lands
      first does not stale an activation of the same identified element. */
  semantic?: string;
}
/** UTF-16 code-unit offsets (Cocoa/JS), end exclusive. */
export type TextRange = [number, number];
export interface InputObservation {
  value: string;
  selection: TextRange;
  composing: boolean;
  composition: TextRange | null;
  sequence: number;
}
export interface EventDetail {
  key?: string;
  code?: string;
  repeat?: boolean;
  value?: Json;
  shiftKey?: boolean;
  altKey?: boolean;
  metaKey?: boolean;
  ctrlKey?: boolean;
  pointerId?: number;
  x?: number;
  y?: number;
  button?: number;
  buttons?: number;
  clickCount?: number;
  deltaX?: number;
  deltaY?: number;
  selection?: TextRange;
  composing?: boolean;
  composition?: TextRange | null;
  drag?: {
    types: string[];
    data: Record<string, Json>;
    effect: "none" | "copy" | "move" | "link";
  };
  range?: TextRange;
  /** A virtual list's `rangechange`: its viewport width in points. */
  width?: number;
}
export interface SurfaceEvent extends EventDetail {
  type: string;
  target: number;
  currentTarget: number;
  defaultPrevented: boolean;
  preventDefault(): void;
  stopPropagation(): void;
  stopImmediatePropagation(): void;
}
export interface SnapshotNode {
  id: number;
  type: string;
  children: number[];
  props: Props;
  text: string;
}
export function textRange(range: TextRange, length: number): TextRange {
  if (
    !Array.isArray(range) ||
    range.length !== 2 ||
    !range.every(Number.isSafeInteger) ||
    range[0] < 0 ||
    range[0] > range[1] ||
    range[1] > length
  )
    throw new RangeError("Invalid UTF-16 range");
  return [...range];
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
/** Complete native worker envelope; generation is checked before any callback. */
export interface NativeEvent {
  generation: number;
  plan: EventPlan;
  type: string;
  detail: EventDetail;
}
export function eventDetail(value: EventDetail): EventDetail {
  const detail = json(value) as unknown as EventDetail;
  for (const key of ["key", "code"] as const)
    if (detail[key] !== undefined && typeof detail[key] !== "string")
      throw new TypeError(`Invalid event ${key}`);
  for (const key of [
    "repeat",
    "shiftKey",
    "altKey",
    "ctrlKey",
    "metaKey",
    "composing",
  ] as const)
    if (detail[key] !== undefined && typeof detail[key] !== "boolean")
      throw new TypeError(`Invalid event ${key}`);
  for (const key of ["x", "y", "deltaX", "deltaY"] as const)
    if (detail[key] !== undefined && typeof detail[key] !== "number")
      throw new TypeError(`Invalid event ${key}`);
  for (const key of ["button", "buttons", "clickCount", "pointerId"] as const)
    if (
      detail[key] !== undefined &&
      (!Number.isSafeInteger(detail[key]) || detail[key]! < 0)
    )
      throw new TypeError(`Invalid event ${key}`);
  for (const key of ["range", "selection", "composition"] as const)
    if (detail[key] !== undefined && detail[key] !== null)
      textRange(
        detail[key],
        typeof detail.value === "string" && key !== "range"
          ? detail.value.length
          : Number.MAX_SAFE_INTEGER,
      );
  if (detail.composition && detail.composing !== true)
    throw new TypeError("Composition range requires composing input");
  if (
    detail.drag &&
    (!Array.isArray(detail.drag.types) ||
      !detail.drag.types.every((t) => typeof t === "string") ||
      !detail.drag.data ||
      Array.isArray(detail.drag.data) ||
      typeof detail.drag.data !== "object" ||
      !["none", "copy", "move", "link"].includes(detail.drag.effect))
  )
    throw new TypeError("Invalid drag data");
  return detail;
}
