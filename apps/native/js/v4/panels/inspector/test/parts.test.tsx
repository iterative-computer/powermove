import { expect, test } from "bun:test";
import type { ModelPartItem, PartsSectionData, Row } from "../contracts";
import { InspectorPanel } from "../view";
import { binding, fixture, snapshot } from "./fixture";
import { Probe } from "./probe";

function part(v: string, props: Partial<ModelPartItem> = {}): ModelPartItem {
  return {
    v,
    label: v,
    parent: null,
    depth: 0,
    children: 0,
    visible: true,
    edited: false,
    ...props,
  };
}
function state(parts: PartsSectionData, rows: Row[] = []) {
  const value = snapshot();
  if (value.target.kind !== "layers")
    throw new Error("Expected selected layer");
  value.target.first = {
    ...value.target.first,
    id: parts.layer,
    type: "3d",
    typeLabel: "3D",
    studio: true,
  };
  value.sections = [
    { id: "3d.part", title: "Parts", collapsed: false, rows, parts },
  ];
  return value;
}
const transforms: Row[] = [
  ["position", "Position", ["x", "y", "z"], "", 0.01],
  ["rotation", "Rotation", ["rx", "ry", "rz"], "°", 1],
  ["scale", "Scale", ["scale"], "%", 1],
].map(([id, label, fields, unit, step]) => ({
  id: `3d.part.0_0.${id}`,
  label,
  animation: "static",
  control: {
    kind: "number",
    axes: fields.map((field, index) => ({
      axis: fields.length > 1 ? ["X", "Y", "Z"][index] : undefined,
      binding: binding(`3d.part.0_0.${field}`),
      value: id === "scale" ? 100 : 0,
      unit,
      step,
      precision: 2,
    })),
  },
}));

test("Parts tree renders grouped controls, selects and edits through the inspector", async () => {
  const parts: PartsSectionData = {
    layer: "parts-actions",
    active: null,
    activeName: null,
    activeEdited: false,
    items: [
      part("0", { label: "Wrapper", children: 1 }),
      part("0_0", {
        label: "Body",
        parent: "0",
        depth: 1,
        edited: true,
        visible: false,
      }),
    ],
  };
  const f = fixture(state(parts)),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    expect(p.result.diagnostics).toEqual([]);
    expect(p.result.wireDiagnostics).toEqual([]);
    expect(p.node("parts:scroll").props.role).toBe("tree");
    expect(p.node("parts:whole").props["aria-selected"]).toBe(true);
    expect(
      Object.values(p.result.nodes).some(({ text }) =>
        text.includes("Double-click the model"),
      ),
    ).toBe(true);

    await p.drive("parts:expand:0", "click");
    expect(() => p.node("parts:item:0_0")).toThrow();
    expect(f.model.intents).toEqual([]);
    await p.drive("parts:expand:0", "keydown", { key: "Enter" });
    expect(p.node("parts:item:0_0").props.value).toBe("Changed");
    expect(p.node("parts:visible:0_0").props["aria-pressed"]).toBe(true);
    await p.drive("parts:visible:0_0", "click");
    expect(f.model.intents).toEqual([
      { type: "set-value", row: "3d.part.0_0.visible", value: true },
    ]);
    await p.drive("parts:item:0_0", "keydown", { key: " " });
    expect(f.model.intents.at(-1)).toEqual({
      type: "set-value",
      row: "3d.part.select",
      value: "0_0",
    });

    parts.active = "0_0";
    parts.activeName = "Body";
    parts.activeEdited = true;
    parts.items[1].visible = true;
    f.model.value = state(parts, transforms);
    await f.publish();
    await f.session.settled();
    await p.flush();
    expect(p.node("parts:item:0_0").props["aria-selected"]).toBe(true);
    expect(p.node("parts:visible:0_0").props["aria-pressed"]).toBe(false);
    for (const row of transforms) {
      if (row.control.kind !== "number")
        throw new Error("Expected number axes");
      for (const [index, axis] of row.control.axes.entries())
        expect(
          p.node(`value:${row.id}:${axis.axis ?? index}`).props.binding,
        ).toBe(axis.binding.handle);
      await p.drive(`key:${row.id}`, "click");
      expect(f.model.intents.at(-1)).toEqual({
        type: "key",
        row: row.id,
        removeAnimation: false,
      });
    }
    await p.drive("parts:reset", "click");
    expect(f.model.intents.at(-1)).toEqual({
      type: "set-value",
      row: "3d.part.0_0.reset",
      value: null,
    });
    await p.drive("parts:whole", "click");
    expect(f.model.intents.at(-1)).toEqual({
      type: "set-value",
      row: "3d.part.select",
      value: null,
    });
    expect(
      await f.session.dispatch({
        type: "set-value",
        row: "3d.part.unknown.visible",
        value: false,
      }),
    ).toBe(false);
    expect(p.result.diagnostics).toEqual([]);
    expect(p.result.wireDiagnostics).toEqual([]);
    expect(f.errors).toEqual([]);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("Parts tree auto-expands small roots, only children and the path to an active part", async () => {
  const parts: PartsSectionData = {
    layer: "parts-auto-open",
    active: null,
    activeName: null,
    activeEdited: false,
    items: [
      part("0", { children: 2 }),
      part("0_0", { parent: "0", depth: 1, children: 1 }),
      part("0_0_0", { parent: "0_0", depth: 2 }),
      part("0_1", { parent: "0", depth: 1 }),
      part("1", { children: 1 }),
      part("1_0", { parent: "1", depth: 1, children: 1 }),
      part("1_0_0", { parent: "1_0", depth: 2 }),
    ],
  };
  const f = fixture(state(parts)),
    p = new Probe();
  await f.session.start();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    expect(p.node("parts:item:0_0").props["aria-expanded"]).toBe(false);
    expect(p.node("parts:item:1_0").props["aria-expanded"]).toBe(true);
    expect(p.node("parts:item:1_0_0")).toBeDefined();
    expect(() => p.node("parts:item:0_0_0")).toThrow();

    parts.items.push(
      ...Array.from({ length: 24 }, (_, index) => part(String(index + 2))),
    );
    parts.active = "0_0_0";
    parts.activeName = "0_0_0";
    f.model.value = state(parts);
    await f.publish();
    await f.session.settled();
    await p.flush();
    expect(p.node("parts:item:0_0_0").props["aria-selected"]).toBe(true);
    expect(() => p.node("parts:item:1_0")).toThrow();
    await p.drive("parts:expand:0", "click");
    expect(() => p.node("parts:item:0_0_0")).toThrow();

    await p.render(<InspectorPanel session={f.session} key="remount" />);
    expect(() => p.node("parts:item:0_0_0")).toThrow();
    expect(p.result.diagnostics).toEqual([]);
    expect(p.result.wireDiagnostics).toEqual([]);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});
