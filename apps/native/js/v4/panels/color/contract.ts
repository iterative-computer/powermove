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
/** Color Curves' Channel choice, in its option order. */
export const CURVE_CHANNELS = ["Master", "Red", "Green", "Blue"];
/** The app command that asks for a `.cube` or `.3dl` file and applies it. */
export const LOAD_LUT = "viewer.applyLut";
/** Import/Export CDL… (app commands; the agent's take a path). */
export const IMPORT_CDL = "color.importCdl";
export const EXPORT_CDL = "color.exportCdl";

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

/** The four wheels, in the order a colourist reads them: each pushes red,
 * green and blue together around its neutral value (gain is a multiplier,
 * so neutral at 1), within its channels' limits. */
export const WHEELS_OF: {
  key: string;
  title: string;
  neutral: number;
  range: [number, number];
}[] = [
  { key: "lift", title: "Lift", neutral: 0, range: [-1, 1] },
  { key: "gamma", title: "Gamma", neutral: 0, range: [-2, 2] },
  { key: "gain", title: "Gain", neutral: 1, range: [0, 8] },
  { key: "offset", title: "Offset", neutral: 0, range: [-1, 1] },
];
/** Controls written together through one grouped binding: their rows' ids
 * in value order, and the binding the host derives from the rows' own
 * (joined by `+`). */
export interface Group {
  binding: string;
  channels: string[];
  value: number[];
}
function group(rows: (ParameterRow | undefined)[]): Group | null {
  if (rows.some((r) => !r || r.parameter.readonly)) return null;
  const all = rows as ParameterRow[];
  return {
    binding: all.map((r) => r.parameter.binding).join("+"),
    channels: all.map((r) => r.parameter.id),
    value: all.map((r) => Number(r.parameter.value)),
  };
}
/** A wheel's red, green and blue. */
export function wheelGroup(g: GradeEffect, wheel: string): Group | null {
  return group(
    ["Red", "Green", "Blue"].map((c) => parameter(g, `${wheel}${c}`)),
  );
}
/** A curve's point count and every point: `[count, x0, y0, …, x14, y14]`. */
export function curveGroup(g: GradeEffect): Group | null {
  const keys = ["count"];
  for (let i = 0; i < 15; i++) keys.push(`x${i}`, `y${i}`);
  return group(keys.map((k) => parameter(g, k)));
}
/** Every grouped binding the panel's controls use. */
export function groups(p: Projection): Group[] {
  const out: Group[] = [];
  for (const g of grade(p)) {
    if (g.header.definition === WHEELS)
      for (const w of WHEELS_OF) {
        const x = wheelGroup(g, w.key);
        if (x) out.push(x);
      }
    if (g.header.definition === CURVES) {
      const x = curveGroup(g);
      if (x) out.push(x);
    }
  }
  return out;
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
    action: "Import CDL…",
    agent: ["run_command", "color.importCdlFile"],
  },
  {
    action: "Export CDL…",
    agent: ["run_command", "color.exportCdlFile"],
  },
  {
    action: "Copy and paste a grade",
    agent: ["get_project_state", "apply_commands", "add_effect"],
  },
  { action: "Read the grade back", agent: ["get_project_state"] },
];
