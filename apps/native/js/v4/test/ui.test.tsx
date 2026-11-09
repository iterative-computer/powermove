import { test, expect } from "bun:test";
import { recorder } from "./helpers";
import * as UI from "../src/ui";
import * as Kit from "../src/components";
import { parseArithmetic } from "../src/arithmetic";
import { json } from "../src/protocol";
test("all tagged public controls and new compositional controls export native semantic trees", async () => {
  const { root } = recorder();
  await root.render(
    <div>
      <UI.BackTitle title="Back" onBack={() => {}} />
      <UI.Disclosure label="Layer" open>
        body
      </UI.Disclosure>
      <UI.Row label="Value">
        <UI.NumField value={1} binding="n" />
      </UI.Row>
      <UI.Section title="Group" />
      <UI.ColorField value="#abcdef" binding="c" />
      <UI.FillField value={{ type: "none" }} binding="f" />
      <UI.FontField value="System" binding="font" options={["System"]} />
      <UI.PointField value={[1, 2]} binding="p" xBinding="x" yBinding="y" />
      <UI.RampField
        value={[{ id: "a", color: "#000", position: 0 }]}
        binding="r"
      />
      <UI.Segmented
        options={[{ id: "a", label: "A" }]}
        value="a"
        onChange={() => {}}
      />
      <UI.SelectField
        value={null}
        binding="s"
        options={[{ v: null, label: "None" }]}
      />
      <UI.SliderField value={30} binding="angle" angle />
      <UI.TextField value="text" binding="t" />
      <UI.ToggleField value={false} binding="toggle" />
      <UI.FieldMenu
        anchor={1}
        open
        items={[{ id: "a", label: "A", onSelect: () => {} }]}
      />
      <UI.ToolbarItem label="Run" />
      <UI.Glass surface="Toolbar" />
    </div>,
  );
  const kinds = [...root.host.nodes.values()]
    .map((n) => n.props.control)
    .filter(Boolean);
  for (const kind of [
    "back-title",
    "disclosure",
    "row",
    "section",
    "number",
    "color",
    "fill",
    "font",
    "point",
    "ramp",
    "segmented",
    "select",
    "angle",
    "text",
    "toggle",
    "field-menu",
    "menu",
    "toolbar-item",
  ])
    expect(kinds).toContain(kind);
});
test("menu navigation skips disabled entries and activates typed callback", async () => {
  const { root } = recorder();
  const selected: string[] = [];
  await root.render(
    <UI.Menu
      items={[
        { id: "a", label: "A", onSelect: () => selected.push("a") },
        {
          id: "b",
          label: "B",
          disabled: true,
          onSelect: () => selected.push("b"),
        },
        { id: "c", label: "C", onSelect: () => selected.push("c") },
      ]}
    />,
  );
  const menu = root.host.root.firstChild!;
  const event = (key: string) =>
    root.dispatch(
      {
        revision: root.inspect().revision,
        target: menu.id,
        preventDefault: true,
        deliveries: [{ id: menu.id, capture: false }],
      },
      "keydown",
      { key },
    );
  await event("ArrowDown");
  await event("Enter");
  expect(selected).toEqual(["c"]);
});
test("numeric parser retains tagged arithmetic intent and rejects code/nonfinite input", () => {
  expect(parseArithmetic("(2+3)*4")).toBe(20);
  expect(parseArithmetic("-2")).toBe(-2);
  for (const value of ["alert(1)", "1/0", "1e309", "1;2", ""])
    expect(Number.isNaN(parseArithmetic(value))).toBe(true);
});
test("JSON serializer rejects nonfinite, cyclic data, accessors and class instances", () => {
  let touched = false;
  const input = {
    get value() {
      touched = true;
      return 1;
    },
  };
  expect(() => json(input)).toThrow("accessors");
  expect(touched).toBe(false);
  const cycle: { self?: unknown } = {};
  cycle.self = cycle;
  for (const value of [NaN, Infinity, cycle, new Date(), 1n])
    expect(() => json(value)).toThrow();
});
test("a field takes the label of either kind of row", async () => {
  const { root } = recorder();
  await root.render(
    <div>
      <UI.Row label="Public">
        <Kit.NumberField id="kit" value={1} binding="k" />
      </UI.Row>
      <Kit.InspectorRow label="Kit">
        <UI.NumField id="public" value={1} binding="p" />
      </Kit.InspectorRow>
    </div>,
  );
  const nodes = [...root.host.nodes.values()];
  for (const field of ["kit", "public"]) {
    const labelledBy = nodes.find((n) => n.props.id === field)!.props
      .labelledBy;
    expect(nodes.find((n) => n.props.id === labelledBy)?.type).toBe("text");
  }
});
