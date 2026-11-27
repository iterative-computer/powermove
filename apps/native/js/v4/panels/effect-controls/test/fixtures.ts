import { emptyProjection, type Projection, type Row } from "../contract";
export const radius = {
  id: '{"layer":"a","path":"f1.radius"}',
  effect: "f1",
  label: "Radius",
  binding: "radius",
  value: 24,
  control: "number" as const,
  animated: false,
  keyAtPlayhead: false,
  mixed: false,
  readonly: false,
  step: 1,
  unit: "px",
};
export function selected(): Projection {
  return {
    ...emptyProjection(),
    target: "Layer",
    layer: "a",
    name: "Headline",
    typeLabel: "Text",
    count: 3,
    effectCount: 2,
    selectedCount: 1,
    range: [0, 3],
    focus: "f1",
    focusLabel: "Glow",
    rows: [
      {
        kind: "effect",
        id: "f1",
        label: "Glow",
        enabled: true,
        expanded: true,
        missing: false,
        selected: true,
        ordinal: 0,
        effectCount: 2,
      },
      {
        kind: "parameter",
        id: "radius",
        parameter: { ...radius },
        effectLabel: "Glow",
        enabled: true,
      },
      {
        kind: "effect",
        id: "f2",
        label: "Film Grain",
        enabled: true,
        expanded: false,
        missing: false,
        selected: false,
        ordinal: 1,
        effectCount: 2,
      },
    ],
  };
}
export function states(): Record<string, Projection> {
  const noEffects = {
    ...selected(),
    rows: [],
    count: 0,
    effectCount: 0,
    selectedCount: 0,
    focus: null,
    range: [0, 0] as [number, number],
  };
  const disabled = selected();
  disabled.rows[0] = {
    ...(disabled.rows[0] as Extract<Row, { kind: "effect" }>),
    enabled: false,
  };
  (disabled.rows[1] as Extract<Row, { kind: "parameter" }>).enabled = false;
  const error = selected();
  (error.rows[0] as Extract<Row, { kind: "effect" }>).missing = true;
  const param = error.rows[1] as Extract<Row, { kind: "parameter" }>;
  param.parameter = {
    ...param.parameter,
    control: "opaque",
    readonly: true,
    value: { v: 24, opaque: true },
  };
  const collapsed = selected();
  collapsed.rows.splice(1, 1);
  collapsed.count = 2;
  collapsed.range = [0, 2];
  (collapsed.rows[0] as Extract<Row, { kind: "effect" }>).expanded = false;
  const multi = selected();
  multi.selectedCount = 2;
  (multi.rows[2] as Extract<Row, { kind: "effect" }>).selected = true;
  const large = selected();
  large.count = 60_000;
  large.effectCount = 60_000;
  large.selectedCount = 0;
  large.range = [30_000, 30_003];
  large.rows = Array.from({ length: 3 }, (_, i) => ({
    kind: "effect",
    id: `f${30_000 + i}`,
    label: "Glow",
    enabled: true,
    expanded: false,
    missing: false,
    selected: false,
    ordinal: 30_000 + i,
    effectCount: 60_000,
  }));
  return {
    "no-project": emptyProjection(),
    "no-selection": { ...emptyProjection(), target: "NoSelection" },
    "no-effects": noEffects,
    unsupported: { ...noEffects, target: "Unsupported" },
    selected: selected(),
    disabled,
    locked: { ...selected(), locked: true },
    error,
    collapsed,
    "multi-select": multi,
    "60k": large,
    "loading-na": selected(),
  };
}
