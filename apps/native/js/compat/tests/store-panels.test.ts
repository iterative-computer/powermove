import { expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import {
  createSandboxAPI,
  sandboxControl,
  sandboxReporter,
} from "../src/frozen/host/store-api";
import { panelFailure, panelStyle } from "../src/panels";
import { harness } from "../../remote-dom/test/harness";

const bootstrap = readFileSync(
  new URL(
    "../../../crates/compat-runner/src/store-bootstrap.js",
    import.meta.url,
  ),
  "utf8",
);

test("Store Library panel placement survives a delayed open and failed notes view offers Reload", async () => {
  const h = harness();
  const scope = {};
  const calls: string[] = [];
  const runtime = {
    engine: (method: string, input: any) => {
      if (method === "store.rpc") calls.push(input.method);
      return method === "store.rpc" ? { value: {} } : true;
    },
  };
  const StoreCompat = {
    createSandboxAPI,
    sandboxControl,
    sandboxReporter,
    panelFailure,
    panelStyle,
    RemoteDOM: function () {
      return h.shim;
    },
    runtimeModules: {},
  };
  const check = new Function(
    "globalThis",
    "StoreCompat",
    "runtime",
    `${bootstrap};return globalThis.__storeCheck;`,
  )(scope, StoreCompat, runtime);
  check.create({
    id: "synthetic-notes",
    apiVersion: 3,
    manifest: { name: "Synthetic Motion Notes" },
    theme: { scheme: "dark" },
    state: { generation: 1 },
  });
  let fail = true;
  try {
    const activation = await check.activate({
      default(api: any) {
        api.panels.register({
          id: "notes",
          title: "Notes",
          build(body: any) {
            const note = h.shim.document.createElement("textarea");
            note.value = "Synthetic motion note";
            body.append(note);
            if (fail) throw Error("Synthetic notes load failure");
          },
        });
      },
    });
    expect(activation.ok).toBe(true);
    check.placePanel("synthetic-notes.notes", [300, 40, 220, 300]);
    check.openPanel("synthetic-notes.notes", "notes");
    await h.settle();
    const doc = h.page.window.document;
    expect(doc.body.textContent).toContain("Synthetic Motion Notes");
    expect(doc.body.textContent).toContain("Synthetic notes load failure");
    expect(doc.querySelector("section")!.getAttribute("style")).toContain(
      "left:300px",
    );
    fail = false;
    doc.querySelector("button")!.click();
    await h.settle();
    expect(calls).toContain("reload");
    // The host restarts the helper after acknowledging this owner-scoped RPC.
    check.closePanel("synthetic-notes.notes");
    check.openPanel("synthetic-notes.notes", "notes");
    await h.settle();
    expect(doc.querySelector("textarea")!.value).toBe("Synthetic motion note");
    expect(doc.querySelector("section")!.getAttribute("style")).toContain(
      "left:300px",
    );
    expect(doc.querySelectorAll("section")).toHaveLength(1);
    expect(h.errors).toEqual([]);
  } finally {
    check.dispose();
    h.close();
  }
});
