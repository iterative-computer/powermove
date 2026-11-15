import type { Json } from "../../src/protocol";
import type { Publication } from "./contract";
const record = (v: unknown): v is Record<string, unknown> =>
  !!v && typeof v === "object" && !Array.isArray(v);
const strings = (v: unknown): v is string[] =>
  Array.isArray(v) && v.every((s) => typeof s === "string");
const count = (v: unknown): v is number =>
  typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
const nullableString = (v: unknown) => v === null || typeof v === "string";
const pairs = (v: unknown) =>
  Array.isArray(v) && v.every((p) => strings(p) && p.length === 2);
/** Finite JSON is already copied by HostClient/json. Refuse incomplete facts before publication. */
export function publication(
  value: Json,
  project: string,
  generation: number,
): Publication {
  if (
    !record(value) ||
    value.generation !== generation ||
    !count(value.sequence) ||
    value.sequence < 1 ||
    !record(value.snapshot) ||
    !record(value.native)
  )
    throw new Error("Invalid agent publication");
  const s = value.snapshot,
    n = value.native,
    r = s.rows,
    d = s.draft,
    c = s.catalog,
    a = s.attachments,
    settings = s.settings,
    slash = s.slash;
  if (
    s.projectId !== project ||
    typeof s.threadId !== "string" ||
    !s.threadId ||
    !nullableString(s.runId) ||
    typeof s.phase !== "string" ||
    !["sendable", "composerDisabled", "stopping", "showJump"].every(
      (k) => typeof s[k] === "boolean",
    ) ||
    !count(s.backgroundRuns) ||
    !nullableString(s.saveError) ||
    !nullableString(s.stopError) ||
    !(s.elapsedMs === null || count(s.elapsedMs)) ||
    !record(r) ||
    !count(r.count) ||
    !Array.isArray(r.range) ||
    r.range.length !== 2 ||
    !r.range.every(count) ||
    r.range[0] > r.range[1] ||
    r.range[1] > r.count ||
    !Array.isArray(r.items) ||
    r.items.length !== r.range[1] - r.range[0] ||
    r.items.length > 256 ||
    !r.items.every(
      (row) =>
        record(row) &&
        typeof row.id === "string" &&
        !!row.id &&
        typeof row.commandId === "string" &&
        typeof row.open === "boolean" &&
        record(row.chosen) &&
        record(row.value),
    ) ||
    new Set(r.items.map((row) => (record(row) ? row.id : undefined))).size !==
      r.items.length ||
    !record(d) ||
    typeof d.text !== "string" ||
    !nullableString(d.armed) ||
    !Array.isArray(d.selection) ||
    d.selection.length !== 2 ||
    !d.selection.every(count) ||
    d.selection[0] > d.selection[1] ||
    d.selection[1] > d.text.length ||
    !Array.isArray(a) ||
    !a.every(
      (item) =>
        record(item) &&
        typeof item.id === "string" &&
        !!item.id &&
        typeof item.name === "string" &&
        typeof item.payloadAvailable === "boolean",
    ) ||
    !record(settings) ||
    !["provider", "model", "effort", "scope"].every(
      (k) => typeof settings[k] === "string",
    ) ||
    !["editor", "supervised", "auto", "full"].includes(
      String(settings.access),
    ) ||
    !record(c) ||
    !pairs(c.models) ||
    !pairs(c.providers) ||
    !strings(c.efforts) ||
    !record(slash) ||
    typeof slash.visible !== "boolean" ||
    !count(slash.index) ||
    !Array.isArray(slash.choices) ||
    !slash.choices.every(
      (item) =>
        record(item) &&
        typeof item.id === "string" &&
        nullableString(item.value),
    ) ||
    !["checking", "waiting", "connected", "unavailable"].includes(
      String(n.connection),
    ) ||
    !["popover", "docked"].includes(String(n.presentation)) ||
    typeof n.reason !== "string" ||
    ![
      "atMinimumDockWidth",
      "visible",
      "extensionContext",
      "loadingHistory",
      "dropTarget",
      "needsAnswer",
      "textSelected",
      "increaseContrast",
      "reduceTransparency",
      "reduceMotion",
    ].every((k) => typeof n[k] === "boolean") ||
    !(
      n.keyboardFocus === null ||
      n.keyboardFocus === "composer" ||
      n.keyboardFocus === "conversation"
    ) ||
    !nullableString(n.hoverRow) ||
    !record(n.bindings) ||
    typeof n.bindings.access !== "string" ||
    typeof n.bindings.model !== "string" ||
    !record(n.slashCopy) ||
    !Object.values(n.slashCopy).every(
      (item) =>
        record(item) &&
        typeof item.label === "string" &&
        typeof item.description === "string",
    )
  )
    throw new Error("Invalid agent publication");
  return value as unknown as Publication;
}
