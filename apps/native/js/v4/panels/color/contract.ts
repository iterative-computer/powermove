/**
 * The Color panel: a second view of the selected layer's grade, which stays
 * ordinary effect data on the layer (Color Wheels, Color Curves, Apply Color
 * LUT, and any effect whose definition says `grade: true`). It reads the
 * Effect Controls model in its own realm (`model.ts`) and drives it with the
 * same intents; values are written by the native controls through their
 * bindings, one Undo step per gesture.
 */
import type { EffectHeader, Intent, ParameterRow, Projection } from "./model";

export const PANEL_ID = "color";
export const PANEL_TITLE = "Color";
export const WHEELS = "colorWheels";
export const CURVES = "colorCurves";
export const LUT = "applyColorLut";
/** The app command that asks for a `.cube` or `.3dl` file and applies it. */
export const LOAD_LUT = "viewer.applyLut";

/** Color Wheels' controls, in the order a colourist reads them. */
export const WHEEL_SECTIONS: { title: string; keys: string[] }[] = [
  { title: "Balance", keys: ["temperature", "tint"] },
  { title: "Tone", keys: ["exposure", "contrast", "pivot", "saturation"] },
  ...["lift", "gamma", "gain", "offset"].map((wheel) => ({
    title: wheel[0].toUpperCase() + wheel.slice(1),
    keys: ["Master", "Red", "Green", "Blue"].map((c) => `${wheel}${c}`),
  })),
];

export interface GradeEffect {
  header: EffectHeader;
  parameters: ParameterRow[];
}

/** The layer's grading effects in stack order, with their parameter rows. */
export function grade(p: Projection): GradeEffect[] {
  const out: GradeEffect[] = [];
  let current: GradeEffect | null = null;
  for (const row of p.rows) {
    if (row.kind === "effect") {
      current = row.grade ? { header: row, parameters: [] } : null;
      if (current) out.push(current);
    } else if (row.kind === "parameter" && current)
      current.parameters.push(row);
  }
  return out;
}

/** What the panel asks of the model so every grading row is projected:
 * the whole list as its window, and each grading effect disclosed. */
export function coverage(p: Projection): Intent[] {
  if (p.target !== "Layer") return [];
  if (p.range[0] !== 0 || p.range[1] !== p.count)
    return [{ type: "range", range: [0, p.count] }];
  return grade(p)
    .filter((g) => !g.header.expanded)
    .map((g) => ({ type: "disclose", id: g.header.id, open: true }));
}

/** A parameter row by its definition key. */
export function parameter(
  g: GradeEffect,
  key: string,
): ParameterRow | undefined {
  return g.parameters.find((r) => r.parameter.key === key);
}

/** Color Curves' rows that show: its channel, point count, and the points in use. */
export function curveRows(g: GradeEffect): ParameterRow[] {
  const count = Number(parameter(g, "count")?.parameter.value ?? 2);
  return g.parameters.filter((r) => {
    const m = /^[xy](\d+)$/.exec(r.parameter.key ?? "");
    return !m || Number(m[1]) < count;
  });
}

/** Every action the panel offers a person, with the agent's route to the
 * same result: MCP tools, each `apply_commands` operation named after it.
 * `color.test.ts` checks every name against the MCP catalog and
 * `crates/mcp/tests/grade.rs` runs each route on a real project. */
export const ACTIONS: { action: string; agent: string[] }[] = [
  { action: "Set a value", agent: ["apply_commands", "set_property"] },
  { action: "Key a value", agent: ["apply_commands", "replace_keyframes"] },
  { action: "Reset a control", agent: ["apply_commands", "set_property"] },
  { action: "Reset a tool", agent: ["apply_commands", "set_property"] },
  { action: "Turn a tool on or off", agent: ["apply_commands", "set_effect"] },
  { action: "Add Color Wheels", agent: ["apply_commands", "add_effect"] },
  { action: "Add Curve", agent: ["apply_commands", "add_effect"] },
  { action: "Remove a tool", agent: ["apply_commands", "remove_effect"] },
  {
    // Adds Apply Color LUT, or relinks the one whose file is missing.
    action: "Load LUT…",
    agent: ["import_media", "apply_commands", "add_effect", "set_effect"],
  },
  {
    action: "Copy and paste a grade",
    agent: ["get_project_state", "apply_commands", "add_effect"],
  },
  { action: "Read the grade back", agent: ["get_project_state"] },
];
