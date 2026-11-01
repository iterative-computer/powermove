import test from "node:test";
import assert from "node:assert/strict";
import {
  AppearanceLedger,
  ABSENT,
  present,
  workspaceFields,
  applyWorkspacePatches,
  ThemeController,
  importLegacyAppearance,
  captureWorkspaceMutation,
} from "../index.mjs";

const input = (id, scheme = "auto") => ({
  id,
  name: id,
  scheme,
  tokens: { "--accent": "#123456" },
});
const fields = (value) => ({ accent: present(value) });

for (const order of [
  ["a", "b"],
  ["b", "a"],
]) {
  test(`overlapping appearance owners disabled ${order.join(" then ")} restore base`, () => {
    const ledger = new AppearanceLedger();
    ledger.record("a", "workspace:w1", fields("base"), fields("a"));
    ledger.record("b", "workspace:w1", fields("a"), fields("b"));
    let current = fields("b");
    for (const owner of order) {
      const result = ledger.release(owner, { "workspace:w1": current });
      for (const patch of result.patches) current[patch.key] = patch.value;
    }
    assert.deepEqual(current, fields("base"));
    assert.equal(ledger.export().scopes.length, 0);
    assert.equal(ledger.export().archive.length, 2);
  });
}
test("later user edit and later user deletion both survive disable", () => {
  for (const current of [fields("user"), { accent: ABSENT }]) {
    const ledger = new AppearanceLedger();
    ledger.record("mod", "w", fields("base"), fields("mod"));
    assert.deepEqual(ledger.release("mod", { w: current }).patches, []);
    assert.equal(ledger.export().scopes.length, 0);
  }
});
test("owner reapply records one current frame and unwinds to initial base", () => {
  const ledger = new AppearanceLedger();
  ledger.record("mod", "w", fields("base"), fields("one"));
  ledger.record("mod", "w", fields("one"), fields("two"));
  const result = ledger.release("mod", { w: fields("two") });
  assert.deepEqual(result.patches[0].value, present("base"));
});
test("a user change between owner writes becomes the new baseline", () => {
  const ledger = new AppearanceLedger();
  ledger.record("mod", "w", fields("base"), fields("mod"));
  ledger.record("mod", "w", fields("user"), fields("again"));
  assert.deepEqual(
    ledger.release("mod", { w: fields("again") }).patches[0].value,
    present("user"),
  );
});
test("JSON restart preserves tombstones, null and empty string separately", () => {
  for (const base of [ABSENT, present(null), present("")]) {
    const ledger = new AppearanceLedger();
    ledger.record("a", "w", { accent: base }, fields("mod"));
    const restarted = new AppearanceLedger(
      JSON.parse(JSON.stringify(ledger.export())),
    );
    assert.deepEqual(
      restarted.release("a", { w: fields("mod") }).patches[0].value,
      base,
    );
  }
});
test("inactive saved workspaces unwind independently; missing workspaces remain pending", () => {
  const ledger = new AppearanceLedger();
  for (const scope of ["active", "inactive", "unavailable"])
    ledger.record("a", scope, fields(scope), fields("mod"));
  const result = ledger.release("a", {
    active: fields("mod"),
    inactive: fields("mod"),
  });
  assert.equal(result.patches.length, 2);
  assert.deepEqual(result.pending, ["unavailable"]);
  assert.equal(ledger.export().scopes.length, 1);
  const resumed = new AppearanceLedger(ledger.export()).release("a", {
    unavailable: fields("mod"),
  });
  assert.deepEqual(resumed.patches[0].value, present("unavailable"));
});
test("startup reconcile releases only absent owners", () => {
  const ledger = new AppearanceLedger();
  ledger.record("a", "w", fields("base"), fields("a"));
  ledger.record("b", "w", fields("a"), fields("b"));
  assert.equal(
    ledger.reconcile(new Set(["b"]), { w: fields("b") })[0].patches.length,
    0,
  );
  assert.equal(
    ledger.release("b", { w: fields("b") }).patches[0].value.value,
    "base",
  );
});

const workspace = () => ({
  id: "one",
  density: "normal",
  theme: { accent: "base" },
  chrome: { custom: "base" },
  layout: {
    docks: [
      {
        id: "left",
        size: 240,
        panels: [
          { id: "inspector", size: 180 },
          { id: "layers", size: 200 },
        ],
      },
    ],
  },
});
test("workspace geometry restores by stable ids after panel reorder/addition", () => {
  const before = workspace(),
    after = workspace();
  after.layout.docks[0].size = 320;
  after.layout.docks[0].panels[0].size = 220;
  const ledger = new AppearanceLedger();
  ledger.record("a", "one", workspaceFields(before), workspaceFields(after));
  after.layout.docks[0].panels.reverse();
  after.layout.docks[0].panels.push({ id: "new", size: 123 });
  const restored = applyWorkspacePatches(
    after,
    ledger.release("a", { one: workspaceFields(after) }).patches,
  );
  assert.equal(restored.layout.docks[0].size, 240);
  assert.equal(
    restored.layout.docks[0].panels.find((panel) => panel.id === "inspector")
      .size,
    180,
  );
  assert.equal(
    restored.layout.docks[0].panels.find((panel) => panel.id === "new").size,
    123,
  );
});
test("deleted dock/panel stays deleted; later edits to another geometry field survive", () => {
  const before = workspace(),
    after = workspace();
  after.layout.docks[0].panels[0].size = 220;
  const ledger = new AppearanceLedger();
  ledger.record("a", "one", workspaceFields(before), workspaceFields(after));
  after.layout.docks[0].panels.splice(0, 1);
  const restored = applyWorkspacePatches(
    after,
    ledger.release("a", { one: workspaceFields(after) }).patches,
  );
  assert.deepEqual(restored, after);
});
test("unknown nested appearance fields and partial writes on throw are reversible", () => {
  const before = workspace(),
    after = workspace();
  try {
    after.chrome.custom = { arbitrary: [1, 2, 3] };
    after.theme.accent = "changed";
    throw new Error("fixture");
  } catch {
    /* The facade records finally, including partial mutations. */
  }
  const ledger = new AppearanceLedger();
  ledger.record("a", "one", workspaceFields(before), workspaceFields(after));
  assert.deepEqual(
    applyWorkspacePatches(
      after,
      ledger.release("a", { one: workspaceFields(after) }).patches,
    ),
    before,
  );
});
test("unsafe paths and unknown ledger versions never overwrite state", () => {
  assert.throws(
    () =>
      applyWorkspacePatches(workspace(), [
        { key: '["theme","__proto__","polluted"]', value: present("yes") },
      ]),
    { code: "invalid_appearance_path" },
  );
  assert.equal({}.polluted, undefined);
  assert.throws(() => new AppearanceLedger({ version: 99 }), {
    code: "invalid_appearance_state",
  });
});
test("theme API sync reads, stable list items and LIFO registration disposals", () => {
  const events = [],
    controller = new ThemeController({
      onChange: (event) => events.push(event),
    });
  const a = controller.api("a"),
    b = controller.api("b");
  const original = input("one"),
    override = input("one", "dark");
  a.register(original);
  a.activate("one");
  assert.equal(a.active(), "one");
  assert.strictEqual(a.list()[0], original);
  assert.notStrictEqual(a.list(), a.list());
  const handle = b.register(override);
  assert.strictEqual(a.list()[0], override);
  handle.dispose();
  handle.dispose();
  assert.strictEqual(a.list()[0], original);
  assert.equal(events.at(-1).scheme, "light");
});
test("disposing a buried registration does not notify or disturb its override", () => {
  const events = [],
    controller = new ThemeController({
      onChange: (event) => events.push(event),
    });
  const a = controller.register("a", input("one")),
    b = controller.register("b", input("one", "dark"));
  controller.change(null, "activeId", "one");
  const count = events.length;
  a.dispose();
  assert.equal(events.length, count);
  assert.equal(controller.definition().scheme, "dark");
  b.dispose();
});
for (const order of [
  ["a", "b"],
  ["b", "a"],
]) {
  test(`theme selection/scheme ownership disabled ${order.join(" then ")}`, () => {
    const controller = new ThemeController(),
      a = controller.api("a"),
      b = controller.api("b");
    a.register(input("a"));
    b.register(input("b"));
    a.activate("a");
    a.setScheme("light");
    b.activate("b");
    b.setScheme("dark");
    const owners = { a, b };
    for (const owner of order) owners[owner].dispose();
    assert.equal(controller.selection.activeId, "default");
    assert.equal(controller.selection.scheme, "system");
  });
}
test("user selection and scheme changes survive mod disposal", () => {
  const controller = new ThemeController(),
    a = controller.api("a");
  a.register(input("a"));
  a.activate("a");
  a.setScheme("dark");
  controller.change(null, "activeId", "user");
  controller.change(null, "scheme", "light");
  a.dispose();
  assert.deepEqual(controller.selection, { activeId: "user", scheme: "light" });
});
test("restart restores saved selection after late registration and missing prior theme queues fallback", () => {
  const controller = new ThemeController(),
    owner = controller.api("mod");
  controller.register("user", input("user"));
  controller.change(null, "activeId", "user");
  owner.register(input("mod"));
  owner.activate("mod");
  const restarted = new ThemeController({
    state: JSON.parse(JSON.stringify(controller.export())),
  });
  restarted.release("mod");
  assert.equal(restarted.selection.activeId, "default");
  assert.equal(restarted.pendingSelection, "user");
  restarted.register("user", input("user"));
  assert.equal(restarted.selection.activeId, "user");
});
test("system changes notify auto/system only; fixed and explicit preferences keep output", () => {
  const events = [],
    controller = new ThemeController({
      onChange: (event) => events.push(event),
    });
  controller.setSystemScheme("dark");
  assert.equal(events.at(-1).scheme, "dark");
  const api = controller.api("a");
  api.register(input("fixed", "light"));
  api.activate("fixed");
  const count = events.length;
  controller.setSystemScheme("light");
  assert.equal(events.length, count);
  api.setScheme("dark");
  assert.equal(api.scheme(), "dark");
  assert.equal(controller.plan().scheme, "light");
});
test("disposed API stops changing state and archived definitions remain available", () => {
  const controller = new ThemeController(),
    api = controller.api("a");
  api.register(input("a"));
  api.activate("a");
  api.dispose();
  api.activate("changed");
  api.setScheme("dark");
  api.register(input("new"));
  assert.equal(controller.selection.activeId, "default");
  assert.equal(controller.registrations.size, 0);
  assert.equal(controller.export().archive[0].definition.id, "a");
});

test("legacy v1 ownership import preserves both stacks, absent values and unknown data", () => {
  const path = '["theme","accent"]';
  const workspaceState = {
    version: 1,
    unknown: { keep: "value" },
    workspaces: [["one", [[path, { stack: [{ owner: "a", value: "mod" }] }]]]],
  };
  const themeState = {
    version: 1,
    fields: [
      ["activeId", { base: "default", stack: [{ owner: "a", value: "mod" }] }],
    ],
  };
  const imported = importLegacyAppearance(workspaceState, themeState);
  assert.deepEqual(imported.diagnostics, []);
  const ledger = new AppearanceLedger(imported.state);
  const release = ledger.release("a", {
    "workspace:one": { [path]: present("mod") },
    theme: { activeId: present("mod") },
  });
  assert.equal(release.patches.length, 2);
  assert.deepEqual(
    release.patches.find((patch) => patch.scope === "workspace:one").value,
    ABSENT,
  );
  assert.deepEqual(ledger.export().archive[0].workspaceState, workspaceState);
});
test("unrecognized legacy format and unsafe paths are archived and reported", () => {
  const unknown = { version: 99, payload: "preserve" };
  const result = importLegacyAppearance(unknown, {
    version: 1,
    fields: [
      ["scheme", { base: "dark", stack: [{ owner: "a", value: "invalid" }] }],
    ],
  });
  assert.equal(result.diagnostics.length, 2);
  assert.deepEqual(result.state.archive[0].workspaceState, unknown);
  const unsafe = importLegacyAppearance({
    version: 1,
    workspaces: [
      [
        "one",
        [
          [
            '["theme","constructor"]',
            { base: null, stack: [{ owner: "a", value: "x" }] },
          ],
        ],
      ],
    ],
  });
  assert.equal(unsafe.diagnostics.length, 1);
});
test("actual workspace capture records normalized mutations in finally and rethrows same error", () => {
  const current = workspace(),
    ledger = new AppearanceLedger(),
    error = new Error("fixture");
  const api = {
    current: () => current,
    mutate: (fn) => {
      fn(current);
      return current;
    },
  };
  assert.throws(
    () =>
      captureWorkspaceMutation(ledger, "a", api, (draft) => {
        draft.theme.accent = "changed";
        throw error;
      }),
    (thrown) => thrown === error,
  );
  const restored = applyWorkspacePatches(
    current,
    ledger.release("a", { "workspace:one": workspaceFields(current) }).patches,
  );
  assert.equal(restored.theme.accent, "base");
  const result = captureWorkspaceMutation(ledger, "a", api, (draft) => {
    draft.density = "compact";
  });
  assert.strictEqual(result, current);
});
test("restoring absent or replaced containers never leaves empty override objects", () => {
  for (const original of [undefined, "legacy-scalar", {}]) {
    const before = { id: "one", theme: original },
      after = { id: "one", theme: { accent: "mod" } };
    const ledger = new AppearanceLedger();
    ledger.record("a", "w", workspaceFields(before), workspaceFields(after));
    const restored = applyWorkspacePatches(
      after,
      ledger.release("a", { w: workspaceFields(after) }).patches,
    );
    if (original === undefined)
      assert.equal(Object.hasOwn(restored, "theme"), false);
    else assert.deepEqual(restored.theme, original);
  }
});

test("container cleanup preserves later user leaves and removes only empty created ancestors", () => {
  for (const userEdit of [false, true]) {
    const before = { id: "one" },
      after = { id: "one", theme: { nested: { accent: "mod" } } };
    const ledger = new AppearanceLedger();
    ledger.record("a", "w", workspaceFields(before), workspaceFields(after));
    if (userEdit) after.theme.nested.font = "user";
    const restored = applyWorkspacePatches(
      after,
      ledger.release("a", { w: workspaceFields(after) }).patches,
    );
    assert.deepEqual(
      restored,
      userEdit ? { id: "one", theme: { nested: { font: "user" } } } : before,
    );
  }
});

test("reconcile stages restorations between stale owners across workspace orderings", () => {
  const ledger = new AppearanceLedger();
  ledger.record("b", "first", fields("base"), fields("b"));
  ledger.record("a", "second", fields("base"), fields("a"));
  ledger.record("b", "second", fields("a"), fields("b"));
  const current = { first: fields("b"), second: fields("b") };
  for (const result of ledger.reconcile(new Set(), current))
    for (const patch of result.patches)
      current[patch.scope][patch.key] = patch.value;
  assert.deepEqual(current, { first: fields("base"), second: fields("base") });
});

test("restoration patches recheck current values when applied after a later user edit", () => {
  const before = workspace(),
    after = workspace();
  after.theme.accent = "mod";
  const ledger = new AppearanceLedger();
  ledger.record("a", "w", workspaceFields(before), workspaceFields(after));
  const released = ledger.release("a", { w: workspaceFields(after) });
  after.theme.accent = "later-user-edit";
  assert.equal(
    applyWorkspacePatches(after, released.patches).theme.accent,
    "later-user-edit",
  );
});

test("new-format unknown state fields round trip and unknown controller versions refuse overwrite", () => {
  const state = { version: 1, scopes: [], archive: [], future: { keep: true } };
  const ledger = new AppearanceLedger(state);
  assert.deepEqual(ledger.export().future, state.future);
  const controller = new ThemeController({
    state: { version: 1, future: { keep: true } },
  });
  assert.deepEqual(controller.export().future, { keep: true });
  assert.throws(
    () => new ThemeController({ state: { version: 99, opaque: "retain" } }),
    { code: "invalid_appearance_state" },
  );
});

test("Store ids are qualified, system preference stays private and scheme changes are denied", () => {
  const controller = new ThemeController({ systemScheme: "dark" });
  const api = controller.api("store-owner", "store");
  const definition = input("fixture");
  api.register(definition);
  api.activate("fixture");
  assert.equal(api.active(), "store-owner.fixture");
  assert.equal(api.scheme(), "dark");
  assert.equal(controller.selection.scheme, "system");
  assert.throws(() => api.setScheme("light"), { code: "permission_denied" });
  definition.css = "#tabs{color:red}";
  const listed = api.list();
  listed[0].css = "#tabs{color:red}";
  assert.equal(
    controller.plan().rules.some((rule) => rule.selector.raw === "#tabs"),
    false,
  );
  assert.equal(
    controller.plan().rootAttributes["data-store-mutated"],
    undefined,
  );
});

test("Store theme registration cannot collide with another owner's qualified id", () => {
  const controller = new ThemeController();
  controller.register("other", input("store-owner.fixture"));
  assert.throws(
    () => controller.api("store-owner", "store").register(input("fixture")),
    { code: "permission_denied" },
  );
});
