import { expect, test } from "bun:test";
import { createRoot, type EventDetail, type SurfaceRoot } from "../../src";
import { PANEL_ID } from "./contract";
import type { ColorModel, Intent, Projection } from "./model";
import { ColorPanel } from "./panel";

/** A controller that records what the panel asks of it. */
function controller(p: Projection) {
  const sent: Intent[] = [];
  const c = {
    snapshot: () => p,
    subscribe: () => () => {},
    send: (intent: Intent) => sent.push(intent),
    controlMenu: async () => {},
  } as unknown as ColorModel;
  return { c, sent };
}

async function mounted(p: Projection) {
  const { c, sent } = controller(p);
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  await root.render(<ColorPanel controller={c} command={() => {}} />);
  return { root, sent };
}

function named(root: SurfaceRoot, name: string) {
  const n = [...root.host.nodes.values()].find((n) => n.props.id === name);
  if (!n) throw new Error(`missing ${name}`);
  return n;
}

async function event(
  root: SurfaceRoot,
  id: number,
  type: string,
  detail: EventDetail = {},
) {
  await root.dispatch(
    {
      revision: root.inspect().revision,
      target: id,
      preventDefault: true,
      deliveries: [{ id, capture: false }],
    },
    type,
    detail,
  );
}

const wheels = {
  kind: "effect" as const,
  id: "w",
  definition: "colorWheels",
  grade: true,
  label: "Color Wheels",
  enabled: true,
  expanded: true,
};

const projection = {
  target: "Layer",
  layer: "L",
  name: "Clip",
  typeLabel: "Video",
  locked: false,
  revision: 3,
  documentId: 1,
  count: 1,
  range: [0, 1],
  rows: [wheels],
} as Projection;

// A person's click on the checkbox, and the agent's, arrive as the box's
// change (the AppKit checkbox's action; the drawn box's click): it turns the
// tool off or on. Before, only a click event toggled, which no host sends
// for a checkbox, so the box did nothing.
test("the enable checkbox turns its tool off and on", async () => {
  const { root, sent } = await mounted(projection);
  const box = named(root, `${PANEL_ID}:on:w`);
  await event(root, box.id, "change", { value: false });
  expect(sent.filter((i) => i.type === "toggle")).toEqual([
    { type: "toggle", id: "w" },
  ]);
  await root.dispose();
});

const tool = (id: string, definition: string, label = definition) => ({
  kind: "effect" as const,
  id,
  definition,
  grade: true,
  label,
  enabled: true,
  expanded: true,
});

const stacked = (headers: ReturnType<typeof tool>[]) =>
  ({
    ...projection,
    count: headers.length,
    range: [0, headers.length],
    rows: headers,
  }) as Projection;

// C1: every grade can be removed from its own header, the first Color
// Wheels included (it had Reset but no Remove).
test("every grade, the first Color Wheels included, has a Remove", async () => {
  const grades = [
    tool("w1", "colorWheels", "Color Wheels"),
    tool("c1", "colorCurves", "Color Curves"),
    tool("l1", "applyColorLut", "Apply Color LUT"),
    tool("t1", "colorSpaceTransform", "Color Space Transform"),
  ];
  const { root, sent } = await mounted(stacked(grades));
  for (const g of grades) {
    const remove = named(root, `${PANEL_ID}:remove:${g.id}`);
    await event(root, remove.id, "click");
  }
  expect(sent.filter((i) => i.type === "remove")).toEqual(
    grades.map((g) => ({ type: "remove", id: g.id })),
  );
  await root.dispose();
});
