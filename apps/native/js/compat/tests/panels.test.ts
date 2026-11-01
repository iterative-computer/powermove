import { test, expect } from "bun:test";
import { compile } from "svelte/compiler";
import { mkdir } from "node:fs/promises";
import { CompatRealm } from "../src";
import { FakeHost } from "./fake-host";
import { harness } from "../../remote-dom/test/harness";

async function panelRuntime() {
  const root = new URL("../dist/panel-test/", import.meta.url).pathname;
  await mkdir(root, { recursive: true });
  const entry = root + "entry.ts";
  await Bun.write(
    entry,
    'export { default as Component } from "../../tests/corpus/Panel.svelte"; export * as svelte from "svelte"; export * as reactivity from "svelte/reactivity";',
  );
  const build = await Bun.build({
    entrypoints: [entry],
    outdir: root,
    naming: "compiled.js",
    target: "browser",
    format: "esm",
    conditions: ["browser", "production"],
    define: { "process.env.NODE_ENV": '"production"' },
    plugins: [
      {
        name: "svelte",
        setup(build) {
          build.onLoad({ filter: /\.svelte$/ }, async ({ path }) => ({
            contents: compile(await Bun.file(path).text(), {
              filename: path,
              generate: "client",
              dev: false,
            }).js.code,
            loader: "js",
          }));
        },
      },
    ],
  });
  if (!build.success)
    throw new Error(build.logs.map((log) => log.message).join("\n"));
  return import(root + "compiled.js?panel-runtime=1");
}
test("Svelte 5 extension panel mounts, tracks native publications, edits through DOM events and disposes", async () => {
  const h = harness(),
    restore = h.shim.installGlobals();
  try {
    const runtime = await panelRuntime(),
      host = new FakeHost();
    const call = host.call.bind(host);
    host.call = (request) => {
      const reply = call(request);
      if (request.method === "document.directWrite")
        reply.publication!.events = [
          { name: "project:changed", value: { kind: "values" } },
        ];
      return reply;
    };
    const realm = new CompatRealm({
      id: "fixture",
      generation: 1,
      initial: host.publication(),
      host,
      dom: h.shim,
      runtimeModules: {
        svelte: runtime.svelte,
        "svelte/reactivity": runtime.reactivity,
      },
    });
    const extension = realm.extension("synthetic-panel", 3);
    const registration = extension.api.panels.register({
      id: "synthetic-panel",
      title: "Synthetic",
      component: runtime.Component,
    });
    const definition = realm.kernel.panels.get("synthetic-panel");
    definition.build(h.shim.document.body, { spec: { label: "mounted" } });
    await h.settle();
    const page = h.page.window.document;
    expect(page.querySelector("output")!.textContent).toBe(
      "Synthetic trace fixture:mounted",
    );
    const input = page.querySelector("input")!;
    input.value = "Remote edit";
    input.dispatchEvent(new h.page.window.Event("input", { bubbles: true }));
    await h.settle();
    runtime.svelte.flushSync();
    host.drain();
    await h.settle();
    expect(host.state.document.name).toBe("Remote edit");
    expect(page.querySelector("output")!.textContent).toBe(
      "Remote edit:mounted",
    );
    host.state.document.name = "Native edit";
    host.state.revision++;
    const publication = host.publication();
    publication.events = [
      { name: "project:changed", value: { kind: "values" } },
    ];
    realm.receive(publication);
    runtime.svelte.flushSync();
    await h.settle();
    expect(page.querySelector("output")!.textContent).toBe(
      "Native edit:mounted",
    );
    // Refresh remounts, preserving one live component instance per panel.
    definition.build(h.shim.document.body, { spec: { label: "rebuilt" } });
    await h.settle();
    expect(page.querySelectorAll("output")).toHaveLength(1);
    registration.dispose();
    await h.settle();
    expect(page.querySelector("output")).toBeNull();
    expect(h.errors).toEqual([]);
    expect(host.errors).toEqual([]);
    extension.dispose();
    realm.dispose();
    expect(h.shim.inspect().lifecycle).toBe("retired");
  } finally {
    restore();
    h.close();
  }
});
test("page lifecycle fences generations, retires pending reads, and keeps background/headless realm callbacks live", async () => {
  const h = harness();
  const host = new FakeHost();
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    initial: host.publication(),
    host,
    dom: h.shim,
  });
  try {
    expect(h.shim.inspect().lifecycle).toBe("connected");
    expect(() =>
      h.shim.connect({ realm: "fixture", generation: 1, task: (fn) => fn() }),
    ).toThrow("already connected");
    const input = h.shim.document.createElement("input");
    input.addEventListener("input", () => {
      realm.PM.proj.name = input.value;
    });
    h.shim.document.body.append(input);
    await h.settle();
    for (const visibility of [
      "background",
      "headless",
      "foreground",
    ] as const) {
      h.shim.setVisibility(visibility);
      const nativeInput = h.page.window.document.querySelector("input")!;
      nativeInput.value = visibility;
      nativeInput.dispatchEvent(
        new h.page.window.Event("input", { bubbles: true }),
      );
      await h.settle();
      host.drain();
      expect(host.state.document.name).toBe(visibility);
      expect(h.shim.inspect().visibility).toBe(visibility);
    }
    const read = h.shim.measure(input);
    const observed = read.catch((error) => error.message);
    h.shim.receive({
      kind: "fault",
      session: h.shim.session,
      seq: h.shim.inspect().sent,
      message: "synthetic page termination",
    });
    expect(await observed).toBe("Remote DOM retired");
    expect(h.shim.inspect().lifecycle).toBe("faulted");
    expect(
      h.shim.receive({ kind: "applied", session: h.shim.session, seq: 1 }),
    ).toBe(false);
    expect(
      h.shim.receive({
        kind: "applied",
        session: { ...h.shim.session, generation: 2 },
        seq: 1,
      }),
    ).toBe(false);
    realm.history.do("logic survives the page", () => {
      realm.PM.proj.name = "kept";
    });
    expect(host.state.document.name).toBe("kept");
  } finally {
    realm.dispose();
    h.close();
  }
});
test("a bridge refuses an unrelated realm generation before exposing its globals", () => {
  const h = harness();
  expect(() =>
    h.shim.connect({ realm: "different", generation: 1, task: (fn) => fn() }),
  ).toThrow("mismatch");
  expect(() =>
    h.shim.connect({ realm: "fixture", generation: 2, task: (fn) => fn() }),
  ).toThrow("mismatch");
  expect(h.shim.inspect().lifecycle).toBe("created");
  h.close();
});
test("remote DOM runs panel handlers before the kernel keymap and keeps input fields in their authored context", async () => {
  const h = harness(),
    restore = h.shim.installGlobals(),
    host = new FakeHost();
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    host,
    initial: host.publication(),
    dom: h.shim,
  });
  try {
    const order: string[] = [],
      api = realm.extension("keys").api;
    api.commands.register({
      id: "synthetic-key",
      run: () => {
        order.push("command");
      },
    });
    api.keybindings.bind({ key: "cmd+k", command: "synthetic-key" });
    const button = h.shim.document.createElement("button"),
      input = h.shim.document.createElement("input");
    button.addEventListener("keydown", () => order.push("target"));
    h.shim.document.body.append(button, input);
    await h.settle();
    const doc = h.page.window.document;
    doc.querySelector("button")!.dispatchEvent(
      new h.page.window.KeyboardEvent("keydown", {
        key: "k",
        metaKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    await h.settle();
    host.drain();
    expect(order).toEqual(["target", "command"]);
    doc.querySelector("input")!.dispatchEvent(
      new h.page.window.KeyboardEvent("keydown", {
        key: "k",
        metaKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    await h.settle();
    expect(order).toEqual(["target", "command"]);
    // This test observes the bridge's unresolved late browser cancellation;
    // it does not assert that preventDefault retroactively cancels WebKit.
    expect(h.late).toContain("keydown");
    expect(h.errors).toEqual([]);
  } finally {
    realm.dispose();
    restore();
    h.close();
  }
});
