import { expect, test } from "bun:test";
import { createRoot, type Json } from "../../../src";
import { EffectControls } from "../panel";
import { ModelHost } from "./model-host";
import { event, find } from "./helpers";
const click = (id: string, shift = false, meta = false) => ({
  type: "select" as const,
  id,
  shift,
  meta,
  ctrl: false,
});

test("B285/B379 selection, range, toggle, disclosure, outside and nested controls use the Rust model", async () => {
  const h = new ModelHost();
  try {
    await h.start();
    const c = h.controller;
    await c.drive(click("f1"));
    expect(c.snapshot().rows[0]).toMatchObject({
      selected: true,
      expanded: true,
    });
    await c.drive({ type: "disclose", id: "f1", open: false });
    expect(c.snapshot().rows[0]).toMatchObject({
      selected: true,
      expanded: false,
    });
    await c.drive(click("f2", false, true));
    expect(c.snapshot().selectedCount).toBe(2);
    await c.drive(click("f3", true));
    expect(c.snapshot().selectedCount).toBe(2);
    await c.drive({ type: "clear" });
    expect(c.snapshot().selectedCount).toBe(0);
    await c.drive(click("f1"));
    const root = createRoot(async (commit) => ({
      accepted: true,
      revision: commit.baseRevision + 1,
    }));
    await root.render(<EffectControls controller={c} rowHeight={24} />);
    const checkbox = find(root, (p) => p.label === "Enable Glow");
    const header = find(root, (p) => p.id === "layer-effects:effect:f1");
    await event(root, checkbox.id, "click", {}, [header.id]);
    await c.drive({ type: "range", range: [0, 32] });
    expect(c.snapshot().selectedCount).toBe(1);
    expect(c.snapshot().rows[0]).toMatchObject({
      expanded: false,
      enabled: false,
    });
    await root.dispose();
  } finally {
    await h.close();
  }
});
test("B338/B371/B372 clipboard persists across destination selection and paste/delete undo atomically", async () => {
  const h = new ModelHost();
  try {
    await h.start();
    const c = h.controller;
    await c.drive(click("f1"));
    await c.drive(click("f2", false, true));
    await c.drive({ type: "copy", id: "f1" });
    expect(c.snapshot().clipboardCount).toBe(2);
    await h.select("b");
    expect(c.snapshot().layer).toBe("b");
    await c.drive({ type: "paste" });
    expect(c.snapshot().effectCount).toBe(2);
    expect(c.snapshot().selectedCount).toBe(2);
    const ids = c
      .snapshot()
      .rows.filter((r) => r.kind === "effect")
      .map((r) => r.id);
    expect(ids.every((id) => !["f1", "f2"].includes(id))).toBe(true);
    expect(h.kit.announcements).toContain("Pasted 2 effects");
    await h.line({ history: "undo" });
    await h.publish();
    expect(c.snapshot().effectCount).toBe(0);
    await h.line({ history: "redo" });
    await h.publish();
    expect(c.snapshot().effectCount).toBe(2);
    await c.drive(click(ids[0]));
    await c.drive(click(ids[1], false, true));
    await c.drive({ type: "remove", id: ids[0] });
    expect(c.snapshot().effectCount).toBe(0);
    await h.line({ history: "undo" });
    await h.publish();
    expect(c.snapshot().effectCount).toBe(2);
  } finally {
    await h.close();
  }
});
test("B367/B368 parameter animation and reset reach one kernel transaction, precise defaults survive Undo", async () => {
  const h = new ModelHost();
  try {
    await h.start();
    const c = h.controller;
    const param = c.snapshot().rows.find((r) => r.kind === "parameter")!;
    if (param.kind !== "parameter") throw new Error("parameter absent");
    await c.drive({
      type: "parameter",
      id: param.parameter.id,
      action: "diamond",
    });
    expect(c.snapshot().rows.find((r) => r.id === param.id)).toMatchObject({
      parameter: { animated: true, keyAtPlayhead: true },
    });
    await c.drive({ type: "reset", id: "f1" });
    expect(c.snapshot().rows.find((r) => r.id === param.id)).toMatchObject({
      parameter: { value: 0, animated: false },
    });
    await h.line({ history: "undo" });
    await h.publish();
    expect(c.snapshot().rows.find((r) => r.id === param.id)).toMatchObject({
      parameter: { value: 24, animated: true },
    });
  } finally {
    await h.close();
  }
});
test("B370 option-arrow and semantic drop preserve stack order and undo once", async () => {
  const h = new ModelHost();
  try {
    await h.start();
    const c = h.controller;
    await c.drive(click("f1"));
    await c.drive(click("f2", false, true));
    await c.drive({ type: "reorder", id: "f2", direction: -1 });
    await c.drive({ type: "range", range: [0, 32] });
    expect(c.snapshot().rows[0].id).toBe("f2");
    await h.line({ history: "undo" });
    await h.publish();
    await c.drive({ type: "range", range: [0, 32] });
    expect(c.snapshot().rows[0].id).toBe("f1");
    await c.drive({ type: "drop", id: "f1", slot: 3 });
    await c.drive({ type: "range", range: [0, 32] });
    expect(
      c
        .snapshot()
        .rows.filter((r) => r.kind === "effect")
        .map((r) => r.id),
    ).toEqual(["f2", "f3", "f1"]);
    await h.line({ history: "undo" });
    await h.publish();
    await c.drive({ type: "range", range: [0, 32] });
    expect(c.snapshot().rows[0].id).toBe("f1");
    await c.drive({ type: "add", definition: "glow", slot: 1 });
    await c.drive({ type: "range", range: [0, 32] });
    expect(
      c
        .snapshot()
        .rows.filter((r) => r.kind === "effect")
        .map((r) => r.id)[1],
    ).toBe(c.snapshot().focus!);
  } finally {
    await h.close();
  }
});
test("failed command retains accepted selection and emits no success announcement; missing data is retained", async () => {
  const h = new ModelHost();
  try {
    await h.start();
    const c = h.controller;
    await c.drive(click("f1"));
    const before = structuredClone(c.snapshot());
    await h.line({ reject: true });
    await expect(c.drive({ type: "remove", id: "f1" })).rejects.toThrow();
    expect(c.snapshot()).toEqual(before);
    expect(h.kit.announcements).toEqual([]);
    expect(c.snapshot().rows.find((r) => r.id === "f3")).toMatchObject({
      missing: true,
    });
    const raw = c
      .snapshot()
      .rows.find((r) => r.kind === "parameter" && r.parameter.effect === "f3");
    expect(raw).toMatchObject({
      parameter: { readonly: true, value: { v: 1, opaque: "preserve" } },
    });
  } finally {
    await h.close();
  }
});
test("§5 every list shortcut routes before layer keys; nested fields retain ownership", async () => {
  const h = new ModelHost();
  try {
    await h.start();
    const c = h.controller;
    await c.drive(click("f1"));
    const root = createRoot(async (commit) => ({
      accepted: true,
      revision: commit.baseRevision + 1,
    }));
    await root.render(<EffectControls controller={c} rowHeight={24} />);
    const list = () => find(root, (p) => p.id === "layer-effects:list");
    for (const [key, detail, action] of [
      ["ArrowDown", {}, "navigate"],
      ["ArrowUp", { shiftKey: true }, "navigate"],
      ["ArrowDown", { altKey: true }, "reorder"],
      ["ArrowLeft", {}, "disclose"],
      ["ArrowRight", {}, "disclose"],
      [" ", {}, "toggle"],
      ["c", { metaKey: true }, "copy"],
      ["v", { metaKey: true }, "paste"],
    ] as const) {
      await event(root, list().id, "keydown", { key, ...detail });
      await c.drive({ type: "range", range: [0, 32] });
      await root.render(<EffectControls controller={c} rowHeight={24} />);
      const commands = h.requests.filter((r) => r.operation.type === "command");
      expect(commands.at(-2)?.operation).toMatchObject({
        args: { intent: { type: action } },
      });
    }
    await c.drive(click("f1"));
    await root.render(<EffectControls controller={c} rowHeight={24} />);
    await event(root, list().id, "keydown", { key: "Enter" });
    expect(h.kit.focus.at(-1)).toContain("layer-effects:field:");
    const param = find(root, (p) => p.control === "number");
    const before = h.requests.length;
    await event(root, param.id, "keydown", { key: "Delete" }, [list().id]);
    expect(h.requests.length).toBe(before);
    await event(root, list().id, "keydown", { key: "Escape" });
    await c.drive({ type: "range", range: [0, 32] });
    expect(c.snapshot().selectedCount).toBe(0);
    expect(h.kit.focus.at(-1)).toBe("layer-effects:list");
    await root.dispose();
  } finally {
    await h.close();
  }
});
test("native menus carry exact empty clipboard guard, groups are host-owned, and add is acknowledged", async () => {
  const h = new ModelHost();
  try {
    await h.start();
    const c = h.controller;
    let menu: any[] = [];
    h.kit.api.contextMenu = async (items) => {
      menu = items;
      return "paste";
    };
    const before = h.requests.length;
    await c.menu("f1");
    expect(menu.find((i) => i.id === "paste")).toEqual({
      id: "paste",
      label: "Paste Effect",
      disabled: true,
      tooltip: "Copy an effect first",
    });
    expect(h.requests.length).toBe(before);
    h.kit.api.effectMenu = async () => "glow";
    await c.add();
    expect(c.snapshot().effectCount).toBe(4);
    expect(h.kit.announcements).toContain("Added Glow");
    await h.select("locked");
    const locked = c.snapshot();
    await expect(
      c.drive({ type: "add", definition: "glow" }),
    ).rejects.toThrow();
    expect(c.snapshot()).toEqual(locked);
    await h.select("audio");
    expect(c.snapshot().target).toBe("Unsupported");
  } finally {
    await h.close();
  }
});
