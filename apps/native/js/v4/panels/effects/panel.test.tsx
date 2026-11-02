import { expect, test } from "bun:test";
import { Evaluation } from "../../eval/harness";
import { EffectsPanel } from "./panel";
import { ids } from "./model";
import { fixture, itemRow, items } from "./fixtures";
import { replay } from "./native-test";

const bindings = {
  search: "local:effects:search",
  scope: "local:effects:scope",
};
type Fixture = ReturnType<typeof fixture>;
const cases: [string, (f: Fixture) => void | Promise<void>][] = [
  [
    "empty",
    ({ model }) => {
      model.replaceRegistry({ items: [], groups: [] });
    },
  ],
  ["loading-N-A-synchronous-registry", () => {}],
  [
    "selected",
    ({ model }) => {
      model.select(itemRow(model).key);
    },
  ],
  [
    "hover",
    ({ model }) => {
      model.hover(itemRow(model).key);
    },
  ],
  [
    "focus",
    ({ model }) => {
      model.focused = itemRow(model).key;
      model.setFocus("list");
    },
  ],
  [
    "disabled",
    ({ model }) => {
      model.select(itemRow(model).key);
      model.setTarget(null);
    },
  ],
  [
    "error",
    ({ model }) => {
      model.replaceRegistry({
        items: [{ ...items[0], failed: true }],
        groups: [],
      });
    },
  ],
  [
    "drag-source-model-payload-native-image-pending",
    ({ model }) => {
      expect(model.drag("blur")).toEqual({
        kind: "effect",
        id: "blur",
        label: "Gaussian Blur",
      });
    },
  ],
  [
    "drop-N-A-panel-does-not-accept",
    ({ model }) => {
      expect(model.inspect().acceptsDrops).toBe(false);
    },
  ],
  [
    "multi-select-N-A-single-ID",
    ({ model }) => {
      model.select(itemRow(model).key);
      model.select(itemRow(model, "glow").key);
    },
  ],
  [
    "no-search-results",
    async ({ model }) => {
      await model.setQuery("absent");
    },
  ],
  [
    "empty-scope",
    ({ model }) => {
      model.replaceRegistry({
        items,
        groups: ["Blur & Sharpen", "Stylize", "Empty"],
      });
      model.pickScope("group:Empty");
    },
  ],
  [
    "search-match",
    async ({ model }) => {
      await model.setQuery("BLUR");
    },
  ],
  [
    "recent-flat",
    ({ model }) => {
      model.restoreSettings(["glow", "blur"], []);
      model.pickScope("recent");
    },
  ],
  [
    "collapsed",
    ({ model }) => {
      model.toggle(model.rows()[0].key, false);
    },
  ],
  [
    "preset-supplied-by-model",
    ({ model }) => {
      model.replaceRegistry({
        items: [{ ...items[0], kind: "preset" }],
        groups: [],
      });
    },
  ],
  [
    "60k-rows",
    ({ model }) => {
      model.replaceRegistry({
        items: Array.from({ length: 60000 }, (_, i) => ({
          ...items[0],
          id: `fx-${i}`,
          label: `Effect ${i}`,
        })),
        groups: [],
      });
      model.setViewport(30000, 2);
    },
  ],
];
for (const [state, arrange] of cases)
  test(`Effects spec state: ${state} has a validated op-tree and native accessibility snapshot`, async () => {
    const f = fixture();
    f.model.setTarget({ id: "layer", effects: [] });
    await arrange(f);
    const evaluation = new Evaluation(process.env.PM_SURFACE_WIRE);
    try {
      await evaluation.render(
        <EffectsPanel model={f.model} bindings={bindings} />,
      );
      const result = replay(evaluation.commits).at(-1)!;
      expect(result.accepted).toBe(true);
      expect(result.diagnostics).toEqual([]);
      expect(result.wireDiagnostics).toEqual([]);
      // Accepted native tree includes listener maps, exact token props and all text.
      expect({
        nodes: result.nodes,
        accessibility: result.accessibility,
        focusOrder: result.focusOrder,
      }).toMatchSnapshot(state);
      expect(evaluation.node(ids.list).props.metadata).toMatchObject({
        acceptsDrops: false,
        selectionMode: "single",
      });
      if (state === "60k-rows") {
        expect(evaluation.nodes().length).toBeLessThan(65);
        expect(evaluation.node(ids.list).props.metadata).toMatchObject({
          rowCount: 60001,
          range: [29998, 30004],
        });
      }
      expect(result.accessibility).toHaveLength(evaluation.nodes().length);
    } finally {
      f.model.dispose();
      await evaluation.dispose();
    }
  });

test("native AX labels, selected/expanded/disabled values and single list Tab stop are projected", async () => {
  const { model } = fixture();
  model.select(itemRow(model).key);
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    const native = replay(e.commits).at(-1)!;
    const ax = (id: string) =>
      native.accessibility.find(
        (node: { id: number }) => node.id === e.node(id).id,
      );
    expect(ax(ids.panel)).toMatchObject({ role: "Group", label: "Effects" });
    expect(ax(ids.search)).toMatchObject({
      role: "TextInput",
      label: "Search effects",
    }); // SearchInput missing in kit.
    expect(ax(ids.scope)).toMatchObject({
      role: "ComboBox",
      label: "Category, All",
      value: "All",
    });
    expect(ax(ids.list)).toMatchObject({ role: "Tree", label: "Effects" });
    const row = itemRow(model);
    expect(ax(ids.row(row.key))).toMatchObject({
      role: "TreeItem",
      label: "Gaussian Blur, effect, Blur & Sharpen",
      selected: true,
    });
    expect(ax(ids.add(row.key))).toMatchObject({
      role: "Button",
      label: "Add Gaussian Blur",
      disabled: true,
      actions: [],
    });
    expect(native.focusOrder).toEqual([
      e.node(ids.search).id,
      e.node(ids.scope).id,
      e.node(ids.list).id,
    ]);
  } finally {
    model.dispose();
    await e.dispose();
  }
});
test("surface row click only selects; Add stops propagation and double-click applies through the model", async () => {
  const { model, applied } = fixture();
  model.setTarget({ id: "layer", effects: [] });
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    const row = itemRow(model);
    await e.event(ids.row(row.key), "click");
    expect(model.selected).toBe("blur");
    expect(applied).toHaveLength(0);
    await e.event(ids.add(row.key), "click");
    expect(applied).toHaveLength(1);
    // Recents adds another presentation; stable key still addresses the original.
    await e.event(ids.row(row.key), "doubleclick");
    expect(applied).toHaveLength(2);
    const secondClick = { key: "", clickCount: 2 };
    await e.event(ids.add(row.key), "click", secondClick);
    await e.event(ids.add(row.key), "doubleclick");
    expect(applied).toHaveLength(2);
  } finally {
    model.dispose();
    await e.dispose();
  }
});
test("surface keyboard and search events use model state, focus requests and exact token highlighting", async () => {
  const { model, applied } = fixture();
  model.setTarget({ id: "layer", effects: [] });
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    await e.event(ids.search, "focus");
    await e.event(ids.search, "input", { value: "BLUR" });
    expect(model.rows()).toHaveLength(1);
    await e.event(ids.search, "keydown", { key: "ArrowDown" });
    expect(model.focus).toBe("list");
    expect(model.focusRequest).toBe(ids.row(itemRow(model).key));
    await e.event(ids.list, "keydown", { key: "Enter" });
    expect(applied).toHaveLength(1);
    await e.event(ids.list, "keydown", { key: "Escape" });
    expect(model.focusRequest).toBe(ids.search);
  } finally {
    model.dispose();
    await e.dispose();
  }
});
test("Increase Contrast adds token-only outlines, with no literal visual values", async () => {
  const { model } = fixture();
  model.select(itemRow(model).key);
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(
      <EffectsPanel model={model} bindings={bindings} increaseContrast />,
    );
    const native = replay(e.commits).at(-1)!;
    expect(native.diagnostics).toEqual([]);
    expect(e.node(ids.panel).props.style).toMatchObject({
      borderWidth: { token: "metrics.border" },
      borderColor: { token: "colors.separator" },
    });
    expect(e.node(ids.row(itemRow(model).key)).props.style).toMatchObject({
      background: { token: "colors.selected_fill" },
    });
  } finally {
    model.dispose();
    await e.dispose();
  }
});

test("native scope, disclosure and hover observations update the same model and keep Add conditional", async () => {
  const { model } = fixture();
  model.setTarget({ id: "layer", effects: [] });
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    await model.setQuery("blur");
    await e.flush();
    await e.event(ids.scope, "change", { value: "group:Stylize" });
    expect(model.query).toBe("");
    expect(model.scope).toBe("group:Stylize");
    expect(model.rows()).toHaveLength(1);
    await e.event(ids.scope, "change", { value: "all" });
    const row = itemRow(model);
    expect(e.find(ids.add(row.key))).toBeUndefined();
    await e.event(ids.row(row.key), "pointermove");
    expect(e.find(ids.add(row.key))).toBeDefined();
    await e.event(ids.list, "pointermove");
    expect(e.find(ids.add(row.key))).toBeUndefined();
    await e.event(ids.row(row.parent!), "click");
    expect(model.rows().some((other) => other.key === row.key)).toBe(false);
  } finally {
    model.dispose();
    await e.dispose();
  }
});
