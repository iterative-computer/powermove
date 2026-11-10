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
    const icon = h.shim.document.createElement("span");
    // The shipped API answers with markup, not a node.
    icon.innerHTML = extension.api.ui.icon("play");
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
test("a panel paints only where the shell placed it, even when placed before it opened", async () => {
  const h = harness();
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    initial: new FakeHost().publication(),
    host: new FakeHost(),
    dom: h.shim,
  });
  const panels = new CompatPanels(realm.kernel.panels, h.shim.document);
  for (const id of ["early", "late"])
    realm.kernel.panels.register("owner", { id, build() {} });
  const root = (id: string) =>
    h.page.window.document.querySelector(`[data-pm-contribution="${id}"]`)!;
  panels.place("early", [10, 20, 300, 400]);
  panels.open("early");
  panels.open("late");
  await h.settle();
  expect(root("early").getAttribute("style")).toContain("left:10px;top:20px");
  expect(root("late").getAttribute("style")).toBe("display:none");
  panels.place("late", [0, 0, 100, 100]);
  await h.settle();
  expect(root("late").getAttribute("style")).toContain("width:100px");
  panels.close("early");
  panels.open("early");
  await h.settle();
  expect(root("early").getAttribute("style")).toBe("display:none");
});
test("a panel the shell restored before its extension activated opens when it registers", async () => {
  const h = harness();
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    initial: new FakeHost().publication(),
    host: new FakeHost(),
    dom: h.shim,
  });
  const panels = new CompatPanels(realm.kernel.panels, h.shim.document);
  panels.open("restored");
  panels.place("restored", [0, 0, 50, 50]);
  expect(panels.isOpen("restored")).toBe(false);
  realm.kernel.panels.register("owner", {
    id: "restored",
    build(target: any) {
      target.append(h.shim.document.createElement("output"));
    },
  });
  await h.settle();
  expect(panels.isOpen("restored")).toBe(true);
  expect(h.page.window.document.querySelector("output")).not.toBeNull();
  panels.close("restored");
  realm.kernel.panels.register("owner", { id: "restored", build() {} });
  expect(panels.isOpen("restored")).toBe(false);
});
test("an extension restyling its panel body keeps the shell's placement", async () => {
  const h = harness();
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    initial: new FakeHost().publication(),
    host: new FakeHost(),
    dom: h.shim,
  });
  const panels = new CompatPanels(realm.kernel.panels, h.shim.document);
  realm.kernel.panels.register("owner", {
    id: "styled",
    build(body: any) {
      Object.assign(body.style, { display: "block", height: "100%" });
    },
  });
  panels.open("styled");
  await h.settle();
  const root = h.page.window.document.querySelector(
    '[data-pm-contribution="styled"]',
  )!;
  expect(root.getAttribute("style")).toBe("display:none");
  panels.place("styled", [5, 6, 7, 8]);
  await h.settle();
  expect(root.getAttribute("style")).toContain("left:5px;top:6px");
});
