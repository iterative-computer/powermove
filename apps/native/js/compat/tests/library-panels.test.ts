import { expect, test } from "bun:test";
import { CompatPanels, CompatRealm } from "../src";
import { createLoader } from "../src/loader";
import { FakeHost } from "./fake-host";
import { harness } from "../../remote-dom/test/harness";

test("Library enable keeps a failed panel visible across registration refresh and Reload recovers", async () => {
  const h = harness(),
    host = new FakeHost();
  let panels: CompatPanels,
    loader: ReturnType<typeof createLoader>,
    reloading: Promise<void> | undefined;
  host.reportError = (report: any) => {
    host.errors.push(report);
    if (report.extensionId)
      loader.reportRuntimeError(report.extensionId, report.error);
  };
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    host,
    initial: host.publication(),
    dom: h.shim,
    capabilities: {
      panelsBackend: { open: (id: string) => panels.open(id) },
      extensions: { reload: (id: string) => (reloading = loader.reload(id)) },
    },
  });
  panels = new CompatPanels(realm.kernel.panels, h.shim.document);
  let fail = true;
  const health: string[] = [];
  loader = createLoader({
    realm,
    document: h.shim.document,
    report: (_, value) => health.push(value.state),
    link: () => [
      Promise.resolve(),
      {
        default(api: any) {
          api.panels.register({
            id: "library-anchor",
            title: "Anchor controls",
            build(body: any) {
              if (fail) throw Error("Synthetic bounds failure");
              body.textContent = "Recovered controls";
            },
          });
          api.panels.open("library-anchor");
        },
      },
    ],
  });
  const record = {
    id: "local-anchor",
    scope: "user",
    trust: "local",
    enabled: false,
    health: { state: "disabled" },
    manifest: { name: "Synthetic Anchor", apiVersion: 1 },
    graph: { sources: {}, edges: {}, hash: "synthetic" },
  };
  try {
    await loader.boot([record]);
    // The dock can already remember a panel while Library turns its owner on.
    panels.open("library-anchor");
    panels.place("library-anchor", [200, 30, 218, 300]);
    await loader.update(
      [{ ...record, enabled: true, health: { state: "ok" } }],
      [record.id],
      "enable",
    );
    await h.settle();
    expect(health).not.toContain("runtime-error");
    expect(h.page.window.document.body.textContent).toContain(
      "Synthetic bounds failure",
    );
    expect(h.page.window.document.body.textContent).toContain(
      "Synthetic Anchor",
    );
    fail = false;
    h.page.window.document.querySelector("button")!.click();
    await h.settle();
    await reloading;
    await h.settle();
    expect(h.page.window.document.body.textContent).toContain(
      "Recovered controls",
    );
  } finally {
    await loader.dispose();
    panels.dispose();
    realm.dispose();
    h.close();
  }
});

test("local component mount failures use the same visible Reload state", async () => {
  const h = harness(),
    host = new FakeHost();
  let panels: CompatPanels,
    fail = true;
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    host,
    initial: host.publication(),
    dom: h.shim,
    capabilities: {
      host: {
        mount: (_component: any, target: any) => {
          if (fail) throw Error("Synthetic component mount failure");
          target.textContent = "Mounted component";
          return () => {
            target.textContent = "";
          };
        },
      },
      panelsBackend: { refresh: (id: string) => panels.refresh(id) },
    },
  });
  panels = new CompatPanels(realm.kernel.panels, h.shim.document);
  const extension = realm.extension("component-owner", 3);
  try {
    extension.api.panels.register({
      id: "component-panel",
      title: "Component fixture",
      component: () => null,
    });
    panels.open("component-panel");
    panels.place("component-panel", [0, 0, 220, 300]);
    await h.settle();
    expect(h.page.window.document.body.textContent).toContain(
      "Synthetic component mount failure",
    );
    fail = false;
    h.page.window.document.querySelector("button")!.click();
    await h.settle();
    expect(h.page.window.document.body.textContent).toContain(
      "Mounted component",
    );
  } finally {
    panels.dispose();
    extension.dispose();
    realm.dispose();
    h.close();
  }
});
