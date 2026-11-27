import { expect, test } from "bun:test";
import { CompatRealm, connectControlState } from "../src";
import { FakeHost } from "./fake-host";
import { harness } from "../../remote-dom/test/harness";

test("control read models follow authoritative publications and release subscriptions/styles", () => {
  const host = new FakeHost();
  const realm = new CompatRealm({
    id: "synthetic",
    generation: 1,
    initial: host.publication(),
    host,
  });
  const h = harness();
  let project: any,
    selection: any,
    updates = 0;
  const stores = {
    doc: {
      replace(value: any) {
        project = value;
        updates++;
      },
    },
    setSelection(value: any) {
      selection = value;
    },
    transport: { time: 0, playing: false },
    updateControlTime() {},
  };
  const connection = connectControlState(realm, stores, h.shim.document, [
    ":root{--synthetic:1}",
  ]);
  try {
    expect(project).toBe(realm.PM.proj);
    const next = host.publication();
    next.document.name = "Synthetic replacement";
    next.selection = { layers: ["synthetic-layer"], keys: [], chan: null };
    next.events = [{ name: "project:changed" }, { name: "selection" }];
    realm.receive(next);
    expect(project.name).toBe("Synthetic replacement");
    expect(selection.layers).toEqual(["synthetic-layer"]);
    expect(h.shim.document.head.querySelector("style")).not.toBeNull();
    connection.dispose();
    const previous = updates;
    realm.PM.bus.emit("project:changed");
    expect(updates).toBe(previous);
    expect(h.shim.document.head.querySelector("style")).toBeNull();
  } finally {
    connection.dispose();
    realm.dispose();
    h.close();
  }
});
