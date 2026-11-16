import { expect, test } from "bun:test";
import { InspectorSession } from "../session";
import { fixture } from "./fixture";
import type { Json } from "../../../src";

function menuFixture() {
  const f = fixture();
  let resolve!: (choice: Json | void) => void;
  const session = new InspectorSession(
    f.host,
    f.model,
    {
      request: () =>
        new Promise((done) => {
          resolve = done;
        }),
    },
    f.clock,
    (error) => f.errors.push(error),
  );
  return { ...f, session, choose: (value: Json | void) => resolve(value) };
}
async function flush() {
  for (let i = 0; i < 12; i++) await Promise.resolve();
}

test("Properties Add Effect dispatches the chosen definition through its model", async () => {
  const f = menuFixture();
  await f.session.start();
  f.session.nativeRequest({ type: "add-effect-menu", anchor: 1, target: "a" });
  f.choose("synthetic.glow");
  await flush();
  expect(f.model.intents).toEqual([
    { type: "drop-effect", effect: "synthetic.glow", target: "a" },
  ]);
  expect(f.errors).toEqual([]);
  await f.session.dispose();
});

test("cancelled Add Effect and results after selection/document replacement never edit", async () => {
  for (const reason of ["cancel", "replacement", "locked", "retire"] as const) {
    const f = menuFixture();
    await f.session.start();
    f.session.nativeRequest({
      type: "add-effect-menu",
      anchor: 1,
      target: "a",
    });
    if (reason === "replacement" && f.model.value.target.kind === "layers") {
      f.model.value.target.key = "new-document-same-layer-id";
      f.host.publish({
        ...f.host.identity,
        version: 1,
        subscription: "inspector:evaluated",
        revision: 0,
        sequence: 1,
        value: null,
      });
      await f.session.settled();
    }
    if (reason === "locked" && f.model.value.target.kind === "layers") {
      f.model.value.target.first.locked = true;
      f.host.publish({
        ...f.host.identity,
        version: 1,
        subscription: "inspector:evaluated",
        revision: 0,
        sequence: 1,
        value: null,
      });
      await f.session.settled();
    }
    if (reason === "retire") await f.session.dispose();
    f.choose(reason === "cancel" ? null : "synthetic.glow");
    await flush();
    expect(f.model.intents).toEqual([]);
    await f.session.dispose();
  }
});

test("row menu accepts only enabled authored actions and rejects unknown choices", async () => {
  for (const [choice, expected] of [
    ["reset", 1],
    ["remove-animation", 0],
    ["foreign", 0],
  ] as const) {
    const f = menuFixture();
    await f.session.start();
    f.session.nativeRequest({
      type: "row-menu",
      anchor: 1,
      row: "opacity",
      items: [
        { label: "Reset", action: "reset", enabled: true },
        {
          label: "Remove Animation",
          action: "remove-animation",
          enabled: false,
        },
      ],
    });
    f.choose(choice);
    await flush();
    expect(f.model.intents).toHaveLength(expected);
    await f.session.dispose();
  }
});

test("Position menu has Electron dimension and easing labels with lock guards", async () => {
  const { rowMenu } = await import("../property-menu");
  const base = {
    id: "position",
    label: "Position",
    control: { kind: "status" as const, value: "" },
    animation: "animated" as const,
    separateDimensions: true,
  };
  const items = rowMenu(base);
  expect(items.find((i) => i.label === "Separate Dimensions")?.checked).toBe(
    true,
  );
  expect(
    items.filter((i) => i.action.startsWith("ease:")).map((i) => i.label),
  ).toEqual([
    "Easing for All Keys",
    "Power",
    "Linear",
    "Ease In and Out",
    "Exponential Out",
    "Back Out",
    "Glide",
    "Snap",
  ]);
  expect(
    rowMenu({ ...base, animation: "static" }).some((i) =>
      i.action.startsWith("ease:"),
    ),
  ).toBe(false);
  expect(
    rowMenu({ ...base, locked: true })
      .filter(
        (i) =>
          i.action === "separate-dimensions" || i.action.startsWith("ease:"),
      )
      .every((i) => !i.enabled),
  ).toBe(true);
});

test("property menu offers Save Animation Preset", async () => {
 const {rowMenu}=await import("../property-menu");
 expect(rowMenu({id:"opacity",label:"Opacity",control:{kind:"readonly",text:"100"}} as any).some(item=>item.action==="save-preset" && item.enabled)).toBe(true);
});
