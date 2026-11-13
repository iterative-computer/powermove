import type { Json } from "../../src/protocol";

export const PANEL_ID = "layer-effects";
export const PANEL_TITLE = "Effect Controls";
export const MODEL_COMMAND = "panel.effect-controls";
export const MODEL_CHANNEL = "panel:layer-effects";
export type Target = "NoProject" | "NoSelection" | "Layer" | "Unsupported";
export interface Parameter {
  id: string;
  effect: string;
  /** The definition's parameter key. */
  key?: string;
  label: string;
  binding: string;
  value: Json;
  /** Without a definition: the stored value at rest, or "Animated" when keyed. */
  display?: Json;
  control: "number" | "text" | "toggle" | "color" | "choice" | "opaque";
  animated: boolean;
  keyAtPlayhead: boolean;
  mixed: boolean;
  readonly: boolean;
  step?: number;
  min?: number;
  max?: number;
  precision?: number;
  unit?: string;
  options?: { v: Json; label: string; disabled?: boolean }[];
}
export interface EffectHeader {
  kind: "effect";
  id: string;
  /** The effect's definition id (`type`). */
  definition?: string;
  /** Its definition says `grade: true`: a grading tool the Color panel lists. */
  grade?: boolean;
  label: string;
  enabled: boolean;
  expanded: boolean;
  missing: boolean;
  selected: boolean;
  ordinal: number;
  effectCount: number;
}
export interface ParameterRow {
  kind: "parameter";
  id: string;
  parameter: Parameter;
  effectLabel: string;
  enabled: boolean;
}
export interface ActionRow {
  kind: "action";
  id: string;
  effect: string;
  index: number;
  label: string;
  readonly: boolean;
}
export type Row = EffectHeader | ParameterRow | ActionRow;
/** Read-only worker projection of pm-panel-inspector, never an editable JS document. */
export interface Projection {
  version: 1;
  target: Target;
  layer: string | null;
  name: string;
  typeLabel: string;
  locked: boolean;
  revision: number;
  documentId: number;
  count: number;
  effectCount: number;
  selectedCount: number;
  range: [number, number];
  rows: Row[];
  focus: string | null;
  focusLabel: string | null;
  clipboardCount: number;
}
export type Intent =
  | { type: "action"; id: string; index: number }
  | { type: "select"; id: string; meta: boolean; ctrl: boolean; shift: boolean }
  | { type: "navigate"; direction: -1 | 1; shift: boolean }
  | { type: "disclose"; id: string; open: boolean }
  | { type: "toggle" | "reset" | "remove" | "copy"; id: string }
  | { type: "paste" | "clear" }
  | { type: "reorder"; id: string; direction: -1 | 1 }
  | { type: "drop"; id: string; slot: number }
  | { type: "add"; definition: string; slot?: number }
  | {
      type: "parameter";
      id: string;
      action: "diamond" | "stopwatch" | "reset";
    }
  | { type: "range"; range: [number, number] };
export interface MenuItem {
  id: string;
  label: string;
  disabled?: boolean;
  tooltip?: string;
  separator?: boolean;
  children?: MenuItem[];
}
export interface Platform {
  /** NSMenu, including registry groups and Recent; never a DOM panel. */
  effectMenu(): Promise<string | null>;
  contextMenu(items: MenuItem[]): Promise<string | null>;
  /** Maps semantic IDs to accepted numeric nodes, queues native focus/reveal. */
  focus(id: string): void;
  announce(message: string): void;
  report(error: unknown): void;
}
export const emptyProjection = (): Projection => ({
  version: 1,
  target: "NoProject",
  layer: null,
  name: "",
  typeLabel: "",
  locked: false,
  revision: 0,
  documentId: 0,
  count: 0,
  effectCount: 0,
  selectedCount: 0,
  range: [0, 0],
  rows: [],
  focus: null,
  focusLabel: null,
  clipboardCount: 0,
});

/** Fail closed on foreign/malformed model output; keep the previous accepted view. */
export function projection(value: Json): Projection {
  const p = value as unknown as Projection;
  if (
    !p ||
    p.version !== 1 ||
    !["NoProject", "NoSelection", "Layer", "Unsupported"].includes(p.target) ||
    !Number.isSafeInteger(p.documentId) ||
    p.documentId < 0 ||
    !Number.isSafeInteger(p.revision) ||
    p.revision < 0 ||
    !Number.isSafeInteger(p.count) ||
    p.count < 0 ||
    !Array.isArray(p.range) ||
    p.range.length !== 2 ||
    !p.range.every(Number.isSafeInteger) ||
    p.range[0] < 0 ||
    p.range[1] < p.range[0] ||
    p.range[1] > p.count ||
    !Array.isArray(p.rows) ||
    p.rows.length !== p.range[1] - p.range[0] ||
    !Number.isSafeInteger(p.effectCount) ||
    p.effectCount < 0 ||
    !Number.isSafeInteger(p.selectedCount) ||
    p.selectedCount < 0 ||
    p.selectedCount > p.effectCount ||
    !Number.isSafeInteger(p.clipboardCount) ||
    p.clipboardCount < 0 ||
    typeof p.locked !== "boolean" ||
    typeof p.name !== "string" ||
    typeof p.typeLabel !== "string" ||
    !(p.layer === null || typeof p.layer === "string") ||
    !(p.focus === null || typeof p.focus === "string") ||
    !(p.focusLabel === null || typeof p.focusLabel === "string")
  )
    throw new TypeError("Invalid effect-controls projection");
  const ids = new Set<string>();
  for (const row of p.rows) {
    if (!row || typeof row.id !== "string" || ids.has(row.id))
      throw new TypeError("Invalid row identity");
    ids.add(row.id);
    if (row.kind === "effect") {
      if (
        typeof row.label !== "string" ||
        ![row.enabled, row.expanded, row.missing, row.selected].every(
          (v) => typeof v === "boolean",
        ) ||
        !Number.isSafeInteger(row.ordinal) ||
        row.ordinal < 0 ||
        !Number.isSafeInteger(row.effectCount) ||
        row.ordinal >= row.effectCount
      )
        throw new TypeError("Invalid effect row");
    } else if (row.kind === "parameter") {
      const r = row.parameter;
      if (
        !r ||
        typeof row.effectLabel !== "string" ||
        typeof row.enabled !== "boolean" ||
        !["number", "text", "toggle", "color", "choice", "opaque"].includes(
          r.control,
        ) ||
        ![r.id, r.effect, r.label, r.binding].every(
          (v) => typeof v === "string",
        ) ||
        ![r.animated, r.keyAtPlayhead, r.mixed, r.readonly].every(
          (v) => typeof v === "boolean",
        ) ||
        (r.control === "number" &&
          (typeof r.value !== "number" || !Number.isFinite(r.value))) ||
        (["text", "color"].includes(r.control) &&
          typeof r.value !== "string") ||
        (r.control === "toggle" && typeof r.value !== "boolean")
      )
        throw new TypeError("Invalid parameter row");
    } else if (row.kind === "action") {
      if (
        typeof row.effect !== "string" ||
        typeof row.label !== "string" ||
        typeof row.readonly !== "boolean" ||
        !Number.isSafeInteger(row.index) ||
        row.index < 0 ||
        row.index >= 16
      )
        throw new TypeError("Invalid action row");
    } else throw new TypeError("Invalid row kind");
  }
  return structuredClone(p);
}
