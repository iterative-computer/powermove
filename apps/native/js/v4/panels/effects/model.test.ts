import { expect, test } from "bun:test";
import { fixture, itemRow, items, deferred } from "./fixtures";
import { labelParts, readIds, workerSearch } from "./model";

test("All groups preserve registry order, Recent precedes them and duplicate presentations share one selection", () => {
  const { model } = fixture();
  model.restoreSettings(["glow"], []);
  expect(
    model
      .rows()
      .filter((row) => row.type === "group")
      .map((row) => row.name),
  ).toEqual(["Recent", "Blur & Sharpen", "Stylize"]);
  const glow = model
    .rows()
    .filter((row) => row.type === "item" && row.item.id === "glow");
  expect(new Set(glow.map((row) => row.key)).size).toBe(2);
  model.select(glow[1].key);
  expect(model.selected).toBe("glow");
  expect(model.inspect().selectionMode).toBe("single");
});
test("click selects one ID without applying or pushing recents", () => {
  const { model, applied } = fixture();
  model.select(itemRow(model).key);
  model.select(itemRow(model, "glow").key);
  expect(model.selected).toBe("glow");
  expect(applied).toEqual([]);
  expect(model.recent).toEqual([]);
});
test("scope selection clears search; disappearing groups and Recent fall back to All", async () => {
  const { model } = fixture();
  await model.setQuery("blur");
  model.pickScope("group:Stylize");
  expect(model.query).toBe("");
  expect(model.rows().map((row) => row.type === "item" && row.item.id)).toEqual(
    ["glow"],
  );
  model.replaceRegistry({ items: [items[0]], groups: [items[0].group] });
  expect(model.scope).toBe("all");
  model.restoreSettings(["blur", "missing"], []);
  model.pickScope("recent");
  expect(model.rows()).toHaveLength(1);
  model.replaceRegistry({ items: [], groups: [] });
  expect(model.scope).toBe("all");
});
test("registered names All/Recent never alias built-in scopes or disclosure keys", () => {
  const { model } = fixture();
  model.replaceRegistry({
    items: [{ ...items[0], group: "Recent" }],
    groups: ["All", "Recent"],
  });
  model.restoreSettings(["blur"], []);
  expect(new Set(model.scopes().map((scope) => scope.value)).size).toBe(4);
  expect(new Set(model.rows().map((row) => row.key)).size).toBe(
    model.rows().length,
  );
  model.pickScope("group:All");
  expect(model.rows()).toEqual([]);
});
test("search trims and folds case and searches label or ID across every scope and collapsed group", async () => {
  const { model } = fixture();
  model.toggle(model.rows()[0].key, false);
  model.pickScope("group:Stylize");
  await model.setQuery("  BLUR  ");
  expect(model.rows().map((row) => row.type === "item" && row.item.id)).toEqual(
    ["blur"],
  );
  expect(model.announcement.text).toBe("1 effects");
  expect(await workerSearch({ items, query: "glow" })).toEqual(["glow"]);
  expect(labelParts("Gaussian Blur", "BLUR")).toEqual({
    before: "Gaussian ",
    match: "Blur",
    after: "",
  });
  expect(labelParts("Glow", "not-in-label")).toEqual({
    before: "Glow",
    match: "",
    after: "",
  });
});
test("slow search retains previous rows and ignores a late answer", async () => {
  const first = deferred<readonly string[]>(),
    second = deferred<readonly string[]>();
  let n = 0;
  const { model } = fixture({
    search: () => (++n === 1 ? first.promise : second.promise),
  });
  const rows = model.rows();
  const one = model.setQuery("blur");
  expect(model.searching).toBe(true);
  expect(model.rows()).toBe(rows);
  const two = model.setQuery("glow");
  second.resolve(["glow"]);
  await two;
  first.resolve(["blur"]);
  await one;
  expect(itemRow(model, "glow").item.id).toBe("glow");
  expect(model.rows()).toHaveLength(1);
});
test("clearing search and picking a scope fence pending worker answers", async () => {
  const pending = deferred<readonly string[]>();
  const { model } = fixture({ search: () => pending.promise });
  const one = model.setQuery("blur");
  model.pickScope("group:Stylize");
  pending.resolve(["blur"]);
  await one;
  expect(model.query).toBe("");
  expect(itemRow(model, "glow").item.id).toBe("glow");
  expect(model.searching).toBe(false);
});
test("registry replacement fences previous search and never publishes an unloaded definition", async () => {
  const searches: ReturnType<typeof deferred<readonly string[]>>[] = [];
  const { model } = fixture({
    search: () => {
      const p = deferred<readonly string[]>();
      searches.push(p);
      return p.promise;
    },
  });
  const one = model.setQuery("blur");
  model.replaceRegistry({ items: [items[1]], groups: [items[1].group] });
  searches[1].resolve([]);
  await Promise.resolve();
  searches[0].resolve(["blur"]);
  await one;
  expect(model.rows()).toEqual([]);
});
test("search failure leaves previous rows inspectable, reports failure and stops progress", async () => {
  const { model, errors } = fixture({
    search: async () => {
      throw new Error("worker unavailable");
    },
  });
  const rows = model.rows();
  await model.setQuery("blur");
  expect(model.rows()).toBe(rows);
  expect(model.searching).toBe(false);
  expect(errors).toHaveLength(1);
});
test("applying without target announces exact copy; rows remain draggable", async () => {
  const { model, applied } = fixture();
  expect(await model.apply("blur")).toBe(false);
  expect(model.announcement.text).toBe("Select a layer first");
  expect(applied).toEqual([]);
  expect(model.drag("blur")).toEqual({
    kind: "effect",
    id: "blur",
    label: "Gaussian Blur",
  });
});
test("apply waits for actor ack before recents or success announcement", async () => {
  const pending = deferred<void>();
  const { model, saved } = fixture({ apply: () => pending.promise });
  model.setTarget({ id: "layer-a", effects: [] });
  const applying = model.apply("blur");
  expect(model.recent).toEqual([]);
  expect(model.announcement.text).toBe("");
  pending.resolve();
  expect(await applying).toBe(true);
  await model.settingsSettled();
  expect(model.recent).toEqual(["blur"]);
  expect(saved).toEqual([["pm.fx.recent", ["blur"]]]);
  expect(model.announcement.text).toBe("Added Gaussian Blur");
});
test("rejected edits never claim success or change recents", async () => {
  const { model, errors, saved } = fixture({
    apply: async () => {
      throw new Error("stale revision");
    },
  });
  model.setTarget({ id: "layer", effects: [] });
  expect(await model.apply("blur")).toBe(false);
  expect(model.recent).toEqual([]);
  expect(saved).toEqual([]);
  expect(errors).toHaveLength(1);
  expect(model.announcement.sequence).toBe(0);
});
test("apply captures target at gesture time without mutating selection or document", async () => {
  const { model, applied } = fixture();
  model.setTarget({ id: "layer-a", effects: [] });
  const pending = model.apply("blur");
  model.setTarget({ id: "layer-b", effects: [] });
  await pending;
  expect(applied).toEqual([[items[0], { id: "layer-a", effects: [] }]]);
  expect(model.selected).toBeNull();
});
test("recents deduplicate and cap at eight while retaining unknown installed-later IDs", async () => {
  const { model } = fixture();
  model.restoreSettings(
    ["a", "b", "c", "d", "e", "f", "g", "h", "ignored"],
    [],
  );
  model.setTarget({ id: "layer", effects: [] });
  await model.apply("blur");
  await model.apply("glow");
  await model.apply("blur");
  expect(model.recent).toEqual(["blur", "glow", "a", "b", "c", "d", "e", "f"]);
  model.replaceRegistry({
    items: [...items, { ...items[0], id: "a" }],
    groups: [],
  });
  expect(model.recentItems().map((item) => item.id)).toEqual([
    "blur",
    "glow",
    "a",
  ]);
});
test("failed definitions stay browsable and cannot be applied or dragged", async () => {
  const { model, applied } = fixture();
  model.replaceRegistry({ items: [{ ...items[0], failed: true }], groups: [] });
  model.select(itemRow(model).key);
  model.setTarget({ id: "layer", effects: [] });
  expect(await model.apply("blur")).toBe(false);
  expect(model.drag("blur")).toBeNull();
  expect(model.context("blur")[0].disabled).toBe(true);
  expect(applied).toEqual([]);
});
test("context menu availability and both actions are ID driven", async () => {
  const { model, revealed } = fixture();
  expect(model.context("blur")).toEqual([
    { id: "add", label: "Add to Selected Layer", disabled: true },
  ]);
  model.setTarget({ id: "layer", effects: ["blur"] });
  expect(model.context("blur")).toHaveLength(2);
  expect(await model.contextAction("blur", "reveal")).toBe(true);
  expect(revealed).toEqual([[items[0], { id: "layer", effects: ["blur"] }]]);
  expect(await model.contextAction("glow", "reveal")).toBe(false);
  expect(await model.contextAction("blur", "add")).toBe(true);
});
test("collapse persists app UI state and moves a hidden focused child to its parent", async () => {
  const { model, saved } = fixture();
  const row = itemRow(model);
  model.focused = row.key;
  model.toggle(row.parent!);
  expect(model.focused).toBe(row.parent!);
  expect(model.rows().some((other) => other.key === row.key)).toBe(false);
  await model.settingsSettled();
  expect(saved).toEqual([["pm.fx.collapsed", [row.parent]]]);
});
test("late setting hydration cannot overwrite a newer collapse or acknowledged apply", async () => {
  const { model } = fixture();
  const stamp = model.settingsStamp();
  const group = model.rows()[0].key;
  model.toggle(group, false);
  model.setTarget({ id: "layer", effects: [] });
  await model.apply("blur");
  model.restoreSettings(["old"], [], stamp);
  expect(model.recent).toEqual(["blur"]);
  expect(model.collapsed.has(group)).toBe(true);
});
test("settings failure reports separately after successful document edit", async () => {
  const { model, errors } = fixture({
    persist: async () => {
      throw new Error("disk unavailable");
    },
  });
  model.setTarget({ id: "layer", effects: [] });
  expect(await model.apply("blur")).toBe(true);
  await model.settingsSettled();
  expect(model.announcement.text).toBe("Added Gaussian Blur");
  expect(errors).toHaveLength(1);
});
test("migration settings decode JSON or arrays and never rewrite malformed source data", () => {
  expect(readIds('["blur",null,"blur","later"]')).toEqual(["blur", "later"]);
  expect(readIds("{")).toEqual([]);
  expect(readIds({ foreign: true })).toEqual([]);
});
test("search Escape clears then focuses list, Down goes first, list Escape returns to search", async () => {
  const { model } = fixture();
  model.setFocus("search");
  await model.setQuery("blur");
  await model.key("Escape");
  expect(model.query).toBe("");
  expect(model.focus).toBe("search");
  await model.key("Escape");
  expect(model.focus).toBe("list");
  expect(model.focused).toBe(model.rows()[0].key);
  await model.key("Escape");
  expect(model.focus).toBe("search");
  await model.key("ArrowDown");
  expect(model.focused).toBe(model.rows()[0].key);
});
test("list Up/Down/Home/End navigate rows without changing browsing selection", async () => {
  const { model, applied } = fixture();
  model.setFocus("list");
  await model.key("Home");
  await model.key("ArrowDown");
  expect(model.focused).toBe(model.rows()[1].key);
  await model.key("End");
  expect(model.focused).toBe(model.rows().at(-1)!.key);
  await model.key("ArrowUp");
  expect(model.focused).toBe(model.rows().at(-2)!.key);
  await model.key("Home");
  await model.key("ArrowUp");
  expect(model.focused).toBe(model.rows()[0].key);
  expect(model.selected).toBeNull();
  expect(applied).toEqual([]);
});
test("Left collapses group or focuses parent, Right expands; Return applies focused item once", async () => {
  const { model, applied } = fixture();
  model.setFocus("list");
  await model.key("Home");
  await model.key("ArrowLeft");
  expect(model.rows()).toHaveLength(3);
  await model.key("ArrowRight");
  expect(model.rows()).toHaveLength(4);
  await model.key("ArrowDown");
  await model.key("ArrowLeft");
  expect(model.focused).toBe(model.rows()[0].key);
  await model.key("ArrowDown");
  model.setTarget({ id: "layer", effects: [] });
  await model.key("Enter");
  await model.key("Enter", { repeat: true });
  expect(applied).toHaveLength(1);
});
test("printable list input transfers into search; modifiers and composition belong to native text/global resolver", async () => {
  const { model } = fixture();
  model.setFocus("list");
  expect(await model.key("g", { metaKey: true })).toBe(false);
  expect(await model.key("g", { composing: true })).toBe(false);
  expect(await model.key("g")).toBe(true);
  expect(model.query).toBe("g");
  expect(model.focus).toBe("search");
  model.setFocus("scope");
  expect(await model.key("Escape")).toBe(false);
});
test("60k row windows bound materialized content, reveal distant focus and preserve anchor by ID", async () => {
  const { model } = fixture();
  const registry = Array.from({ length: 60000 }, (_, n) => ({
    ...items[0],
    id: `fx-${n}`,
    label: `Effect ${n}`,
  }));
  model.replaceRegistry({ items: registry, groups: [] });
  model.setViewport(30000, 20);
  expect(model.window().rows).toHaveLength(60);
  const anchor = model.rows()[model.viewport.start].key;
  model.replaceRegistry({
    items: [{ ...items[0], id: "new" }, ...registry],
    groups: [],
  });
  expect(model.rows()[model.viewport.start].key).toBe(anchor);
  model.setFocus("list");
  await model.key("End");
  expect(model.window().rows.some((row) => row.key === model.focused)).toBe(
    true,
  );
  await model.setQuery("fx-59999");
  expect(model.rows()).toHaveLength(1);
});
test("viewport admission rejects negative, fractional, nonfinite and overflowing counts", () => {
  const { model } = fixture();
  for (const value of [-1, 0.5, Infinity, NaN, Number.MAX_SAFE_INTEGER + 1])
    expect(() => model.setViewport(value, 20)).toThrow();
  model.setViewport(Number.MAX_SAFE_INTEGER, 0);
  expect(model.window().rows).toEqual([]);
});
test("registry admission rejects duplicate IDs atomically, copies data and preserves previous state", () => {
  const { model } = fixture();
  const before = model.rows();
  expect(() =>
    model.replaceRegistry({ items: [items[0], items[0]], groups: [] }),
  ).toThrow("duplicate");
  expect(model.rows()).toBe(before);
  const authored = { ...items[0] };
  model.replaceRegistry({ items: [authored], groups: [] });
  authored.label = "mutated";
  expect(itemRow(model).item.label).toBe("Gaussian Blur");
});
test("uninstalled items do not appear and presets use the same model actions without fabricating a library", async () => {
  const { model, applied } = fixture();
  model.replaceRegistry({
    items: [
      { ...items[0], installed: false },
      { ...items[1], kind: "preset" },
    ],
    groups: [],
  });
  expect(model.installedCount()).toBe(1);
  model.setTarget({ id: "layer", effects: [] });
  await model.apply("glow");
  expect(applied).toHaveLength(1);
  expect(model.drag("glow")?.kind).toBe("effect");
});
test("retirement fences pending search, apply, registry and setting observations", async () => {
  const apply = deferred<void>(),
    search = deferred<readonly string[]>();
  const { model, saved } = fixture({
    apply: () => apply.promise,
    search: () => search.promise,
  });
  model.setTarget({ id: "layer", effects: [] });
  const applying = model.apply("blur"),
    searching = model.setQuery("blur");
  model.dispose();
  apply.resolve();
  search.resolve(["blur"]);
  expect(await applying).toBe(false);
  await searching;
  model.replaceRegistry({ items: [], groups: [] });
  model.restoreSettings(["foreign"], []);
  expect(model.installedCount()).toBe(2);
  expect(model.recent).toEqual([]);
  expect(saved).toEqual([]);
});
