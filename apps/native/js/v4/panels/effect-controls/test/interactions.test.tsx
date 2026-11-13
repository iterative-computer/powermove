import { expect, test } from "bun:test";
import { createRoot } from "../../../src";
import { EffectControls } from "../panel";
import { ModelHost } from "./model-host";
import { event, find } from "./helpers";
async function mount(h: ModelHost) {
  await h.start();
  const root = createRoot(async (commit) => ({
    accepted: true,
    revision: commit.baseRevision + 1,
  }));
  await root.render(
    <EffectControls controller={h.controller} rowHeight={24} />,
  );
  const settle = async () => {
    await h.controller.settled();
    await root.flush();
  };
  return { root, settle };
}
test("§4 header selection, disclosure, checkbox, reset/remove, and outside content dispatch to the actual model", async () => {
  const h = new ModelHost();
  const { root, settle } = await mount(h);
  const c = h.controller;
  try {
    const header = () => find(root, (p) => p.id === "layer-effects:effect:f1");
    await event(root, header().id, "click");
    await settle();
    expect(c.snapshot().rows[0]).toMatchObject({
      selected: true,
      expanded: true,
    });
    const chevron = find(
      root,
      // An expanded heading's disclosure points down.
      (p) => (p.attr as any)?.symbol === "chevron.down",
    );
    await event(root, chevron.id, "click", {}, [header().id]);
    await settle();
    expect(c.snapshot().rows[0]).toMatchObject({
      selected: true,
      expanded: false,
    });
    await event(
      root,
      find(root, (p) => p.label === "Enable Glow").id,
      "click",
      {},
      [header().id],
    );
    await settle();
    expect(c.snapshot().rows[0]).toMatchObject({
      selected: true,
      expanded: false,
      enabled: false,
    });
    await event(
      root,
      find(root, (p) => p.label === "Reset Glow").id,
      "click",
      {},
      [header().id],
    );
    await settle();
    expect(c.snapshot().rows[0]).toMatchObject({ selected: true });
    const panel = find(root, (p) => p.id === "layer-effects");
    const identity = find(root, (p) => p.role === "heading");
    await event(root, identity.id, "click", {}, [panel.id]);
    await settle();
    expect(c.snapshot().selectedCount).toBe(0);
    await event(root, header().id, "click");
    await settle();
    await event(
      root,
      find(root, (p) => p.label === "Remove Glow").id,
      "click",
      {},
      [header().id],
    );
    await settle();
    expect(c.snapshot().effectCount).toBe(2);
    expect(h.kit.announcements).toContain("Removed Glow");
  } finally {
    await root.dispose();
    await h.close();
  }
});
test("§4 UI drag/drop inserts before a header; Escape cancels and foreign reorder ids are ignored", async () => {
  const h = new ModelHost();
  const { root, settle } = await mount(h);
  const c = h.controller;
  try {
    const header = (id: string) =>
      find(root, (p) => p.id === `layer-effects:effect:${id}`);
    const list = () => find(root, (p) => p.id === "layer-effects:list");
    await event(root, header("f1").id, "dragstart");
    await event(root, header("f3").id, "dragover");
    await event(root, list().id, "drop");
    await settle();
    await c.drive({ type: "range", range: [0, 32] });
    await root.flush();
    expect(
      c
        .snapshot()
        .rows.filter((r) => r.kind === "effect")
        .map((r) => r.id),
    ).toEqual(["f2", "f1", "f3"]);
    await event(root, header("f1").id, "dragstart");
    await event(root, list().id, "keydown", { key: "Escape" });
    await settle();
    await event(root, list().id, "drop");
    await settle();
    expect(c.snapshot().effectCount).toBe(3);
    await event(root, header("f2").id, "dragover", {
      drag: {
        types: ["application/x-powermove-effect"],
        data: { "application/x-powermove-effect": "f1" },
        effect: "move",
      },
    });
    await event(root, list().id, "drop");
    await settle();
    expect(c.snapshot().effectCount).toBe(3);
    await event(root, header("f2").id, "dragover", {
      drag: {
        types: ["application/x-powermove-effect-definition"],
        data: { "application/x-powermove-effect-definition": "glow" },
        effect: "copy",
      },
    });
    await event(root, list().id, "drop");
    await settle();
    expect(c.snapshot().effectCount).toBe(4);
    await c.drive({ type: "range", range: [0, 32] });
    expect(c.snapshot().rows[0].id).toBe(c.snapshot().focus!);
  } finally {
    await root.dispose();
    await h.close();
  }
});
test("§4 both Add controls and native row menu preserve the real model clipboard context", async () => {
  const h = new ModelHost();
  const { root, settle } = await mount(h);
  const c = h.controller;
  try {
    h.kit.api.effectMenu = async () => "glow";
    await event(
      root,
      find(root, (p) => p.id === "layer-effects:add").id,
      "click",
    );
    await settle();
    expect(c.snapshot().effectCount).toBe(4);
    await event(
      root,
      find(root, (p) => p.id === "layer-effects:add-bottom").id,
      "click",
    );
    await settle();
    expect(c.snapshot().effectCount).toBe(5);
    await c.drive({ type: "range", range: [0, 64] });
    await root.flush();
    const header = find(root, (p) => p.id === "layer-effects:effect:f1");
    await event(root, header.id, "click");
    await settle();
    h.kit.api.contextMenu = async () => "copy";
    await event(root, header.id, "pointerdown", { button: 2 });
    await settle();
    expect(c.snapshot().clipboardCount).toBe(1);
    const list = () => find(root, (p) => p.id === "layer-effects:list");
    await event(root, list().id, "keydown", { key: "v", ctrlKey: true });
    await settle();
    expect(c.snapshot().effectCount).toBe(6);
    await event(root, list().id, "keydown", { key: "Delete" });
    await settle();
    expect(c.snapshot().effectCount).toBe(5);
  } finally {
    await root.dispose();
    await h.close();
  }
});
