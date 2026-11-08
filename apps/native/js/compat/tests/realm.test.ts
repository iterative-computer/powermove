import { describe, expect, test } from "bun:test";
import { CompatRealm, EditGesture } from "../src";
import {
  DocumentMirror,
  cloneAcceptingViews,
  copy,
  unwrapViews,
} from "../src/mirror";
import type { Data } from "../src/types";
import { FakeHost } from "./fake-host";
const setup = () => {
  const host = new FakeHost();
  const realm = new CompatRealm({
    id: "fixture",
    generation: 1,
    host,
    initial: host.publication(),
  });
  return {
    host,
    realm,
    PM: realm.PM,
    history: realm.history,
    edit: realm.edit,
  };
};
describe("kernel-backed compatibility returns", () => {
  test("nested history, exact callback identity, inline undo and redo", () => {
    const { history, PM } = setup(),
      value = {};
    expect(history.begin("outer")).toBe(history.begin("inner"));
    expect(
      history.do("folded", () => {
        PM.proj.name = "folded";
        return value;
      }),
    ).toBe(value);
    expect(history.commit()).toBe(false);
    expect(history.commit()).toBe(true);
    expect(history.undo()).toBe(true);
    expect(PM.proj.name).toBe("Synthetic trace fixture");
    expect(history.redo()).toBe(true);
    expect(PM.proj.name).toBe("folded");
  });
  test("throw rolls back and preserves error object; cancel keeps writes", () => {
    const { history, PM } = setup(),
      error = {};
    try {
      history.do("bad", () => {
        PM.proj.name = "bad";
        throw error;
      });
    } catch (caught) {
      expect(caught).toBe(error);
    }
    expect(PM.proj.name).toBe("Synthetic trace fixture");
    history.begin("keep");
    PM.proj.name = "kept";
    expect(history.cancel()).toBeUndefined();
    expect(PM.proj.name).toBe("kept");
    expect(PM.hist.canUndo()).toBe(false);
  });
  test("Promise not awaited; commit precedes continuation", async () => {
    const { history, realm, PM, host } = setup();
    const promise = Promise.resolve(false);
    expect(
      history.do("async", () => {
        PM.proj.name = "sync";
        return promise;
      }),
    ).toBe(promise);
    expect(PM.hist.canUndo()).toBe(true);
    await promise.then(() =>
      realm.task(() => {
        PM.proj.name = "async";
      }),
    );
    host.drain();
    expect(history.undo()).toBe(true);
    expect(PM.proj.name).toBe("Synthetic trace fixture");
  });
  test("edit admission precedes callback and errors have original forms", () => {
    const { edit, history } = setup();
    expect(edit.begin("live")).toBeUndefined();
    expect(() => edit.begin("nested")).toThrow(
      "A source edit is already active",
    );
    let ran = false;
    expect(
      edit.mutate("refused", () => {
        ran = true;
      }),
    ).toEqual({
      ok: false,
      message: "Finish the active source edit before changing structure",
    });
    expect(ran).toBe(false);
    expect(history.begin("raw")).toBeNull();
    history.commit();
    edit.cancel();
    expect(edit.mutate("invalid", null).ok).toBe(false);
    const result = {};
    expect(edit.mutate("noop", () => result).data!.result).toBe(result);
    expect(edit.commit().ok).toBe(false);
    expect(edit.apply([]).ok).toBe(false);
    expect(
      edit.apply({ type: "add_layer" }, { baseRevision: 99 }).message,
    ).toBe("Project changed: expected revision 99, found 0");
  });
  test("structural mutation rejects non-tree changes and keeps result identity", () => {
    const { edit, PM } = setup();
    const project = PM.proj,
      result = {};
    expect(
      edit.mutate("wrong", () => {
        PM.proj.name = "wrong";
      }).ok,
    ).toBe(false);
    expect(PM.proj).toBe(project);
    expect(PM.proj.name).toBe("Synthetic trace fixture");
    expect(
      edit.mutate("tree", () => {
        PM.proj.layers.push({ id: "a" });
        return result;
      }).data!.result,
    ).toBe(result);
    expect(PM.proj.layers[0].id).toBe("a");
    expect(PM.hist.canUndo()).toBe(true);
  });
  test("empty cancel retains mounted references; failed live batch is atomic", () => {
    const { edit, PM } = setup(),
      project = PM.proj;
    edit.begin("empty");
    expect(edit.cancel()).toBe(true);
    expect(PM.proj).toBe(project);
    edit.begin("drag");
    expect(edit.dispatch({ type: "add_layer", id: "a" }).ok).toBe(true);
    expect(
      edit.apply([{ type: "add_layer", id: "b" }, { type: "invalid" }]).ok,
    ).toBe(false);
    expect(PM.proj.layers.map((x: any) => x.id)).toEqual(["a"]);
    expect(edit.cancel()).toBe(true);
    expect(PM.proj.layers).toEqual([]);
    expect(edit.cancel()).toBe(false);
  });
  test("all three EditGesture modes preserve discarded and once returns", () => {
    const { realm, PM } = setup();
    const set = new EditGesture(realm, {
      mode: "set",
      label: "set",
      set: (value) => {
        PM.proj.name = value;
        return value;
      },
    });
    expect(set.begin()).toBeUndefined();
    expect(set.write("kept")).toBeUndefined();
    expect(set.cancel()).toBeUndefined();
    expect(PM.proj.name).toBe("kept");
    expect(set.once("once")).toBe("once");
    const command = new EditGesture(realm, {
      mode: "command",
      label: "drag",
      command: { type: "add_layer", id: "a" },
    });
    command.begin();
    command.write(1);
    command.cancel();
    expect(PM.proj.layers).toEqual([]);
    command.begin();
    command.write(1);
    command.commit();
    expect(PM.proj.layers.length).toBe(1);
    let local: any;
    const localGesture = new EditGesture(realm, {
      mode: "local",
      label: "local",
      set: (v) => {
        local = v;
      },
    });
    localGesture.begin();
    localGesture.write(2);
    localGesture.cancel();
    expect(localGesture.once(3)).toBeUndefined();
    expect(local).toBe(3);
  });
  test("workspace partial writes survive callback error with exact error identity", () => {
    const { realm, host, PM } = setup(),
      error = {};
    try {
      realm.workspace.mutate(
        (w) => {
          w.name = "partial";
          throw error;
        },
        { inPlace: true },
      );
    } catch (caught) {
      expect(caught).toBe(error);
    }
    expect(PM.WS.current.name).toBe("partial");
    expect(host.state.workspace.name).toBe("partial");
    const current = realm.workspace.current();
    current.name = "detached";
    expect(PM.WS.current.name).toBe("partial");
  });
});
describe("live mirror and transaction boundary", () => {
  test("raw aliases, defineProperty, array changes and root replacement are captured", () => {
    const { realm, PM, host } = setup();
    const raw: Data = { id: "a", nested: { value: 1 } };
    PM.proj.layers.push(raw);
    realm.flush();
    raw.nested.value = 2;
    Object.defineProperty(raw, "extra", {
      enumerable: true,
      configurable: true,
      value: true,
    });
    realm.flush();
    expect(host.state.document.layers[0]).toEqual({
      id: "a",
      nested: { value: 2 },
      extra: true,
    });
    delete raw.extra;
    realm.flush();
    expect(host.state.document.layers[0].extra).toBeUndefined();
    const next = { ...PM.proj, name: "replacement" };
    PM.proj = next;
    expect(PM.proj).toBe(next);
    host.drain();
    expect(host.state.document.name).toBe("replacement");
  });
  test("native publication merges disjoint paths and local last writer wins", () => {
    const { realm, PM, host } = setup(),
      held = PM.proj;
    PM.proj.name = "local";
    host.state.document.name = "foreign";
    host.state.document.extra = "foreign";
    host.state.revision++;
    realm.receive(host.publication());
    expect(PM.proj).toBe(held);
    realm.flush();
    expect(host.state.document.name).toBe("local");
    expect(host.state.document.extra).toBe("foreign");
  });
  test("retained objects follow ids across reorder; deleted references do not retarget", () => {
    const mirror = new DocumentMirror({
      layers: [
        { id: "a", p: { v: 1 } },
        { id: "b", p: { v: 2 } },
      ],
    });
    const a = mirror.document.layers[0],
      prop = a.p;
    mirror.accept({
      layers: [
        { id: "b", p: { v: 3 } },
        { id: "a", p: { v: 4 } },
      ],
    });
    expect(mirror.document.layers[1]).toBe(a);
    expect(a.p).toBe(prop);
    expect(prop.v).toBe(4);
    mirror.accept({ layers: [{ id: "b", p: { v: 3 } }] });
    a.p.v = 9;
    expect(mirror.pending()).toEqual([]);
  });
  test("rejection resyncs and throws, without inventing success or undo", () => {
    const { realm, PM, host } = setup();
    const held = PM.proj;
    PM.proj.name = "rejected";
    host.rejectNextWrite = true;
    expect(() => realm.flush()).toThrow("Synthetic validation rejection");
    expect(PM.proj).toBe(held);
    expect(PM.proj.name).toBe("Synthetic trace fixture");
    expect(PM.hist.canUndo()).toBe(false);
  });
  test("lost acknowledgement reuses txid exactly once and preserves later local writes", () => {
    const { realm, PM, host } = setup();
    PM.proj.name = "acknowledged";
    host.failAfterApply = true;
    expect(() => realm.flush()).toThrow("lost acknowledgement");
    expect(host.state.document.name).toBe("acknowledged");
    expect(realm.channel.pending).toBe(true);
    PM.proj.name = "later";
    realm.flush();
    expect(host.state.document.name).toBe("later");
    expect(
      host.requests.filter((r) => r.method === "document.directWrite"),
    ).toHaveLength(2);
    expect(realm.channel.pending).toBe(false);
  });
  test.each([undefined, NaN, Infinity, 1n, () => 1])(
    "lossy value %p is rejected and resynced",
    (value) => {
      const { realm, PM, host } = setup();
      PM.proj.bad = value;
      expect(() => realm.flush()).toThrow();
      expect(PM.proj.bad).toBeUndefined();
      expect(host.requests).toHaveLength(0);
    },
  );
  test("cycles, accessors and prototype keys reject without running getters", () => {
    const { realm, PM } = setup();
    PM.proj.cycle = PM.proj;
    expect(() => realm.flush()).toThrow();
    let read = false;
    Object.defineProperty(PM.proj, "getter", {
      enumerable: true,
      configurable: true,
      get: () => {
        read = true;
        return 1;
      },
    });
    expect(() => realm.flush()).toThrow();
    expect(read).toBe(false);
    expect(() => copy(JSON.parse('{"__proto__":{"x":1}}'))).toThrow();
  });
  test("task boundary flush and attributed failure are inspectable", () => {
    const { realm, PM, host } = setup();
    host.rejectNextWrite = true;
    expect(
      realm.task(() => {
        PM.proj.name = "bad";
        return 7;
      }),
    ).toBe(7);
    host.drain();
    expect(host.errors).toHaveLength(1);
    expect(PM.proj.name).toBe("Synthetic trace fixture");
  });
});
describe("realm identity and ownership", () => {
  test("sync services retain identity across replacements and out-of-order disposal", () => {
    const { realm } = setup(),
      a = realm.extension("a", 1),
      b = realm.extension("b", 3);
    const base = { call: (n: number) => n + 1 },
      replacement = {};
    const d = a.api.services.register("timeline", base);
    expect(b.api.services.get("timeline")).toBe(base);
    expect(b.api.services.get("timeline").call(2)).toBe(3);
    const top = b.api.services.register("timeline", replacement);
    const held = base;
    d.dispose();
    expect(a.api.services.get("timeline")).toBe(replacement);
    expect(held).toBe(base);
    top.dispose();
    expect(a.api.services.get("timeline")).toBeNull();
    a.api.services.register("timeline", base);
    b.api.services.register("timeline", replacement);
    b.dispose();
    expect(a.api.services.get("timeline")).toBe(base);
    a.dispose();
    expect(realm.services.get("timeline")).toBeNull();
  });
  test("external closure pins survive extension disposal; throwing undo moves cursor", () => {
    const { realm, history, host, PM } = setup();
    const extension = realm.extension("a");
    let value = 1,
      cleanup = 0;
    history.external(
      "external",
      () => {
        value = 0;
      },
      () => {
        value = 1;
      },
      { cleanup: () => cleanup++ },
    );
    expect(host.pins.size).toBe(1);
    extension.dispose();
    expect(history.undo()).toBe(true);
    expect(value).toBe(0);
    expect(history.redo()).toBe(true);
    expect(value).toBe(1);
    realm.retireExternal([...host.pins][0]);
    expect(cleanup).toBe(1);
    expect(host.pins.size).toBe(0);
    const error = {};
    history.external(
      "throws",
      () => {
        throw error;
      },
      () => {},
    );
    try {
      history.undo();
    } catch (caught) {
      expect(caught).toBe(error);
    }
    expect(PM.hist.canRedo()).toBe(true);
  });
  test("external undo can synchronously call another history operation", () => {
    const { history, PM } = setup();
    history.external(
      "reentry",
      () => {
        history.do("nested", () => {
          PM.proj.name = "nested";
        });
      },
      () => {},
    );
    expect(history.undo()).toBe(true);
    expect(PM.proj.name).toBe("nested");
  });
  test("dispose seals acknowledged edits; stale realm API refuses work", () => {
    const { realm, history, PM, host } = setup();
    history.begin("open");
    PM.proj.name = "kept";
    realm.dispose();
    expect(host.state.document.name).toBe("kept");
    expect(host.entries).toHaveLength(1);
    expect(() => history.begin("late")).toThrow("disposed");
  });
  test("temporary globals restore prior descriptors", () => {
    const { realm } = setup();
    const target = { PM: "prior" };
    const restore = realm.installGlobals(target);
    expect(target.PM as unknown).toBe(realm.PM);
    restore();
    expect(target.PM).toBe("prior");
  });
});
test("retained layer and keyframe writes rebase by identity over native reorder", () => {
  const original = {
    layers: [
      {
        id: "a",
        p: {
          kf: [
            { i: "one", v: 1 },
            { i: "two", v: 2 },
          ],
        },
      },
      { id: "b", p: {} },
    ],
  };
  const mirror = new DocumentMirror(original),
    layer = mirror.document.layers[0],
    key = layer.p.kf[0];
  key.v = 42;
  mirror.receive({
    layers: [
      { id: "b", p: {} },
      {
        id: "a",
        p: {
          kf: [
            { i: "two", v: 2 },
            { i: "one", v: 3 },
          ],
        },
      },
    ],
  });
  expect(mirror.document.layers[1]).toBe(layer);
  expect(layer.p.kf[1]).toBe(key);
  expect(key.v).toBe(42);
  expect(mirror.pending()).toEqual([
    { path: ["layers", 1, "p", "kf", 1, "v"], exists: true, value: 42 },
  ]);
});
test("pending retained write to a deleted record fails with authoritative resync", () => {
  const mirror = new DocumentMirror({
    layers: [
      { id: "a", name: "a" },
      { id: "b", name: "b" },
    ],
  });
  mirror.document.layers[0].name = "local";
  expect(() =>
    mirror.receive({ layers: [{ id: "b", name: "foreign" }] }),
  ).toThrow("no longer exists");
  expect(mirror.document.layers).toEqual([{ id: "b", name: "foreign" }]);
});
test("structural callback lease permits reentrant synchronous host edits and keeps exact result", () => {
  const { realm, PM, host } = setup(),
    sentinel = {};
  const result = realm.edit.mutate("outer structural", () => {
    PM.proj.layers.push({ id: "raw", name: "Raw", type: "shape" });
    expect(
      realm.edit.apply({ type: "add_layer", id: "typed", layerType: "shape" })
        .ok,
    ).toBe(true);
    return sentinel;
  });
  expect(result.ok).toBe(true);
  expect(result.data!.result).toBe(sentinel);
  expect(PM.proj.layers.map((v: Data) => v.id)).toEqual(["raw", "typed"]);
  expect(host.entries).toHaveLength(1);
  expect(realm.history.undo()).toBe(true);
  expect(PM.proj.layers).toEqual([]);
});
test("throwing structural callback aborts a lease after reentrant writes", () => {
  const { realm, PM } = setup();
  const result = realm.edit.mutate("throwing", () => {
    realm.edit.apply({ type: "add_layer", id: "typed" });
    throw new Error("synthetic mutation failure");
  });
  expect(result).toEqual({ ok: false, message: "synthetic mutation failure" });
  expect(PM.proj.layers).toEqual([]);
  expect(PM.hist.busy()).toBe(false);
});
test("raster override and nested capture callbacks remain inline in the shared realm", () => {
  const host = new FakeHost(),
    order: string[] = [];
  let realm: CompatRealm;
  host.query = (method, args) => {
    order.push(method);
    if (method === "render.raster")
      return { native: true, layer: args.args[0] };
    if (method === "render.snapshot") return "data:image/jpeg;base64,synthetic";
    if (method === "render.renderFrameTo")
      return realm.PM.raster("frame-layer");
    throw new Error("Unavailable");
  };
  realm = new CompatRealm({
    id: "raster",
    generation: 1,
    host,
    initial: host.publication(),
  });
  const provider = realm.extension("override"),
    consumer = realm.extension("consumer");
  const base = provider.api.services.get("raster");
  const registration = provider.api.services.register(
    "raster",
    (layer: string) => {
      order.push("override");
      expect(consumer.api.render.snapshot()).toStartWith("data:image/jpeg");
      return base(layer);
    },
  );
  expect(consumer.api.render.renderFrameTo()).toEqual({
    native: true,
    layer: "frame-layer",
  });
  expect(order).toEqual([
    "render.renderFrameTo",
    "override",
    "render.snapshot",
    "render.raster",
  ]);
  registration.dispose();
  expect(provider.api.services.get("raster")).toBe(base);
});
test("simultaneous raw selection and document writes survive each acknowledgement", () => {
  const { realm, PM, host } = setup();
  PM.proj.name = "document";
  PM.sel.layers.push("layer");
  realm.flush();
  expect(host.state.document.name).toBe("document");
  expect(host.state.selection.layers).toEqual(["layer"]);
  expect(PM.sel.layers).toEqual(["layer"]);
});
test("workspace callback retains its draft through a reentrant host publication", () => {
  const { realm, PM, host } = setup();
  realm.workspace.mutate((w) => {
    w.name = "draft";
    PM.touch();
    w.extra = true;
  });
  expect(host.state.workspace).toMatchObject({ name: "draft", extra: true });
});
test("invalid raw write plus thrown history callback preserves original error and closes entry", () => {
  const { history, PM, host } = setup(),
    original = {};
  let caught: unknown;
  try {
    history.do("bad JSON", () => {
      PM.proj.bad = NaN;
      throw original;
    });
  } catch (error) {
    caught = error;
  }
  expect(caught).toBe(original);
  expect(PM.hist.busy()).toBe(false);
  expect(host.errors).toHaveLength(1);
});
test("unsolicited native publication preserves pending raw selection writes", () => {
  const { realm, PM, host } = setup();
  PM.sel.layers.push("pending");
  host.state.document.name = "foreign";
  host.state.revision++;
  realm.receive(host.publication());
  realm.flush();
  expect(host.state.selection.layers).toEqual(["pending"]);
  expect(PM.proj.name).toBe("foreign");
});
test("native namespace backends retain extra methods and accessors beside mirror adapters", () => {
  const host = new FakeHost(),
    marker = {};
  const model = {
    get custom() {
      return marker;
    },
    nativeMethod: () => 42,
  };
  const realm = new CompatRealm({
    id: "backends",
    generation: 1,
    host,
    initial: host.publication(),
    capabilities: { model, render: { gl: marker } },
  });
  const api = realm.extension("x").api;
  expect(api.model.custom).toBe(marker);
  expect(api.model.nativeMethod()).toBe(42);
  expect(api.render.gl).toBe(marker);
  expect(api.model.layer("missing")).toBeNull();
});
test("guarded contribution callbacks attribute synchronous and asynchronous errors", async () => {
  const { realm, host } = setup(),
    extension = realm.extension("guarded"),
    error = {};
  extension.api.commands.register({
    id: "throws",
    run: () => {
      throw error;
    },
  });
  expect(extension.api.commands.run("throws")).toBeUndefined();
  expect(host.errors[0]).toEqual({
    extensionId: "guarded",
    label: "command throws",
    error,
  });
  extension.api.commands.register({
    id: "rejects",
    run: async () => {
      throw error;
    },
  });
  expect(await extension.api.commands.run("rejects")).toBeUndefined();
  expect(host.errors).toHaveLength(2);
  let called = 0;
  extension.api.events.on("event", () => {
    called++;
    throw error;
  });
  extension.api.events.emit("event");
  expect(called).toBe(1);
  expect(host.errors).toHaveLength(3);
  extension.dispose();
  realm.PM.bus.emit("event");
  expect(called).toBe(1);
});
test("disposed extension history callbacks can still read the live services registry", () => {
  const { realm } = setup(),
    extension = realm.extension("history-owner"),
    provider = {};
  realm.services.register("retained", provider);
  let observed: unknown;
  extension.api.history.external(
    "kept",
    () => {
      observed = extension.api.services.get("retained");
    },
    () => {},
  );
  extension.dispose();
  realm.history.undo();
  expect(observed).toBe(provider);
});
test("owned onDispose handlers run once without reporting retired-owner health failures (v1.2.0)", () => {
  const { realm, host } = setup(),
    extension = realm.extension("lifecycle"),
    error = {};
  let called = 0;
  expect(
    extension.api.onDispose(() => {
      called++;
      throw error;
    }),
  ).toBeUndefined();
  extension.dispose();
  extension.dispose();
  expect(called).toBe(1);
  expect(host.errors).toEqual([]);
});
describe("writes cost what they touch", () => {
  const big = (layers: number, keys: number) => ({
    name: "big",
    layers: Array.from({ length: layers }, (_, l) => ({
      id: `l${l}`,
      p: {
        x: {
          v: 0,
          kf: Array.from({ length: keys }, (_, k) => ({
            i: `k${k}`,
            t: k,
            v: k,
          })),
        },
      },
    })),
  });
  test("a deep write sends only its path and settles only there", () => {
    const mirror = new DocumentMirror(big(3, 4));
    const key = mirror.document.layers[2].p.x.kf[1];
    key.v = 99;
    expect(mirror.take()).toEqual([
      { path: ["layers", 2, "p", "x", "kf", 1, "v"], exists: true, value: 99 },
    ]);
    expect(mirror.pending()).toEqual([]);
    expect(mirror.document.layers[2].p.x.kf[1]).toBe(key);
  });
  test("a record an extension built keeps taking its later raw writes", () => {
    const mirror = new DocumentMirror(big(1, 1));
    const raw: Data = { i: "new", t: 5, v: 1, ease: { speed: 0 } };
    mirror.document.layers[0].p.x.kf.push(raw);
    expect(mirror.take()).toEqual([
      {
        path: ["layers", 0, "p", "x", "kf"],
        exists: true,
        value: [
          { i: "k0", t: 0, v: 0 },
          { i: "new", t: 5, v: 1, ease: { speed: 0 } },
        ],
      },
    ]);
    mirror.acknowledged.layers[0].p.x.kf.push(copy(raw));
    raw.ease.speed = 3;
    expect(mirror.take()).toEqual([
      {
        path: ["layers", 0, "p", "x", "kf", 1, "ease", "speed"],
        exists: true,
        value: 3,
      },
    ]);
  });
  test("a record an extension inserts is found again by identity", () => {
    const mirror = new DocumentMirror(big(1, 1));
    const doc = mirror.document;
    const mine: Data = { id: "mine", p: { x: { v: 1 } } };
    doc.layers.push(mine);
    expect(doc.layers.indexOf(mine)).toBe(1);
    expect(doc.layers.includes(mine)).toBe(true);
    expect(doc.layers.find((l: Data) => l === mine)).toBe(mine);
    expect(doc.layers[1].p).toBe(mine.p);
    // A record that came from the project keeps one identity too.
    const theirs = doc.layers[0];
    expect(doc.layers.indexOf(theirs)).toBe(0);
    expect(mirror.take()).toHaveLength(1);
  });
  test("project records structured-clone as plain data", () => {
    const mirror = new DocumentMirror(big(2, 2));
    const clone = cloneAcceptingViews(structuredClone);
    const layer = mirror.document.layers[0];
    expect(() => structuredClone(layer)).toThrow();
    const copied = clone(layer);
    expect(copied).toEqual({
      id: "l0",
      p: {
        x: {
          v: 0,
          kf: [
            { i: "k0", t: 0, v: 0 },
            { i: "k1", t: 1, v: 1 },
          ],
        },
      },
    });
    copied.id = "copy";
    expect(mirror.document.layers[0].id).toBe("l0");
    // Views nested in the extension's own containers, and cycles.
    const bag: Data = { layers: [layer], keyed: new Map([["a", layer.p]]) };
    bag.self = bag;
    const out = clone(bag);
    expect(out.layers[0].id).toBe("l0");
    expect(out.keyed.get("a").x.v).toBe(0);
    expect(out.self).toBe(out);
    expect(unwrapViews(mirror.document.layers)).toBe(
      unwrapViews(mirror.document.layers),
    );
    expect(mirror.take()).toEqual([]);
  });
  test("a thousand one-key boundaries on 200k keyframes stay cheap", () => {
    const mirror = new DocumentMirror(big(400, 500));
    const started = performance.now();
    for (let i = 0; i < 1000; i++) {
      mirror.document.layers[i % 400].p.x.kf[i % 500].v = -(i + 1);
      const ops = mirror.take();
      expect(ops).toHaveLength(1);
      // The native adapter answers with the stored change's operations.
      mirror.applyOps(ops);
      mirror.settle(ops);
    }
    // One whole-tree walk per boundary takes tens of milliseconds here.
    expect(performance.now() - started).toBeLessThan(3000);
    expect(mirror.acknowledged.layers[999 % 400].p.x.kf[999 % 500].v).toBe(
      -1000,
    );
    expect(mirror.pending()).toEqual([]);
  });
});
