import { expect, test } from "bun:test";
import { EffectControls } from "../panel";
import { states, selected } from "./fixtures";
import { fixture, event, find, nodes, wire } from "./helpers";
import { projection } from "../contract";
import type { Json } from "../../../src";

for (const [name, value] of Object.entries(states())) {
  test(`spec §3 ${name}: native op-tree and accessibility snapshot`, async () => {
    const f = await fixture(value);
    await f.root.render(
      <EffectControls controller={f.controller} rowHeight={24} />,
    );
    const result = wire(f.commits);
    expect(result.diagnostics).toEqual([]);
    expect(result.accessibilityNodes).toBeGreaterThan(1);
    const data =
      JSON.stringify({
        ops: f.commits,
        tree: result.nodes,
        focusOrder: result.focusOrder,
        accessibilityNodes: result.accessibilityNodes,
        accessibility: result.accessibility,
      }) + "\n";
    const path = new URL(`../snapshots/${name}.json`, import.meta.url);
    if (process.env.UPDATE_EFFECT_SNAPSHOTS === "1")
      await Bun.write(path, data);
    expect(data).toBe(await Bun.file(path).text());
    const list = nodes(f.root).find((n) => n.props.id === "layer-effects:list");
    if (list) {
      expect(list.props.role).toBe("list");
      expect(
        (list.props.metadata as Record<string, Json>).multiselectable,
      ).toBe(true);
    }
    if (name === "60k") {
      expect(nodes(f.root).length).toBeLessThan(80);
      expect(find(f.root, (p) => p.count === 60_000).props.range).toEqual([
        30_000, 30_003,
      ]);
    }
    if (name === "disabled")
      expect(find(f.root, (p) => p.binding === "radius").props.disabled).toBe(
        false,
      );
    if (name === "locked") expect(result.focusOrder.length).toBeLessThan(4);
    if (name === "error") {
      expect(nodes(f.root).some((n) => n.textContent === "Not installed")).toBe(
        true,
      );
      expect(nodes(f.root).some((n) => n.props.binding === "radius")).toBe(
        false,
      );
    }
    if (name === "loading-na")
      expect(nodes(f.root).some((n) => n.props.role === "progressbar")).toBe(
        false,
      );
    await f.root.dispose();
    await f.controller.dispose();
  });
}
for (const name of [
  "hover",
  "focus",
  "drag-reorder",
  "drop-new",
  "increase-contrast",
  "reduce-motion",
]) {
  test(`spec §3/7 ${name}: native state snapshot`, async () => {
    const p = selected();
    if (name === "hover") {
      p.selectedCount = 0;
      p.rows.forEach((r) => {
        if (r.kind === "effect") r.selected = false;
      });
    }
    const f = await fixture(p);
    await f.root.render(
      <EffectControls
        controller={f.controller}
        rowHeight={24}
        keyboardFocus={name === "focus"}
        increaseContrast={name === "increase-contrast"}
        reduceMotion={name === "reduce-motion"}
      />,
    );
    const header = find(f.root, (p) => p.id === "layer-effects:effect:f1");
    if (name === "hover") await event(f.root, header.id, "pointermove");
    if (name === "focus")
      await event(
        f.root,
        find(f.root, (p) => p.id === "layer-effects:list").id,
        "focus",
      );
    if (name === "drag-reorder") await event(f.root, header.id, "dragstart");
    if (name === "drop-new")
      await event(f.root, header.id, "dragover", {
        drag: {
          types: ["application/x-powermove-effect-definition"],
          data: { "application/x-powermove-effect-definition": "blur" },
          effect: "copy",
        },
      });
    const result = wire(f.commits);
    expect(result.diagnostics).toEqual([]);
    if (name === "focus")
      expect(
        header.props.metadata &&
          (header.props.metadata as Record<string, Json>).focused,
      ).toBe(true);
    const data =
      JSON.stringify({
        ops: f.commits,
        tree: result.nodes,
        focusOrder: result.focusOrder,
        accessibilityNodes: result.accessibilityNodes,
        accessibility: result.accessibility,
      }) + "\n";
    const path = new URL(`../snapshots/${name}.json`, import.meta.url);
    if (process.env.UPDATE_EFFECT_SNAPSHOTS === "1")
      await Bun.write(path, data);
    expect(data).toBe(await Bun.file(path).text());
    await f.root.dispose();
    await f.controller.dispose();
  });
}
test("§2/5/6 one list stop, stable value column, exact names and separate parameter controls", async () => {
  const f = await fixture(selected());
  await f.root.render(
    <EffectControls controller={f.controller} rowHeight={24} />,
  );
  const list = find(f.root, (p) => p.id === "layer-effects:list");
  expect(list.props.tabIndex).toBe(0);
  for (const row of nodes(f.root).filter((n) => n.props.role === "listitem"))
    expect(row.props.tabIndex).toBe(-1);
  const row = find(f.root, (p) => p["data-pm-component"] === "inspector-row");
  const value = nodes(f.root).find(
    (n) => n.parentNode === row && n.props["data-pm-slot"] === "value",
  )!;
  expect((value.props.style as Record<string, Json>).width).toEqual({
    token: "metrics.value_column",
  });
  expect(find(f.root, (p) => p.label === "Enable Glow").props.role).toBe(
    "checkbox",
  );
  expect(find(f.root, (p) => p.label === "Reset Glow").props.title).toBe(
    "Reset Effect",
  );
  expect(find(f.root, (p) => p.label === "Remove Glow").props.title).toBe(
    "Remove Effect",
  );
  expect(find(f.root, (p) => p.label === "Keyframe, Radius").props.title).toBe(
    "not animated",
  );
  await f.root.dispose();
  await f.controller.dispose();
});
test("model projection rejects invalid windows, duplicate ids, values and control schemas", () => {
  for (const mutate of [
    (p: any) => (p.range = [0, 99]),
    (p: any) => (p.rows[1].id = "f1"),
    (p: any) => (p.rows[1].parameter.value = false),
    (p: any) => (p.rows[1].parameter.control = "made-up"),
    (p: any) => (p.selectedCount = 9),
  ]) {
    const p = selected();
    mutate(p);
    expect(() => projection(p as unknown as Json)).toThrow();
  }
});

test("§6 native AccessKit exposes selected rows, checkbox state, and numeric spoken units", async () => {
  const f = await fixture(selected());
  await f.root.render(
    <EffectControls controller={f.controller} rowHeight={24} />,
  );
  const ax = wire(f.commits).accessibility;
  expect(ax.find((n: any) => n.label === "Effects")).toMatchObject({
    role: "List",
  });
  expect(
    ax.find((n: any) => n.role === "ListItem" && n.selected),
  ).toMatchObject({ label: "Glow effect, enabled, expanded, 1 of 2" });
  expect(ax.find((n: any) => n.label === "Enable Glow")).toMatchObject({
    role: "CheckBox",
    toggled: "Some(True)",
  });
  expect(ax.find((n: any) => n.role === "NumberInput")).toMatchObject({
    label: "Radius",
    numericValue: 24,
    value: "24 pixels",
  });
  await f.root.dispose();
  await f.controller.dispose();
});
