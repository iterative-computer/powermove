import { test, expect } from "bun:test";
import { Evaluation } from "./harness";
import {
  TimelineRows,
  visibleRows,
  selectRow,
  type LayerRow,
} from "../examples/panels/timeline";
import { InspectorGroup, type PropertyRow } from "../examples/panels/inspector";
import {
  EffectsBrowser,
  browseEffects,
  recentAfterApply,
} from "../examples/panels/effects";
const wire = process.env.PM_SURFACE_WIRE;
const wireTest = test.skipIf(!wire);
const rows: LayerRow[] = Array.from({ length: 10000 }, (_, i) => ({
  id: `layer-${i}`,
  label: `Layer ${i}`,
  parent: null,
  group: false,
  collapsed: false,
  locked: i === 2,
}));
const property: PropertyRow = {
  id: "opacity",
  label: "Opacity",
  values: [0.625, 0.75],
  binding: "synthetic:opacity",
  editable: true,
  animated: true,
  keyAtPlayhead: false,
  generic: false,
  min: 0,
  max: 1,
  precision: 3,
  unit: "%",
};
const items = [
  { id: "blur", label: "Blur", group: "Filter" },
  { id: "tint", label: "Tint", group: "Color" },
  { id: "curves", label: "Curves", group: "Color" },
];

wireTest(
  "timeline ports stable range anchor, shrink and toggle selection",
  () => {
    const ids = ["a", "b", "c", "d"];
    const first = selectRow(ids, [], null, "b", false, false);
    const extended = selectRow(
      ids,
      first.selected,
      first.anchor,
      "d",
      true,
      false,
    );
    expect(extended).toEqual({ anchor: "b", selected: ["b", "c", "d"] });
    expect(
      selectRow(ids, extended.selected, extended.anchor, "c", true, false),
    ).toEqual({ anchor: "b", selected: ["b", "c"] });
    expect(selectRow(ids, ["a"], "a", "c", true, true).selected).toEqual([
      "a",
      "b",
      "c",
    ]);
    expect(selectRow(ids, ["a", "b"], "a", "b", false, true).selected).toEqual([
      "a",
    ]);
    expect(selectRow(ids, ["c"], "missing", "d", true, false).selected).toEqual(
      ["c", "d"],
    );
  },
);
wireTest(
  "timeline collapsed ancestor hides descendants and rejects broken fixture graphs",
  () => {
    const group = { ...rows[0], id: "group", group: true, collapsed: true };
    const child = { ...rows[1], id: "child", parent: "group" };
    expect(visibleRows([group, child, rows[2]]).map((row) => row.id)).toEqual([
      "group",
      rows[2].id,
    ]);
    expect(visibleRows([{ ...group, collapsed: false }, child]).length).toBe(2);
    expect(() => visibleRows([{ ...child, parent: "child" }])).toThrow(
      "Cyclic",
    );
    expect(() => visibleRows([{ ...child, parent: "absent" }])).toThrow(
      "Missing",
    );
  },
);
wireTest(
  "timeline bounds ten thousand rows, preserves IDs and forwards selection",
  async () => {
    const e = new Evaluation(wire),
      selections: unknown[] = [];
    const props = {
      rows,
      selected: [] as string[],
      anchor: null,
      start: 0,
      size: 32,
      onSelect: (value: unknown) => selections.push(value),
      onCollapse: () => {},
    };
    try {
      await e.render(<TimelineRows {...props} />);
      const stable = e.node("row:layer-1").id;
      expect(e.nodes().length).toBeLessThan(100);
      await e.event("name:layer-1", "click");
      expect(selections).toEqual([
        { anchor: "layer-1", selected: ["layer-1"] },
      ]);
      await e.render(
        <TimelineRows
          {...props}
          rows={[rows[0], rows[2], rows[1], ...rows.slice(3)]}
          selected={["layer-1"]}
        />,
      );
      expect(e.node("row:layer-1").id).toBe(stable);
      e.props("row:layer-1", { state: { selected: true, locked: false } });
      await e.render(<TimelineRows {...props} start={9000} />);
      expect(e.nodes().length).toBeLessThan(100);
      expect(e.find("row:layer-1")).toBeUndefined();
    } finally {
      await e.dispose();
    }
  },
);
wireTest(
  "timeline disclosure requests collapse without directly changing project data",
  async () => {
    const e = new Evaluation(wire),
      changes: unknown[] = [];
    try {
      await e.render(
        <TimelineRows
          rows={[{ ...rows[0], group: true }]}
          selected={[]}
          anchor={null}
          start={0}
          size={32}
          onSelect={() => {}}
          onCollapse={(...change) => changes.push(change)}
        />,
      );
      await e.event("disclosure:layer-0", "click");
      expect(changes).toEqual([["layer-0", true]]);
      expect(rows[0].collapsed).toBe(false);
    } finally {
      await e.dispose();
    }
  },
);
wireTest(
  "inspector preserves authored values, mixed state, metadata and binding identity",
  async () => {
    const e = new Evaluation(wire);
    try {
      await e.render(
        <InspectorGroup
          title="Transform"
          time={1.25}
          properties={[property]}
          onKey={() => {}}
          onSelectChannel={() => {}}
        />,
      );
      e.props("property:opacity", {
        value: 0.625,
        mixed: true,
        binding: "synthetic:opacity",
        min: 0,
        max: 1,
        precision: 3,
        unit: "%",
      });
      const id = e.node("property:opacity").id;
      await e.render(
        <InspectorGroup
          title="Transform"
          time={2.5}
          properties={[
            { ...property, values: [0.875, 0.875], keyAtPlayhead: true },
          ]}
          onKey={() => {}}
          onSelectChannel={() => {}}
        />,
      );
      e.props("property:opacity", {
        value: 0.875,
        mixed: false,
        binding: "synthetic:opacity",
        state: { animated: true, keyAtPlayhead: true },
      });
      expect(e.node("property:opacity").id).toBe(id);
    } finally {
      await e.dispose();
    }
  },
);
wireTest(
  "inspector sends current time for keys and distinguishes generic channel selection",
  async () => {
    const e = new Evaluation(wire),
      keys: unknown[] = [],
      channels: string[] = [];
    try {
      await e.render(
        <InspectorGroup
          title="Transform"
          time={1.25}
          properties={[property, { ...property, id: "uniform", generic: true }]}
          onKey={(...args) => keys.push(args)}
          onSelectChannel={(id) => channels.push(id)}
        />,
      );
      await e.event("key:opacity", "click");
      await e.event("property:opacity", "pointerdown");
      await e.event("property:uniform", "pointerdown");
      expect(keys).toEqual([["opacity", 1.25]]);
      expect(channels).toEqual(["opacity"]);
      await e.render(
        <InspectorGroup
          title="Transform"
          time={2}
          properties={[{ ...property, editable: false }]}
          onKey={(...args) => keys.push(args)}
          onSelectChannel={() => {}}
        />,
      );
      e.props("property:opacity", { disabled: true });
      await e.event("key:opacity", "click");
      expect(keys.length).toBe(1);
    } finally {
      await e.dispose();
    }
  },
);
wireTest(
  "effects preserves tag filter, category fallback, recents and max-eight behavior",
  () => {
    expect(
      browseEffects(items, "  BLUR  ", "Color", []).shown.map(
        (item) => item.id,
      ),
    ).toEqual(["blur"]);
    expect(
      browseEffects(items, "", "Color", []).shown.map((item) => item.id),
    ).toEqual(["tint", "curves"]);
    expect(browseEffects(items, "", "removed", []).active).toBe("all");
    expect(
      browseEffects(items, "", "recent", ["gone", "curves", "blur"]).shown.map(
        (item) => item.id,
      ),
    ).toEqual(["curves", "blur"]);
    expect(browseEffects(items, "", "recent", ["gone"]).active).toBe("all");
    expect(
      recentAfterApply(["a", "b", "c", "d", "e", "f", "g", "h"], "c"),
    ).toEqual(["c", "a", "b", "d", "e", "f", "g", "h"]);
  },
);
wireTest(
  "effects reports no selection, applies by ID and clears search on category pick",
  async () => {
    const e = new Evaluation(wire),
      applied: unknown[] = [],
      categories: string[] = [];
    let missing = 0;
    const props = {
      items,
      category: "all",
      recent: [],
      selectedLayer: null as string | null,
      onPickCategory: (id: string) => categories.push(id),
      onApply: (...args: string[]) => applied.push(args),
      onMissingSelection: () => missing++,
    };
    try {
      await e.render(<EffectsBrowser {...props} />);
      await e.event("effect:blur", "click");
      expect(missing).toBe(1);
      expect(applied).toEqual([]);
      await e.render(<EffectsBrowser {...props} selectedLayer="layer-1" />);
      await e.event("effect:blur", "click");
      expect(applied).toEqual([["layer-1", "blur"]]);
      await e.event("effects:query", "input", { value: "tint" });
      expect(e.find("effect:blur")).toBeUndefined();
      expect(e.find("effect:tint")).toBeDefined();
      await e.event("category:Color", "click");
      expect(categories).toEqual(["Color"]);
      expect(e.find("effect:blur")).toBeDefined();
    } finally {
      await e.dispose();
    }
  },
);
