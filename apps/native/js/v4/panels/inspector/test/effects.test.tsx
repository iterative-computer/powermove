import { expect, test } from "bun:test";
import { InspectorPanel } from "../view";
import { fixture, snapshot } from "./fixture";
import { Probe } from "./probe";
import { fixture as effectsFixture } from "../../effect-controls/test/helpers";
import { selected } from "../../effect-controls/test/fixtures";

test("Properties embeds the shared effect rows, fields and keyframes with heading add", async () => {
  const state = snapshot();
  state.sections.push({
    id: "effects",
    title: "Effects",
    collapsed: false,
    rows: [],
  });
  const inspector = fixture(state);
  const effects = await effectsFixture(selected());
  let menus = 0;
  effects.kit.api.effectMenu = async () => {
    menus++;
    return "blur";
  };
  await inspector.session.start();
  const p = new Probe();
  await p.render(
    <InspectorPanel
      session={inspector.session}
      effects={{ controller: effects.controller, rowHeight: 24 }}
    />,
  );
  expect(p.node("section:effects")).toBeDefined();
  expect(p.node("layer-effects:effect:f1")).toBeDefined();
  expect(p.node("layer-effects:field:radius").props.binding).toBe("radius");
  expect(
    Object.values(p.result.nodes).some(
      (n) => n.props.label === "Keyframe, Radius",
    ),
  ).toBe(true);
  expect(
    Object.values(p.result.nodes).some((n) => n.type === "virtual-list"),
  ).toBe(false);
  await p.drive("inspector:add-effect", "click");
  await effects.controller.settled();
  expect(menus).toBe(1);
  expect(
    effects.requests.some(
      (r: any) =>
        r.command === "panel.effect-controls" && r.args.intent.type === "add",
    ),
  ).toBe(true);
  await p.dispose();
  await inspector.session.dispose();
  await effects.controller.dispose();
  await effects.root.dispose();
});
