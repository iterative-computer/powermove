import type { Row, RowAction, MenuItem } from "./contracts";

export function rowMenu(row: Row): MenuItem[] {
  const enabled = !row.locked && !row.disabled;
  return [
    { label: "Save Animation Preset…", action: "save-preset", enabled },
    {
      label: "Add Keyframe at Playhead",
      action: "add-key",
      enabled: enabled && !!row.animation,
    },
    {
      label: "Remove Animation",
      action: "remove-animation",
      enabled: enabled && !!row.animation && row.animation !== "static",
    },
    {
      label: "Show in Graph Editor",
      action: "show-graph",
      enabled: !!row.animation,
    },
    ...(row.separateDimensions === undefined
      ? []
      : [
          {
            label: "Separate Dimensions",
            action: "separate-dimensions" as RowAction,
            enabled,
            checked: row.separateDimensions,
          },
        ]),
    ...(row.animation && row.animation !== "static"
      ? [
          {
            label: "Easing for All Keys",
            action: "ease:heading" as RowAction,
            enabled: false,
            separatorBefore: true,
          },
          ...[
            ["power", "Power"],
            ["linear", "Linear"],
            ["easeInOut", "Ease In and Out"],
            ["expoOut", "Exponential Out"],
            ["backOut", "Back Out"],
            ["glide", "Glide"],
            ["snap", "Snap"],
          ].map(([preset, label]) => ({
            label: label!,
            action: `ease:${preset}` as RowAction,
            enabled,
          })),
        ]
      : []),
    {
      label: row.expression ? "Edit Expression…" : "Add Expression…",
      action: "edit-expression",
      enabled: enabled && !!row.animation,
      separatorBefore: true,
    },
    {
      label: "Remove Expression",
      action: "remove-expression",
      enabled: enabled && !!row.expression,
    },
    { label: "Reset", action: "reset", enabled, separatorBefore: true },
    ...(row.linked === undefined
      ? []
      : [
          {
            label: "Link Scale Axes",
            action: "link" as RowAction,
            enabled,
            checked: row.linked,
          },
        ]),
  ];
}
