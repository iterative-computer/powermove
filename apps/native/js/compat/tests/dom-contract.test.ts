import { expect, test } from "bun:test";
import { CompatRealm } from "../src";
import type { RemoteDOMBridge, DOMBatch } from "../src/remote-dom";
import { FakeHost } from "./fake-host";
test("DOM bridge receives realm/generation, shared canvas factory and owned task boundary", () => {
  const host = new FakeHost();
  let disconnected = false;
  let context: any;
  const document = { createElement: () => ({ id: "remote-node" }) };
  const bridge: RemoteDOMBridge = {
    connect(value) {
      context = value;
      return {
        globals: { document },
        dispose: () => {
          disconnected = true;
        },
      };
    },
    submit(_batch: DOMBatch) {},
    read() {
      throw new Error("layout not acknowledged");
    },
    invoke() {},
    subscribe() {
      return { dispose() {} };
    },
    resolveDefault() {},
  };
  const realm = new CompatRealm({
    id: "dom",
    generation: 2,
    initial: host.publication(),
    host,
    dom: bridge,
  });
  expect(realm.globals.document).toBe(document);
  expect(context.realm).toBe("dom");
  expect(context.generation).toBe(2);
  context.task(() => {
    realm.PM.proj.name = "remote event";
  });
  host.drain();
  expect(host.state.document.name).toBe("remote event");
  expect(() => bridge.read("remote-node", "clientWidth", 5)).toThrow(
    "not acknowledged",
  );
  realm.dispose();
  expect(disconnected).toBe(true);
});
