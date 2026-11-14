import { expect, test } from "bun:test";
import { createRoot, type EventDetail, type SurfaceRoot } from "../../src";
import { PANEL_ID } from "./contract";
import type { ColorModel, Intent, Projection } from "./model";
import { ColorPanel } from "./panel";

/** A controller that records what the panel asks of it; its menus answer
 * with `choices`, in turn. */
function controller(p: Projection, choices: string[]) {
  const sent: Intent[] = [];
  const menus: { id: string; label: string; disabled?: boolean }[][] = [];
  const c = {
    snapshot: () => p,
    subscribe: () => () => {},
    send: (intent: Intent) => sent.push(intent),
    controlMenu: async () => {},
    platform: {
      contextMenu: async (items: { id: string; label: string }[]) => {
        menus.push(items);
        return choices.shift() ?? null;
      },
      report: (error: unknown) => {
        throw error;
      },
    },
  } as unknown as ColorModel;
  return { c, sent, menus };
}

async function mounted(p: Projection, choices: string[] = []) {
  const { c, sent, menus } = controller(p, choices);
  const commands: string[] = [];
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  await root.render(
    <ColorPanel controller={c} command={(name) => commands.push(name)} />,
  );
  return { root, sent, menus, commands };
}

/** Lets a menu's answer arrive. */
const settle = () => new Promise((r) => setTimeout(r, 0));

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

// C2: grades stack. The header's Add menu puts the new grade after the last
// one; the panel lists grades in the order they apply, named apart, and a
// grade's heading menu (a right click) moves it past any other effect
// between them (the effect stack's order).
test("grades stack in the order they apply, named apart, and move", async () => {
  const at = (h: ReturnType<typeof tool>, ordinal: number) => ({
    ...h,
    ordinal,
  });
  const blur = { ...tool("b", "gaussianBlur", "Gaussian Blur"), grade: false };
  const p = stacked([
    at(tool("w1", "colorWheels", "Color Wheels"), 0),
    at(blur as ReturnType<typeof tool>, 1),
    at(tool("l1", "applyColorLut", "Apply Color LUT"), 2),
    at(tool("w2", "colorWheels", "Color Wheels"), 3),
  ]);
  const { root, sent, menus, commands } = await mounted(p, [
    "colorWheels",
    "colorCurves",
    "lut",
    "up",
    "down",
    null as unknown as string,
  ]);
  const grades = [...root.host.nodes.values()]
    .filter((n) => n.props["data-pm-component"] === "disclosure-group")
    .map((n) => n.props.label);
  expect(grades).toEqual(["Color Wheels", "Apply Color LUT", "Color Wheels 2"]);
  const add = named(root, `${PANEL_ID}:add`).id;
  for (let i = 0; i < 3; i++) {
    await event(root, add, "click");
    await settle();
  }
  expect(menus[0].filter(Boolean).map((i) => i.label)).toEqual([
    "Color Wheels",
    "Curve",
    "Color Space Transform",
    "LUT…",
    "CDL…",
  ]);
  expect(commands).toEqual(["viewer.applyLut"]);
  // The LUT moves up past the blur to before the first wheels' successor
  // slot, and down after the second wheels; the first can't go up.
  const heading = (id: string) => named(root, `${PANEL_ID}:grade:${id}`).id;
  await event(root, heading("l1"), "pointerdown", { button: 2 });
  await settle();
  await event(root, heading("l1"), "pointerdown", { button: 2 });
  await settle();
  await event(root, heading("w1"), "pointerdown", { button: 2 });
  await settle();
  expect(menus.at(-1)!.find((i) => i?.id === "up")!.disabled).toBe(true);
  expect(sent.filter((i) => i.type === "add" || i.type === "drop")).toEqual([
    { type: "add", definition: "colorWheels", slot: 4 },
    {
      type: "add",
      definition: "colorCurves",
      slot: 4,
      parameters: { channel: 0 },
    },
    { type: "drop", id: "l1", slot: 0 },
    { type: "drop", id: "l1", slot: 4 },
  ]);
  await root.dispose();
});

// C3/C4: a grade's box turns it off and on; its name folds it, as a
// Properties section's does; and a grade that is off can still be set up:
// its wheels and curve stay editable.
test("a grade's name folds it, and an off grade stays editable", async () => {
  const off = { ...tool("w", "colorWheels", "Color Wheels"), enabled: false };
  const rows = [
    off,
    ...["Red", "Green", "Blue"].map((c) => wheelRow(`lift${c}`)),
  ];
  const { root, sent } = await mounted({
    ...stacked([off]),
    count: rows.length,
    range: [0, rows.length],
    rows,
  } as Projection);
  expect(named(root, `${PANEL_ID}:wheel:w:lift`).props.disabled).toBe(false);
  const button = [...root.host.nodes.values()].find(
    (n) =>
      n.props["data-pm-component"] === "disclosure-button" &&
      n.props.label === "Color Wheels",
  )!;
  await event(root, button.id, "click");
  expect(
    [...root.host.nodes.values()].some(
      (n) => n.props.id === `${PANEL_ID}:wheel:w:lift`,
    ),
  ).toBe(false);
  expect(sent.filter((i) => i.type === "toggle")).toEqual([]);
  await root.dispose();
});

function wheelRow(key: string) {
  return {
    kind: "parameter" as const,
    id: `parameter:w.${key}`,
    enabled: false,
    parameter: {
      id: `w.${key}`,
      effect: "w",
      key,
      label: key,
      binding: `b:w.${key}`,
      value: 0,
      control: "number" as const,
      animated: false,
      keyAtPlayhead: false,
      mixed: false,
      readonly: false,
    },
  };
}

// C4: Color Space Transform's two Gamut and two Transfer rows stand under
// Input and Output, so the two pairs can be told apart.
test("a Color Space Transform's rows stand under Input and Output", async () => {
  const cst = tool("t", "colorSpaceTransform", "Color Space Transform");
  const row = (key: string, label: string) => ({
    ...wheelRow(key),
    id: `parameter:t.${key}`,
    parameter: {
      ...wheelRow(key).parameter,
      id: `t.${key}`,
      effect: "t",
      label,
    },
  });
  const rows = [
    cst,
    row("inputGamut", "Gamut"),
    row("inputTransfer", "Transfer"),
    row("outputGamut", "Gamut"),
    row("outputTransfer", "Transfer"),
    row("extra", "Extra"),
  ];
  const { root } = await mounted({
    ...stacked([cst]),
    count: rows.length,
    range: [0, rows.length],
    rows,
  } as Projection);
  const groups = [...root.host.nodes.values()].filter(
    (n) =>
      n.props.role === "group" && ["Input", "Output"].includes(n.props.label),
  );
  expect(groups.map((g) => g.props.label)).toEqual(["Input", "Output"]);
  // A row no part names is still shown.
  expect(
    [...root.host.nodes.values()].some(
      (n) => n.props.id === `${PANEL_ID}:field:parameter:t.extra`,
    ),
  ).toBe(true);
  await root.dispose();
});
