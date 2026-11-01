import { expect, test } from "bun:test";
import { CompatRealm } from "../src";
import { FakeHost } from "./fake-host";
import {
  activateRegistrations,
  activateRuntime,
  manifests,
} from "./corpus/extensions";
import { frozen } from "../scripts/freeze-host";
import { tagSource } from "./corpus/reference";
import ts from "typescript";
import type { Data } from "../src/types";

for (const manifest of manifests)
  test(`synthetic extension apiVersion ${manifest.apiVersion}: all contribution families, ownership and callbacks`, () => {
    const host = new FakeHost(),
      realm = new CompatRealm({
        id: "corpus",
        generation: 1,
        initial: host.publication(),
        host,
      });
    const extension = realm.extension(
      manifest.id,
      manifest.apiVersion as 1 | 2 | 3,
      { manifest },
    );
    const trace: any[] = [];
    activateRegistrations(extension.api, trace);
    const kernel = realm.kernel,
      api = extension.api,
      argument = {};
    expect(api.commands.run("synthetic-command", argument)).toBe(argument);
    expect(
      api.keybindings.chordOf({ key: "k", metaKey: true, shiftKey: true }),
    ).toBe("cmd+shift+k");
    expect(api.keybindings.list()).toHaveLength(1);
    expect(api.effects.get("SyntheticFx")).toBe(api.effects.list()[0]);
    expect(api.transitions.get("SyntheticTransition")).toBe(
      api.transitions.list()[0],
    );
    expect(api.layers.get("synthetic-layer")).toBe(api.layers.list()[0]);
    expect(api.model.layerDefinition("synthetic-layer")).toBe(
      api.layers.get("synthetic-layer"),
    );
    const target: Data = {
      replaceChildren() {
        this.textContent = "";
      },
    };
    kernel.panels.get("synthetic-panel").build(target, { spec: {} });
    expect(target.textContent).toBe("mounted");
    const mount = api.inspector.sections()[0].build(target, { layerIds: [] });
    expect(target.textContent).toBe("inspector");
    kernel.paletteProviders()[0].provider("probe")[0].run();
    const menu = api.menus.collect("panel:context", { label: "Synthetic" });
    expect(menu[0].label).toBe("Synthetic");
    menu[0].action();
    expect(api.status.list()[0].text()).toBe("Synthetic");
    api.status.list()[0].onClick();
    api.events.emit("synthetic:event", argument);
    expect(api.theme.active()).toBe("synthetic-theme");
    expect(api.theme.scheme()).toBe("dark");
    extension.dispose();
    extension.dispose();
    expect(target.textContent).toBe("");
    expect(trace.filter((value) => value === "inspector-cleanup")).toHaveLength(
      1,
    );
    expect(trace.filter((value) => value === "disposed")).toHaveLength(1);
    for (const name of [
      "panels",
      "commands",
      "keybindings",
      "effects",
      "transitions",
      "layerTypes",
      "themes",
      "status",
      "inspectorSections",
    ])
      expect(kernel[name].list(), name).toEqual([]);
    expect(kernel.paletteProviders()).toEqual([]);
    expect(api.menus.collect("panel:context")).toEqual([]);
    expect(api.theme.active()).toBe("default");
    expect(api.theme.scheme()).toBe("system");
    api.effects.register({
      id: "Late",
      label: "Late",
      group: "Synthetic",
      params: [],
      frag: "synthetic",
    });
    expect(api.effects.list()).toEqual([]);
  });
for (const manifest of manifests)
  test(`synthetic extension apiVersion ${manifest.apiVersion}: runtime, storage namespaces and activation vars`, () => {
    const host = new FakeHost(),
      values = new Map<string, Data>();
    const realm = new CompatRealm({
      id: "runtime",
      generation: 1,
      initial: host.publication(),
      host,
      capabilities: {
        PM: {
          store: {
            get: (key: string, fallback: Data) => values.get(key) ?? fallback,
            set: (key: string, value: Data) => values.set(key, value),
          },
        },
      },
    });
    const vars = { SYNTHETIC_VALUE: "public-fixture" },
      trace: any[] = [];
    const extension = realm.extension(
      manifest.id,
      manifest.apiVersion as 1 | 2 | 3,
      { manifest, vars },
    );
    vars.SYNTHETIC_VALUE = "different";
    activateRuntime(extension.api, trace);
    expect(trace).toEqual([
      true,
      true,
      true,
      true,
      { value: 1 },
      ["SYNTHETIC_VALUE"],
      "public-fixture",
    ]);
    expect(values.get("ext." + manifest.id)).toEqual({});
    const other = realm.extension("synthetic-other");
    other.api.storage.set("synthetic", 2);
    expect(extension.api.storage.get("synthetic")).toBeUndefined();
    expect(other.api.storage.get("synthetic")).toBe(2);
    extension.dispose();
    expect(realm.services.get("synthetic-service")).toBeNull();
  });
test("every typed synchronous service family preserves live provider/callback identity through override and disposal", async () => {
  const source = await tagSource("kernel/api.ts"),
    tree = ts.createSourceFile("api.ts", source, ts.ScriptTarget.Latest, true);
  const names = new Set([
    "TimelineService",
    "ViewerService",
    "InspectorService",
    "ToolService",
    "ShaderHooks",
    "CaptionsService",
  ]);
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "services",
      generation: 1,
      initial: host.publication(),
      host,
    });
  const provider = realm.extension("synthetic-provider"),
    override = realm.extension("synthetic-override");
  let count = 0;
  for (const node of tree.statements)
    if (ts.isInterfaceDeclaration(node) && names.has(node.name.text)) {
      const object: Data = {};
      for (const member of node.members)
        object[member.name!.getText(tree)] = ts.isMethodSignature(member)
          ? (value: unknown) => value
          : { live: true };
      provider.api.services.register(node.name.text, object);
      const held = override.api.services.get(node.name.text);
      expect(held).toBe(object);
      for (const [name, value] of Object.entries(object))
        if (typeof value === "function") {
          const arg = {};
          expect(held[name](arg)).toBe(arg);
        }
      const replacement = override.api.services.register(node.name.text, {});
      replacement.dispose();
      expect(override.api.services.get(node.name.text)).toBe(held);
      count++;
    }
  expect(count).toBe(6);
  provider.dispose();
  override.dispose();
});
test("workspace layout edits retain hidden panel geometry and appearance ownership preserves later user changes", () => {
  const host = new FakeHost(undefined, {
    id: "synthetic-workspace",
    name: "Synthetic",
    density: "normal",
    layout: {
      docks: [
        {
          id: "right",
          panels: [
            { id: "a", size: 210 },
            { id: "b", size: 320 },
          ],
        },
      ],
    },
  });
  const realm = new CompatRealm({
    id: "workspace",
    generation: 1,
    initial: host.publication(),
    host,
  });
  const extension = realm.extension("workspace-owner"),
    api = extension.api;
  expect(api.workspace.hasPanel("a")).toBe(true);
  expect(api.workspace.hidePanel("a")).toBe(true);
  expect(api.workspace.hasPanel("a")).toBe(false);
  expect(api.workspace.restorePanel("a")).toBe(true);
  expect(
    host.state.workspace.layout.docks[0].panels.find(
      (panel: Data) => panel.id === "a",
    ).size,
  ).toBe(210);
  api.workspace.addPanel("c", "left");
  expect(api.workspace.movePanel("c", "right", 0)).toBe(true);
  api.workspace.mutate((w: Data) => {
    w.density = "compact";
  });
  // A user's subsequent edit is authoritative and survives owner release.
  realm.workspace.mutate((w) => {
    w.density = "comfortable";
  });
  extension.dispose();
  expect(host.state.workspace.density).toBe("comfortable");
  expect(
    realm.workspace
      .current()
      .layout.docks.some((dock: Data) =>
        dock.panels.some((panel: Data) => panel.id === "c"),
      ),
  ).toBe(true);
});
test("frozen host graph regenerates exactly from v1.2.0 plus recorded adaptations", async () => {
  for (const name of [
    "host",
    "registries",
    "glsl",
    "extension-layers",
    "import-defaults",
    "workspace-appearance",
    "theme-state",
    "panel-frame",
    "layout-model",
    "side-scroll",
    "drop",
    "binding",
    "pm-facades",
  ]) {
    const expected = await frozen(name),
      actual = await Bun.file(
        new URL(`../src/frozen/host/${name}.js`, import.meta.url),
      ).text();
    expect(actual, name).toBe(expected);
  }
});
test("asynchronous command when remains synchronous for listings and settles to its latest answer", async () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "when",
      generation: 1,
      initial: host.publication(),
      host,
    });
  const api = realm.extension("async-when").api;
  let answer!: (value: boolean) => void;
  api.commands.register({
    id: "async",
    label: "Synthetic",
    when: () =>
      new Promise<boolean>((resolve) => {
        answer = resolve;
      }),
    run: () => false,
  });
  const definition = api.commands.list()[0];
  expect(definition.when()).toBe(true);
  answer(false);
  await Promise.resolve();
  await Promise.resolve();
  expect(definition.when()).toBe(false);
  expect(api.commands.run("async")).toBe(false);
});
test("menu collect ignores asynchronous results while gather awaits ordered contributions", async () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "menu",
      generation: 1,
      initial: host.publication(),
      host,
    });
  const api = realm.extension("menus").api;
  api.menus.contribute("panel:context", () => [{ label: "sync" }]);
  api.menus.contribute("panel:context", async () => [{ label: "async" }]);
  expect(api.menus.collect("panel:context")).toEqual([{ label: "sync" }]);
  expect(await api.menus.gather("panel:context")).toEqual([
    { label: "sync" },
    { label: "async" },
  ]);
});
test("activation respects the user's hidden panel choice; later explicit opens reach the host", () => {
  const opened: string[] = [],
    host = new FakeHost();
  const realm = new CompatRealm({
    id: "panels",
    generation: 1,
    initial: host.publication(),
    host,
    capabilities: {
      panelsBackend: {
        isHidden: () => true,
        open: (id: string) => opened.push(id),
      },
    },
  });
  const extension = realm.extension("hidden");
  extension.setActivating(true);
  extension.api.panels.open("hidden");
  expect(opened).toEqual([]);
  extension.setActivating(false);
  extension.api.panels.open("hidden");
  expect(opened).toEqual(["hidden"]);
});
test("media import defaults validate and own their stack; reads cannot mutate a registered default", () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "media",
      generation: 1,
      initial: host.publication(),
      host,
    });
  const a = realm.extension("a"),
    b = realm.extension("b");
  a.api.media.registerImportDefaults({ anchor: { x: 0.5, y: 0.5 } });
  b.api.media.registerImportDefaults({ anchor: { x: 0, y: 1 } });
  const value = b.api.media.getImportDefaults();
  value.anchor.x = 0.25;
  expect(a.api.media.getImportDefaults()).toEqual({ anchor: { x: 0, y: 1 } });
  expect(() =>
    b.api.media.registerImportDefaults({ anchor: { x: NaN, y: 0 } }),
  ).toThrow("finite");
  b.dispose();
  expect(a.api.media.getImportDefaults()).toEqual({
    anchor: { x: 0.5, y: 0.5 },
  });
  a.dispose();
  expect(realm.services.get("media.importDefaults")).toBeNull();
});
test("drag payload families preserve canonical MIME keys, current drag identity and malformed decline", () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "drag",
      generation: 1,
      initial: host.publication(),
      host,
    });
  const api = realm.extension("drag").api;
  const values = new Map<string, string>();
  const transfer = {
    types: [] as string[],
    effectAllowed: "",
    setData(type: string, value: string) {
      values.set(type, value);
      this.types.push(type);
    },
    getData: (type: string) => values.get(type) ?? "",
  };
  const payload = {
    id: "synthetic-asset",
    name: "Synthetic",
    kind: "image",
    dur: 2,
  };
  api.dnd.startAssetDrag(transfer, payload);
  expect(transfer.effectAllowed).toBe("copy");
  expect(api.dnd.hasAssetDrag(transfer)).toBe(true);
  expect(api.dnd.readAssetDrag(transfer)).toEqual(payload);
  api.dnd.mediaDrag = payload;
  expect(api.dnd.mediaDrag).toBe(payload);
  expect(realm.PM.mediaDrag).toBe(payload);
  values.set(api.dnd.ASSET_MIME, "invalid");
  expect(api.dnd.readAssetDrag(transfer)).toBeNull();
  transfer.setData(
    api.dnd.FX_MIME,
    JSON.stringify({ kind: "effect", id: "synthetic-effect" }),
  );
  expect(api.dnd.hasFxDrag(transfer)).toBe(true);
  expect(api.dnd.readFxDrag(transfer)).toEqual({
    kind: "effect",
    id: "synthetic-effect",
    label: "synthetic-effect",
  });
});
test("control binding factories preserve work-area limits and linked-scale authored intent", () => {
  const host = new FakeHost(),
    realm = new CompatRealm({
      id: "bindings",
      generation: 1,
      initial: host.publication(),
      host,
      capabilities: {
        anim: {
          ev: (_layer: unknown, key: string) => (key === "scale.x" ? 50 : 100),
        },
      },
    });
  Object.assign(realm.PM.proj, { dur: 10, fps: 30, work: [1, 9] });
  realm.PM.proj.layers.push({ id: "scaled", scaleLinked: true, p: {} });
  const binding = realm.extension("bindings").api.ui.controls.binding;
  expect(
    binding.layerFieldBinding("scaled", "layer.name").command("Name"),
  ).toEqual({ type: "set_layer", target: "scaled", patch: { name: "Name" } });
  expect(
    binding.contentBinding("scaled", "content.text").command("Caption"),
  ).toEqual({
    type: "set_content",
    target: "scaled",
    patch: { text: "Caption" },
  });
  expect(binding.compositionBinding("workArea.start").command(20)).toEqual({
    type: "set_composition",
    patch: { workArea: [9 - 1 / 30, 9] },
  });
  const scale = binding.channelBinding("scaled", "transform.scale.x", {
    time: 2,
    origin: "synthetic",
    label: "Scale",
  });
  scale.prepare();
  const commands = scale.command(30);
  expect(
    commands.map((command: Data) => [command.path, command.value]),
  ).toEqual([
    ["scale.x", 30],
    ["scale.y", 60],
  ]);
  expect(commands[0]).toMatchObject({
    type: "set_property",
    time: 2,
    mode: "auto",
    preserveHandEdits: false,
    markIntent: "human",
  });
  expect(scale.origin).toBe("synthetic");
});
