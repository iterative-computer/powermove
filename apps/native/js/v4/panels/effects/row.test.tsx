import { expect, test } from "bun:test";
import { Evaluation } from "../../eval/harness";
import { fixture, itemRow } from "./fixtures";
import { ids } from "./model";
import { EffectsPanel } from "./panel";

const bindings = {
  search: "local:effects:search",
  scope: "local:effects:scope",
};

test("effect row tooltip describes double-clicking to apply", async () => {
  const { model } = fixture();
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    expect(e.node(ids.row(itemRow(model).key)).props.title).toBe(
      "Drag onto a layer, or double-click to apply to the selected layer.",
    );
  } finally {
    model.dispose();
    await e.dispose();
  }
});

test("double-click applies only when a layer is selected", async () => {
  const { model, applied } = fixture();
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    const row = ids.row(itemRow(model).key);
    await e.event(row, "doubleclick");
    expect(applied).toHaveLength(0);
    expect(model.announcement.text).toBe("Select a layer first");
    model.setTarget({ id: "layer", effects: [] });
    await e.event(row, "doubleclick");
    expect(applied).toHaveLength(1);
  } finally {
    model.dispose();
    await e.dispose();
  }
});

test("double-click without a target displays a status hint", async () => {
  const { model } = fixture();
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    await e.event(ids.row(itemRow(model).key), "doubleclick");
    const status = e.nodes().find((node) => node.props.role === "status");
    expect(status?.props.label).toBe("Select a layer first");
  } finally {
    model.dispose();
    await e.dispose();
  }
});

test("drag history can use the effect name independently of its accessibility label", async () => {
  const { model } = fixture();
  model.replaceRegistry({
    items: [
      {
        id: "acceptance-tint.effect",
        label: "Acceptance Tint",
        group: "Stylize",
        kind: "effect",
        installed: true,
      },
    ],
    groups: ["Stylize"],
  });
  const e = new Evaluation(process.env.PM_SURFACE_WIRE);
  try {
    await e.render(<EffectsPanel model={model} bindings={bindings} />);
    const { props } = e.node(
      ids.row(itemRow(model, "acceptance-tint.effect").key),
    );
    expect(props.metadata).toMatchObject({ dragLabel: "Acceptance Tint" });
    expect(props.label).toBe("Acceptance Tint, effect, Stylize");
  } finally {
    model.dispose();
    await e.dispose();
  }
});
