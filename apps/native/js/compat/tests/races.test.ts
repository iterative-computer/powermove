import { expect, test } from "bun:test";
import { CompatRealm, EditGesture } from "../src";
import { copy } from "../src/mirror";
import { FakeHost } from "./fake-host";
import type { Data, Request } from "../src/types";
const setup = () => {
  const host = new FakeHost();
  const realm = new CompatRealm({
    id: "race",
    generation: 1,
    initial: host.publication(),
    host,
  });
  return { host, realm, PM: realm.PM };
};

test("lost write acknowledgement before a newer native publication cannot resurrect an acknowledged value", () => {
  const { host, realm, PM } = setup();
  PM.proj.name = "acknowledged then superseded";
  host.failAfterApply = true;
  expect(() => realm.flush()).toThrow("lost acknowledgement");
  host.state.document.name = "newer native edit";
  host.state.revision++;
  PM.proj.later = "later local intent";
  realm.receive(host.publication());
  realm.flush();
  expect(PM.proj.name).toBe("newer native edit");
  expect(host.state.document).toMatchObject({
    name: "newer native edit",
    later: "later local intent",
  });
  expect(
    host.requests.filter((r) => r.method === "document.directWrite"),
  ).toHaveLength(2);
});
test("a rejected write with a lost reply discards submitted writes and retains subsequent intent", () => {
  const { host, realm, PM } = setup();
  PM.proj.name = "rejected";
  host.rejectNextWrite = host.failAfterApply = true;
  expect(() => realm.flush()).toThrow("lost acknowledgement");
  PM.proj.extra = "later";
  expect(() => realm.flush()).toThrow("validation rejection");
  expect(PM.proj.name).toBe("Synthetic trace fixture");
  expect(PM.proj.extra).toBe("later");
  realm.flush();
  expect(host.state.document.extra).toBe("later");
});
test("requests are immutable down to patch values while awaiting their acknowledgement", () => {
  const { realm, host, PM } = setup();
  let sent: Request | undefined;
  const call = host.call.bind(host);
  host.call = (request) => {
    sent = request;
    return call(request);
  };
  host.failAfterApply = true;
  PM.proj.extra = { nested: [1] };
  expect(() => realm.flush()).toThrow();
  expect(Object.isFrozen(sent)).toBe(true);
  expect(Object.isFrozen(sent!.args.ops[0].value.nested)).toBe(true);
  realm.flush();
  expect(host.requests).toHaveLength(1);
});
test("invalid local JSON resyncs when an unsolicited native publication arrives", () => {
  const { realm, PM, host } = setup(),
    root = PM.proj;
  PM.proj.bad = NaN;
  host.state.document.name = "native";
  host.state.revision++;
  expect(() => realm.receive(host.publication())).toThrow("finite");
  expect(PM.proj).toBe(root);
  expect(PM.proj.bad).toBeUndefined();
  expect(PM.proj.name).toBe("native");
  realm.flush();
  expect(host.requests).toHaveLength(0);
});
test("publication observers see authoritative state, can reenter, and do not mask a rejection", () => {
  const { realm, PM, host } = setup();
  const order: string[] = [];
  realm.extension("observer").api.events.on("project:changed", () => {
    order.push(PM.proj.name);
    PM.touch();
  });
  PM.bus.on("project:changed", () => {
    throw new Error("observer failure");
  });
  const call = host.call.bind(host);
  host.call = (request) => {
    const reply = call(request);
    if (request.method === "document.directWrite")
      reply.publication!.events = [{ name: "project:changed" }];
    return reply;
  };
  PM.proj.name = "bad";
  host.rejectNextWrite = true;
  expect(() => realm.flush()).toThrow("validation rejection");
  expect(order).toEqual(["Synthetic trace fixture"]);
  expect(host.errors).toHaveLength(1);
  expect(realm.channel.pending).toBe(false);
});
test.each(["history.undo", "history.externalDone"])(
  "lost acknowledgement at %s runs an external continuation once",
  (method) => {
    const { realm, host, PM } = setup();
    let count = 0;
    realm.history.external(
      "external",
      () => {
        count++;
        realm.edit.apply({ type: "add_layer", id: "inline" });
      },
      () => {},
    );
    const call = host.call.bind(host);
    let lost = false;
    host.call = (request) => {
      if (request.method === method && !lost) {
        host.failAfterApply = true;
        lost = true;
      }
      return call(request);
    };
    expect(() => realm.history.undo()).toThrow("lost acknowledgement");
    realm.flush();
    expect(count).toBe(1);
    expect(PM.proj.layers.map((layer: Data) => layer.id)).toEqual(["inline"]);
    expect(
      host.requests.filter((r) => r.method === "history.externalDone"),
    ).toHaveLength(1);
  },
);
test("external undo and a page command reenter synchronously with cursor movement already visible", () => {
  const { realm, host, PM } = setup();
  const order: string[] = [];
  const api = realm.extension("page").api;
  api.commands.register({
    id: "page-command",
    run: () => {
      order.push("page");
      return api.edit.apply({ type: "add_layer", id: "page-layer" });
    },
  });
  realm.history.external(
    "external",
    () => {
      order.push("undo");
      expect(PM.hist.canRedo()).toBe(true);
      api.commands.run("page-command");
      order.push("done");
    },
    () => {},
  );
  realm.history.undo();
  expect(order).toEqual(["undo", "page", "done"]);
  expect(host.state.document.layers[0].id).toBe("page-layer");
});
test("gesture previews publish on each write before commit and command cancel restores them", () => {
  const { realm, host, PM } = setup();
  const gesture = new EditGesture(realm, {
    mode: "command",
    label: "Preview",
    command: (value) => ({ type: "add_layer", id: String(value) }),
  });
  gesture.begin();
  gesture.write("preview");
  expect(host.state.document.layers[0].id).toBe("preview");
  expect(PM.proj.layers[0].id).toBe("preview");
  expect(PM.hist.canUndo()).toBe(false);
  gesture.cancel();
  expect(host.state.document.layers).toEqual([]);
});
test("dirty direct writes join a live native edit without silently auto-committing the gesture", () => {
  const { realm, host, PM } = setup();
  realm.edit.begin("native gesture");
  PM.proj.extra = "page";
  host.state.document.native = true;
  host.state.revision++;
  realm.receive(host.publication());
  realm.flush();
  expect(host.state.document).toMatchObject({ extra: "page", native: true });
  expect(host.live).toBe(true);
  expect(host.pending).toBeDefined();
  // This double follows the tag's whole-scope rollback. Native selective cancel
  // is an actor obligation and is deliberately not certified by this assertion.
});
test("cleanup failures retire all handles exactly once without losing the acknowledgement", () => {
  const { realm, host, PM } = setup();
  let cleaned = 0;
  for (let i = 0; i < 2; i++)
    realm.history.external(
      "cleanup",
      () => {},
      () => {},
      {
        cleanup: () => {
          cleaned++;
          throw new Error("cleanup failure");
        },
      },
    );
  PM.hist.clear();
  expect(cleaned).toBe(2);
  expect(host.pins.size).toBe(0);
  expect(host.errors).toHaveLength(2);
});
test("a failed live batch preserves the preceding preview and selection references", () => {
  const { realm, PM } = setup();
  const selected = PM.sel;
  const before = copy(selected);
  realm.edit.begin("preview");
  realm.edit.dispatch({ type: "add_layer", id: "before" });
  expect(
    realm.edit.apply([
      { type: "add_layer", id: "discarded" },
      { type: "invalid" },
    ]).ok,
  ).toBe(false);
  expect(PM.proj.layers.map((l: Data) => l.id)).toEqual(["before"]);
  expect(PM.sel).toBe(selected);
  expect(PM.sel).toEqual(before);
  realm.edit.cancel();
});
test("lost structural finish acknowledgement never converts resync into a deletion of acknowledged work", () => {
  const { realm, PM, host } = setup(),
    call = host.call.bind(host);
  let ran = 0;
  host.call = (request) => {
    if (request.method === "edit.mutateFinish") host.failAfterApply = true;
    return call(request);
  };
  const result = realm.edit.mutate("structural", () => {
    ran++;
    PM.proj.layers.push({ id: "acknowledged" });
  });
  expect(result.ok).toBe(false);
  expect(realm.channel.pending).toBe(true);
  realm.flush();
  expect(ran).toBe(1);
  expect(PM.proj.layers.map((layer: Data) => layer.id)).toEqual([
    "acknowledged",
  ]);
  expect(host.state.document.layers[0].id).toBe("acknowledged");
  expect(
    host.requests.filter(
      (request) => request.method === "document.directWrite",
    ),
  ).toHaveLength(0);
});
test("lost structural-start acknowledgement retires the unused callback lease without running the action later", () => {
  const { realm, PM, host } = setup(),
    call = host.call.bind(host);
  let ran = 0,
    lost = false;
  host.call = (request) => {
    if (request.method === "edit.mutateBegin" && !lost) {
      host.failAfterApply = lost = true;
    }
    return call(request);
  };
  expect(() =>
    realm.edit.mutate("not run", () => {
      ran++;
      PM.proj.layers.push({ id: "must-not-run" });
    }),
  ).toThrow("lost acknowledgement");
  realm.flush();
  expect(ran).toBe(0);
  expect(PM.proj.layers).toEqual([]);
  expect(PM.hist.busy()).toBe(false);
  expect(
    host.requests.filter((request) => request.method === "edit.mutateAbort"),
  ).toHaveLength(1);
});
test("headless inspection exposes lifecycle and pending operation metadata without extension vars", () => {
  const { realm, host, PM } = setup();
  const extension = realm.extension("synthetic-inspected", 2, {
    vars: { SYNTHETIC_VALUE: "public-fixture" },
  });
  host.failAfterApply = true;
  PM.proj.name = "pending";
  expect(() => realm.flush()).toThrow();
  expect(realm.inspect()).toMatchObject({
    realm: "race",
    generation: 1,
    lifecycle: "active",
    pendingMethod: "document.directWrite",
    extensions: ["synthetic-inspected"],
  });
  expect(JSON.stringify(realm.inspect())).not.toContain("public-fixture");
  realm.flush();
  extension.dispose();
  realm.dispose();
  expect(realm.inspect().lifecycle).toBe("retired");
});
test("inline host continuations preserve writes made after the request's flush", () => {
  const { realm, host, PM } = setup(),
    call = host.call.bind(host);
  host.call = (request) => {
    const reply = call(request);
    if (request.method === "document.touch") PM.proj.name = "inline write";
    return reply;
  };
  PM.touch();
  expect(PM.proj.name).toBe("inline write");
  realm.flush();
  expect(host.state.document.name).toBe("inline write");
});
