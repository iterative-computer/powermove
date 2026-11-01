/** Public, synthetic behavior-only scaffold; no owner panel names, sources, or IDs. */
export type Group = "builtin" | "wild" | "held-out" | "owner";
export interface Task {
  id: string;
  group: Group;
  status: "ready" | "pending-owner";
  brief: string;
  checks: string[];
}
const groups: [Group, string, string[]][] = [
  [
    "builtin",
    "inspector",
    [
      "Render a labelled numeric property row and preserve the binding handle.",
      "Render keyed timeline rows and reverse their order without replacing IDs.",
      "Render a scrollable media list with image resource IDs.",
      "Render a curve path and an engine frame handle without raster readback.",
      "Render a layer disclosure and expand its children.",
      "Render controlled caption text and update it from a native observation.",
      "Render settings sections with typed boolean and select values.",
      "Render an export form with numeric, text and progress data.",
      "Render command search results with disabled choices and semantic labels.",
      "Render toolbar items and Glass with semantic surface metadata.",
    ],
  ],
  [
    "wild",
    "surface",
    [
      "Render a grid of metric cards from structured data.",
      "Render a graph using paths and anchored metadata.",
      "Render canvas and frame handles in a scroll container.",
      "Render markdown and code with exact authored whitespace.",
      "Render a virtual list with a bounded visible range.",
      "Render an anchored menu and close it by changing state.",
      "Render a nested focus scope with labelled inputs.",
      "Render a ramp, point and fill control preserving structured values.",
      "Render a webview requiring explicit host opt-in.",
      "Render selector metadata for id/class/role/attr/state.",
    ],
  ],
  [
    "held-out",
    "contract",
    [
      "Move an existing keyed child between sibling positions.",
      "Remove a subtree containing listeners and create a fresh sibling.",
      "Change listener capture/default declarations while keeping the element ID.",
      "Remove props with null while preserving false and empty strings.",
      "Render fragments and conditional empty children without phantom nodes.",
      "Preserve nested hook state through keyed sibling reordering.",
      "Replace a component through Refresh while preserving state.",
      "Preserve context label association through nested function components.",
      "Keep unusual finite JSON values exact through the op protocol.",
      "Reject an invalid whole commit and keep the last accepted tree.",
    ],
  ],
  [
    "owner",
    "reserved",
    Array.from(
      { length: 10 },
      () =>
        "Owner-panel behavior specification pending; must not count as a pass.",
    ),
  ],
];
export const tasks: Task[] = groups.flatMap(([group, label, briefs]) =>
  briefs.map((brief, index) => ({
    id: `${group}-${String(index + 1).padStart(2, "0")}`,
    group,
    status: group === "owner" ? "pending-owner" : "ready",
    brief,
    checks:
      group === "owner"
        ? []
        : ["whole-commit-validity", "semantic-tree", `${label}-${index + 1}`],
  })),
);
