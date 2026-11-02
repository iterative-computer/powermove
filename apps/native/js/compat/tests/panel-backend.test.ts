import { expect, test } from "bun:test";
import { CompatPanels, CompatRealm } from "../src";
import { FakeHost } from "./fake-host";
import { harness } from "../../remote-dom/test/harness";

test("panel backend mounts real remote nodes, refreshes overrides and disposes owned views", async () => {
  const h = harness();
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    initial: new FakeHost().publication(),
    host: new FakeHost(),
    dom: h.shim,
  });
  const panels = new CompatPanels(realm.kernel.panels, h.shim.document);
  let cleaned = 0;
  const registration = realm.kernel.panels.register("synthetic-owner", {
    id: "synthetic-panel",
    build(target: any) {
      const output = h.shim.document.createElement("output");
      output.textContent = "Synthetic panel";
      target.append(output);
      return () => cleaned++;
    },
  });
  try {
    panels.open("synthetic-panel");
    panels.open("synthetic-panel");
    await h.settle();
    expect(h.page.window.document.querySelectorAll("output")).toHaveLength(1);
    panels.refresh("synthetic-panel");
    expect(cleaned).toBe(1);
    panels.close("synthetic-panel");
    expect(panels.isHidden("synthetic-panel")).toBe(true);
    expect(cleaned).toBe(2);
    panels.open("synthetic-panel");
    registration.dispose();
    await h.settle();
    expect(panels.isOpen("synthetic-panel")).toBe(false);
    expect(h.page.window.document.querySelector("output")).toBeNull();
    expect(cleaned).toBe(3);
  } finally {
    panels.dispose();
    realm.dispose();
    h.close();
  }
});

test("frozen icon helper creates the reference SVG in the remote document", async () => {
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
    const extension = realm.extension("synthetic-icons", 3);
    const icon = extension.api.ui.icon("play");
    h.shim.document.body.append(icon);
    await h.settle();
    const svg = h.page.window.document.querySelector("svg")!;
    expect(svg.getAttribute("viewBox")).toBe("0 0 256 256");
    expect(svg.getAttribute("data-icon")).toBe("play");
    expect(svg.getAttribute("data-icon-set")).toBe("phosphor");
    expect(svg.querySelector("path")).not.toBeNull();
    expect(h.errors).toEqual([]);
  } finally {
    realm.dispose();
    h.close();
  }
});
