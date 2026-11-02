import { beforeAll, expect, test } from "bun:test";
import { ProjectPanel } from "../index";
import { card, event, find, layout, model, native, recorder } from "./helpers";
import { project } from "./fixtures";
import type { Input, Modifiers } from "../contract";
beforeAll(async () => {
  const { binaries } = await import("./build-model");
  binaries();
}, 120_000);
const none: Modifiers = {
  meta: false,
  control: false,
  shift: false,
  alt: false,
};
const down = (id: string, modifiers = none): Input => ({
  Pointer: {
    id: { Asset: id },
    phase: "Down",
    button: 0,
    nested_control: false,
    modifiers,
  },
});
const key = (key: string, id: string) =>
  ({ Key: { target: { Row: { Asset: id } }, key, modifiers: none } }) as Input;
const runtime = {
  a: "Ready",
  b: "Ready",
  d: "Ready",
  nested: "Ready",
} as const;

test("accepted op-tree events select through the real Rust model", async () => {
  const m = await model(project, runtime);
  const { root } = recorder();
  try {
    await root.render(
      <ProjectPanel service={m.service} native={native} layout={layout} />,
    );
    await event(root, card(root, "a").id, "pointerdown", { button: 0 });
    await m.service.idle();
    expect(m.service.view!.selected_count).toBe(1);
    await event(root, card(root, "b").id, "pointerdown", {
      button: 0,
      metaKey: true,
    });
    await m.service.idle();
    expect(m.service.view!.selected_count).toBe(2);
    expect(
      m.service.view!.page.rows.filter((r) => r.selected).map((r) => r.id),
    ).toEqual([{ Asset: "a" }, { Asset: "b" }]);
    await event(root, find(root, "Project media").id, "keydown", {
      key: "Escape",
    });
    await m.service.idle();
    expect(m.service.view!.selected_count).toBe(0);
  } finally {
    await root.dispose();
    await m.close();
  }
});
test("shift selection, wrap, Return and Space are model inputs, never local edits", async () => {
  const m = await model(project, runtime);
  try {
    await m.service.input(down("a"));
    await m.service.input(down("d", { ...none, shift: true }));
    expect(m.service.view!.selected_count).toBe(3);
    let r = await m.service.input(key("Right", "d"));
    expect(r!.view.focused).toEqual({ Asset: "a" });
    r = await m.service.input(key("Enter", "a"));
    expect((r!.outcome as any).effects).toContainEqual({ AddAsset: "a" });
    r = await m.service.input(key("Space", "a"));
    expect((r!.outcome as any).effects).toContainEqual({ TogglePreview: "a" });
  } finally {
    await m.close();
  }
});
test("folder creation and rename wait for kernel; Undo/Redo preserve media", async () => {
  const m = await model(project, runtime);
  try {
    const r = await m.service.input({ Invoke: { NewFolder: null } });
    const id = r!.view.renaming!;
    expect(id).toBeTruthy();
    const named = await m.service.input({
      CommitRename: { folder: id, value: " Day 2 " },
    });
    expect(
      named!.view.page.rows.some(
        (r) => "Folder" in r.data && r.data.Folder.name === "Day 2",
      ),
    ).toBe(true);
    const undo = await m.send({ history: "Undo" });
    expect(undo.error).toBeUndefined();
    expect(
      undo.view.page.rows.some((r: any) => r.data.Folder?.name === "Day 2"),
    ).toBe(false);
    const redo = await m.send({ history: "Redo" });
    expect(
      redo.view.page.rows.some((r: any) => r.data.Folder?.name === "Day 2"),
    ).toBe(true);
    expect(redo.view.page.total).toBe(6);
  } finally {
    await m.close();
  }
});
test("referenced delete confirmation is cancel-safe and real-kernel undoable", async () => {
  const m = await model(project, runtime);
  try {
    await m.service.input(down("a"));
    let r = await m.service.input({ Invoke: "DeleteAssets" });
    const token = (r!.outcome as any).effects.find((e: any) => e.ConfirmDelete)
      .ConfirmDelete.token;
    await m.service.input({ ConfirmDelete: { token, approved: false } });
    expect(
      m.service.view!.page.rows.some(
        (r) => "Asset" in r.id && r.id.Asset === "a",
      ),
    ).toBe(true);
    r = await m.service.input({ Invoke: "DeleteAssets" });
    const token2 = (r!.outcome as any).effects.find((e: any) => e.ConfirmDelete)
      .ConfirmDelete.token;
    await m.service.input({ ConfirmDelete: { token: token2, approved: true } });
    expect(
      m.service.view!.page.rows.some(
        (r) => "Asset" in r.id && r.id.Asset === "a",
      ),
    ).toBe(false);
    const undo = await m.send({ history: "Undo" });
    expect(undo.view.page.rows.some((r: any) => r.id.Asset === "a")).toBe(true);
  } finally {
    await m.close();
  }
});
test("offline and cloud activate locate/download; Loading never becomes offline", async () => {
  const m = await model(project, {
    a: "Offline",
    b: { Cloud: { provider: "Fixture", download: "Stored" } },
    d: "Loading",
  });
  try {
    const double = (id: string): Input => ({
      Pointer: {
        id: { Asset: id },
        phase: "DoubleClick",
        button: 0,
        nested_control: false,
        modifiers: none,
      },
    });
    expect((await m.service.input(double("a")))!.outcome).toMatchObject({
      effects: expect.arrayContaining([{ PickFiles: { replace: "a" } }]),
    });
    expect((await m.service.input(double("b")))!.outcome).toMatchObject({
      effects: expect.arrayContaining([{ Download: "b" }]),
    });
    expect((await m.service.input(double("d")))!.outcome).toMatchObject({
      effects: [],
    });
  } finally {
    await m.close();
  }
});
test("system menu receives model eligibility including last composition guard", async () => {
  const m = await model(project, runtime);
  try {
    const comp = await m.service.menu({ Composition: "C" });
    expect(comp!.menu!.entries).toContainEqual({
      command: { DeleteComposition: "C" },
      enabled: false,
    });
    const asset = await m.service.menu({ Asset: "a" });
    expect(
      asset!.menu!.entries.some(
        (e) => typeof e.command === "object" && "NewCompFromAsset" in e.command,
      ),
    ).toBe(true);
    const audio = await m.service.menu({ Asset: "b" });
    expect(
      audio!.menu!.entries.some(
        (e) => typeof e.command === "object" && "NewCompFromAsset" in e.command,
      ),
    ).toBe(false);
  } finally {
    await m.close();
  }
});
test("external Option drop imports only; internal drop moves once with Undo", async () => {
  const m = await model(project, runtime);
  try {
    let r = await m.service.input({
      Drop: {
        target: "f",
        files: ["/synthetic/frames"],
        modifiers: { ...none, alt: true },
      },
    });
    expect((r!.outcome as any).effects).toContainEqual({
      Import: { files: ["/synthetic/frames"], folder: "f", as_folder: true },
    });
    await m.service.input(down("b"));
    await m.service.input({
      DragStart: { id: { Asset: "b" }, nested_control: false },
    });
    await m.service.input({
      Drop: { target: "f", files: [], modifiers: none },
    });
    expect(
      m.service.view!.page.rows.some(
        (r) => "Asset" in r.id && r.id.Asset === "b",
      ),
    ).toBe(false);
    const undo = await m.send({ history: "Undo" });
    expect(undo.view.page.rows.some((r: any) => r.id.Asset === "b")).toBe(true);
  } finally {
    await m.close();
  }
});
test("all-row marquee and autoscroll use model pointer ownership", async () => {
  const m = await model(project, runtime);
  try {
    await m.service.input({ BeginMarquee: { pointer: 9, modifiers: none } });
    await m.service.input({
      MarqueeHits: { pointer: 8, activated: true, ids: ["a", "b"] },
    });
    expect(m.service.view!.selected_count).toBe(0);
    await m.service.input({
      MarqueeHits: { pointer: 9, activated: true, ids: ["a", "b"] },
    });
    expect(m.service.view!.selected_count).toBe(2);
    const r = await m.service.input({
      MarqueeEdge: { pointer: 9, activated: true, direction: "After" },
    });
    expect((r!.outcome as any).effects).toContainEqual({
      ScrollMarquee: "After",
    });
    await m.service.input({ EndMarquee: { pointer: 9 } });
  } finally {
    await m.close();
  }
});
test("60k model rows navigate outside mounted window and keep bounded view", async () => {
  const assets = Object.fromEntries(
    Array.from({ length: 60_000 }, (_, i) => {
      const id = `a${String(i).padStart(5, "0")}`;
      return [id, { id, name: `${id}.mov`, kind: "video" }];
    }),
  );
  const m = await model({ ...project, assets, assetFolders: {}, layers: [] });
  try {
    expect(m.service.view!.page.total).toBe(60_001);
    expect(m.service.view!.page.rows.length).toBe(10);
    await m.service.input({
      Key: { target: "List", key: "A", modifiers: { ...none, meta: true } },
    });
    expect(m.service.view!.selected_count).toBe(60_000);
    await m.service.input(key("End", "a00000"));
    expect(m.service.view!.focused).toEqual({ Asset: "a59999" });
    await m.service.page(59_990, 11);
    expect(m.service.view!.page.rows.length).toBe(11);
    expect(m.service.view!.page.rows.at(-1)!.focused).toBe(true);
  } finally {
    await m.close();
  }
}, 30_000);
