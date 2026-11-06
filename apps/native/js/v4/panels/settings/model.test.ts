import { test, expect } from "bun:test";
import { HostClient } from "../../src/host-api";
import { SettingsModel, type Publication } from "./model";
import { fixture, publication, extension, profileFor } from "./fixtures";

for (const project of [undefined, { id: "p", name: "Fixture" }])
  test(`default section, singleton window intent, continuous section order: ${!!project}`, () => {
    const { model, effects } = fixture({ project });
    model.show();
    model.show();
    expect(model.active).toBe(project ? "project" : "general");
    expect(model.availableSections).toEqual([
      ...(project ? ["project" as const, "memory" as const] : []),
      "general",
      "accounts",
      "transcription",
      "extensions",
    ]);
    expect(effects.filter((e) => e[0] === "present")).toHaveLength(2);
    model.dispose();
  });
test("no-project requests fall back; missing project publications remove project rows", () => {
  const { model } = fixture();
  model.show("project");
  expect(model.active).toBe("general");
  model.publish(
    publication({ revision: 2, project: { id: "p", name: "Fixture" } }),
  );
  model.select("project");
  model.publish(publication({ revision: 3 }));
  expect(model.active).toBe("general");
  expect(model.rows.some((r) => r.section === "project")).toBe(false);
});
test("profile hydration preserves legacy keys, defaults and unknown authored values", async () => {
  const { model, settings, requests } = fixture();
  settings.set("previewMemoryMiB", 512);
  settings.set("themeMode", "extension-authored");
  await model.hydrate();
  expect(model.rows.find((r) => r.id === "preview-memory")?.value).toBe(512);
  expect(settings.get("themeMode")).toBe("extension-authored");
  expect(model.rows.find((r) => r.id === "appearance")?.value).toBe(
    "extension-authored",
  );
  expect(requests.map((r) => r.operation.type)).toEqual(
    Array(4).fill("settings.get"),
  );
  expect(model.rows.find((r) => r.id === "cloud-media")?.value).toBe(false);
  expect(model.rows.find((r) => r.id === "restore-windows")?.value).toBe(true);
});
test("preferences persist immediately through serialized v4 host acknowledgements", async () => {
  const { model, settings, requests } = fixture();
  expect(await model.write("appearance", "dark")).toBe(true);
  expect(await model.write("preview-memory", 0)).toBe(true);
  expect(await model.write("cloud-media", true)).toBe(true);
  expect(await model.write("restore-windows", false)).toBe(true);
  expect([...settings]).toEqual([
    ["themeMode", "dark"],
    ["previewMemoryMiB", 0],
    ["autoDownloadCloudMedia", true],
    ["restoreWindows", false],
  ]);
  expect(requests.map((r) => r.expectedRevision)).toEqual([0, 1, 2, 3]);
});
test("failed acknowledgement keeps the last accepted value and allows retry", async () => {
  const f = fixture();
  let reject = true;
  const host = new HostClient(
    { realm: "failure", generation: 1, project: "p" },
    async (request) => ({
      ...request,
      revision: 0,
      accepted: !reject,
      stateRevision: 0,
      error: reject ? "declined" : undefined,
    }),
  );
  const model = new SettingsModel(
    host,
    publication(),
    f.window,
    undefined,
    profileFor(host),
  );
  expect(await model.write("appearance", "dark")).toBe(false);
  expect(model.rows.find((r) => r.id === "appearance")?.value).toBe("system");
  expect(model.inspect().failedIds).toEqual(["appearance"]);
  reject = false;
  expect(await model.write("appearance", "dark")).toBe(true);
  expect(model.inspect().failedIds).toEqual([]);
});
test("invalid, disabled, non-finite and unsupported inputs never reach host", async () => {
  const { model, requests } = fixture();
  for (const [id, value] of [
    ["missing", 0],
    ["density", "Compact"],
    ["preview-memory", -1],
    ["preview-memory", NaN],
    ["appearance", "unknown"],
    ["cloud-media", 1],
  ] as const)
    expect(await model.write(id, value)).toBe(false);
  expect(requests).toHaveLength(0);
});
test("multi-window gate removes the restoration row", () =>
  expect(
    fixture({ multiWindow: false }).model.rows.some(
      (r) => r.id === "restore-windows",
    ),
  ).toBe(false));
test("search spans details, option labels, transcription and extension details", async () => {
  const { model, effects } = fixture({
    extensions: [{ ...extension(), about: "Quantum fixture" }],
    speech: [
      {
        id: "speech",
        name: "Fixture speech",
        size: "Fixture size",
        state: "available",
      },
    ],
  });
  await model.search("instant");
  expect([...model.matches.rows]).toEqual(["preview-memory"]);
  await model.search("1 GB");
  expect([...model.matches.rows]).toEqual(["preview-memory"]);
  await model.search("Quantum");
  expect([...model.matches.extensions]).toEqual(["fixture-0"]);
  expect(model.visibleSections).toEqual(["extensions"]);
  await model.search("speech");
  expect(model.visibleSections).toEqual(["transcription"]);
  await model.search("no-fixture-match");
  expect(model.matches.count).toBe(0);
  expect(effects.at(-1)).toEqual(["announce", "No settings found"]);
});
test("worker search retains previous results and drops stale completion", async () => {
  const { model } = fixture();
  const before = model.matches;
  const older = model.search("instant");
  expect(model.matches).toBe(before);
  expect(model.searching).toBe(true);
  const newer = model.search("Appearance");
  await Promise.all([older, newer]);
  expect([...model.matches.rows]).toEqual(["appearance"]);
  expect(model.searching).toBe(false);
});
test("sidebar arrows/Home/End navigate matching sections only; scroll tracking changes title", async () => {
  const { model, effects } = fixture();
  model.show();
  model.navigate("End");
  expect(model.active).toBe("extensions");
  model.navigate("Home");
  expect(model.active).toBe("general");
  model.navigate("ArrowDown");
  expect(model.active).toBe("accounts");
  model.navigate("ArrowUp");
  expect(model.active).toBe("general");
  await model.search("instant");
  model.navigate("End");
  expect(model.active).toBe("general");
  await model.search("");
  model.trackSection("accounts");
  expect(effects.at(-1)).toEqual(["title", "accounts"]);
});
test("reduced motion navigation jumps through the host contract", () => {
  const { model, effects } = fixture();
  model.reduceMotion = true;
  model.select("accounts");
  expect(effects.at(-1)).toEqual([
    "scroll",
    { section: "accounts", reduceMotion: true },
  ]);
});
test("Escape clears search, then closes detail, then window and restores editor focus", async () => {
  const { model, effects } = fixture({ extensions: [extension()] });
  model.show();
  model.viewExtension("fixture-0");
  await model.search("Fixture");
  model.key({ key: "Escape" });
  await Promise.resolve();
  expect(model.query).toBe("");
  expect(model.detailId).toBe("fixture-0");
  model.key({ key: "Escape" });
  expect(model.detailId).toBeUndefined();
  expect(model.open).toBe(true);
  model.key({ key: "Escape" });
  expect(model.open).toBe(false);
  expect(effects.at(-1)).toEqual(["close"]);
});
test("Cmd/Ctrl comma toggle, exact modifiers, repeat guard, Cmd F and Cmd W", () => {
  const { model, effects } = fixture();
  expect(model.key({ key: ",", metaKey: true, shiftKey: true })).toBe(false);
  model.key({ key: ",", metaKey: true });
  expect(model.open).toBe(true);
  model.key({ key: ",", metaKey: true, repeat: true });
  expect(model.open).toBe(true);
  model.key({ key: "f", metaKey: true });
  expect(effects.at(-1)).toEqual(["focus"]);
  model.key({ key: "w", metaKey: true });
  expect(model.open).toBe(false);
  model.key({ key: ",", ctrlKey: true });
  expect(model.open).toBe(true);
  model.key({ key: ",", metaKey: true });
  expect(model.open).toBe(false);
  expect(model.key({ key: "Space" })).toBe(false);
});
test("extension filters, in-place details and stale publications preserve identity", () => {
  const { model } = fixture({
    extensions: [extension(), { ...extension(1), type: "Commands" }],
  });
  model.filterExtensions("Fixture 1", "Commands");
  expect(model.extensions.map((e) => e.id)).toEqual(["fixture-1"]);
  model.viewExtension("fixture-0");
  expect(model.detail).toBeUndefined();
  model.viewExtension("fixture-1");
  expect(model.detail?.id).toBe("fixture-1");
  expect(model.publish(publication({ revision: 1 }))).toBe(false);
  expect(model.detail?.id).toBe("fixture-1");
  expect(model.publish(publication({ revision: 2 }))).toBe(true);
  expect(model.detail).toBeUndefined();
});
test("60k extension virtual range is bounded; stable first-visible anchor survives prepend", () => {
  const extensions = Array.from({ length: 60_000 }, (_, i) => extension(i));
  const { model } = fixture({ extensions });
  model.viewport(30_000, 20);
  expect(model.extensions.length).toBe(60_000);
  expect(model.visibleExtensions).toHaveLength(60);
  expect(model.range).toEqual([29_980, 30_040]);
  model.publish(
    publication({
      revision: 2,
      extensions: [extension(60_000), ...extensions],
    }),
  );
  expect(model.range).toEqual([29_981, 30_041]);
  expect(model.anchorId).toBe("fixture-30000");
  for (const args of [
    [-1, 20],
    [NaN, 2],
    [0, Infinity],
    [0.1, 1],
  ])
    expect(() => model.viewport(args[0], args[1])).toThrow();
});
test("model service actions acknowledge; archive never invokes a raw delete", async () => {
  const { model, actions } = fixture({
    extensions: [extension(), { ...extension(1), canArchive: false }],
  });
  expect(
    await model.perform({ type: "extension.archive", id: "fixture-1" }),
  ).toBe(false);
  expect(
    await model.perform({ type: "extension.archive", id: "fixture-0" }),
  ).toBe(true);
  expect(actions).toEqual([{ type: "extension.archive", id: "fixture-0" }]);
  expect(
    await model.perform({ type: "row", id: "density", action: "invented" }),
  ).toBe(false);
});
test("a published account row replaces the disabled catalogue row and performs its action", async () => {
  const { model, actions } = fixture();
  expect(model.rows.find((r) => r.id === "account.chatgpt")?.disabled).toBe(
    true,
  );
  model.publish(
    publication({
      revision: 2,
      rows: [
        {
          id: "account.chatgpt",
          section: "accounts",
          group: "Subscriptions",
          label: "ChatGPT",
          control: "action",
          action: "account.connect",
          actionLabel: "Sign In…",
        },
      ],
    }),
  );
  expect(
    await model.perform({
      type: "row",
      id: "account.chatgpt",
      action: "account.connect",
    }),
  ).toBe(true);
  expect(actions).toEqual([
    { type: "row", id: "account.chatgpt", action: "account.connect" },
  ]);
});
test("secure input cannot be persisted or published through ordinary row writes", async () => {
  const { model, actions, requests } = fixture();
  expect(await model.write("account.key", "synthetic-key")).toBe(false);
  expect(
    await model.perform({
      type: "write",
      id: "account.key",
      value: "synthetic-key",
    }),
  ).toBe(false);
  expect(actions).toEqual([]);
  expect(requests).toEqual([]);
});
for (const [state, action] of [
  ["available", "download"],
  ["downloading", "cancel"],
  ["installed", "remove"],
  ["error", "retry"],
] as const)
  test(`speech ${state} routes ${action} through the service`, async () => {
    const { model, actions } = fixture({
      speech: [{ id: "m", name: "Fixture", size: "Fixture", state }],
    });
    expect(await model.perform({ type: "speech", id: "m", action })).toBe(true);
    expect(actions).toEqual([{ type: "speech", id: "m", action }]);
    expect(await model.perform({ type: "speech", id: "missing", action })).toBe(
      false,
    );
  });
test("dispose suppresses late hydration and service publications", async () => {
  const { model, effects } = fixture();
  const pending = model.hydrate();
  model.dispose();
  await pending;
  expect(model.publish(publication({ revision: 99 }))).toBe(false);
  model.show();
  expect(effects).toEqual([]);
});
test("in-flight duplicate writes are suppressed and retired project results are fenced", async () => {
  let resolve!: (value: Publication) => void;
  const f = fixture();
  const model = new SettingsModel(
    f.host,
    publication({ extensions: [extension()] }),
    f.window,
    {
      perform: () =>
        new Promise((r) => {
          resolve = r;
        }),
    },
  );
  const pending = model.perform({ type: "extension.archive", id: "fixture-0" });
  expect(
    await model.perform({ type: "extension.archive", id: "fixture-0" }),
  ).toBe(false);
  model.publish(
    publication({ revision: 3, project: { id: "new", name: "Fixture" } }),
  );
  resolve(publication({ revision: 100 }));
  expect(await pending).toBe(false);
  expect(model.publication.revision).toBe(3);
});

test("retired controller cannot enqueue new preference writes or native effects", async () => {
  const { model, effects, requests } = fixture();
  model.show();
  model.dispose();
  const before = effects.length;
  expect(await model.write("appearance", "dark")).toBe(false);
  await model.hydrate();
  model.select("accounts");
  model.close();
  model.trackSection("accounts");
  expect(model.key({ key: "f", metaKey: true })).toBe(false);
  expect(requests).toEqual([]);
  expect(effects).toHaveLength(before);
});

test("ordinary extension-scoped settings client cannot read or write app preferences", async () => {
  const f = fixture();
  const model = new SettingsModel(f.host, publication(), f.window);
  await model.hydrate();
  expect(await model.write("appearance", "dark")).toBe(false);
  expect(model.rows.find((row) => row.id === "appearance")?.disabled).toBe(
    true,
  );
  expect(model.rows.find((row) => row.id === "preview-memory")?.disabled).toBe(
    true,
  );
  expect(f.requests).toEqual([]);
  model.dispose();
});

test("profile adapter declined acknowledgements never publish read or write values", async () => {
  const f = fixture();
  const model = new SettingsModel(f.host, publication(), f.window, undefined, {
    scope: "kernel-profile",
    getSetting: async (key) => ({
      ...(await f.host.getSetting(key)),
      accepted: false,
      value: "dark",
    }),
    setSetting: async (key, value) => ({
      ...(await f.host.getSetting(key)),
      accepted: false,
      value,
    }),
  });
  await model.hydrate();
  expect(model.rows.find((row) => row.id === "appearance")?.value).toBe(
    "system",
  );
  expect(await model.write("appearance", "dark")).toBe(false);
  expect(model.rows.find((row) => row.id === "appearance")?.value).toBe(
    "system",
  );
  expect(f.settings.size).toBe(0);
  model.dispose();
});

test("project memory is read-only, searchable and only available with a project", async () => {
  const { model } = fixture({
    project: { id: "p", name: "Synthetic" },
    memory: {
      enabled: false,
      entries: [
        {
          id: "note",
          kind: "decision",
          text: "Synthetic intent",
          why: "Synthetic reason",
          author: "agent",
          updatedAt: 1000,
          resolved: true,
        },
      ],
    },
  });
  model.show("memory");
  expect(model.active).toBe("memory");
  await model.search("Synthetic reason");
  expect(model.visibleSections).toEqual(["memory"]);
  expect(model.publication.memory?.entries[0].text).toBe("Synthetic intent");
  expect(await model.write("note", "Changed")).toBe(false);
  model.publish(publication({ revision: 2 }));
  expect(model.availableSections).not.toContain("memory");
  expect(model.active).toBe("general");
});
