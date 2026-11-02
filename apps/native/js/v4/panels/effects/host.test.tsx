import { expect, test } from "bun:test";
import { HostClient, type HostRequest, type HostAck } from "../../src/host-api";
import type { Json } from "../../src/protocol";
import { EffectsHost, type EffectsHostAdapter } from "./host";
import { createEffectsExtension, EffectsMountError } from "./index";
import { EffectsModel, workerSearch } from "./model";
import { OwnerHost } from "../../eval/host-fixture";
import { deferred, fixture, items } from "./fixtures";

const identity = {
  realm: "synthetic-effects",
  generation: 1,
  project: "synthetic-project",
};
const ack = (r: HostRequest, value?: Json): HostAck => ({
  ...identity,
  version: 1,
  requestId: r.requestId,
  revision: r.expectedRevision,
  accepted: true,
  ...(r.operation.type.startsWith("settings.")
    ? { stateRevision: r.expectedStateRevision ?? 0 }
    : {}),
  ...(value === undefined ? {} : { value }),
});
function setup(transport?: (r: HostRequest) => Promise<HostAck>) {
  const requests: HostRequest[] = [],
    errors: unknown[] = [];
  let publishRegistry: EffectsHostAdapter["registry"]["subscribe"] extends (
    f: infer F,
  ) => unknown
    ? F
    : never;
  let stops = 0;
  const adapter: EffectsHostAdapter = {
    registry: {
      subscribe: (publish) => {
        publishRegistry = publish;
        publish({ items, groups: [] });
        return () => {
          stops++;
        };
      },
    },
    target: (project, selection) =>
      Array.isArray(selection) &&
      typeof selection[0] === "string" &&
      project !== null
        ? { id: selection[0], effects: ["blur"] }
        : null,
    add: (item, target) => [
      {
        command: "fixture.edit",
        args: { effect: item.id, target: target.id, kind: item.kind },
      },
    ],
    reveal: (item, target) => ({
      command: "fixture.reveal",
      args: { effect: item.id, target: target.id },
    }),
  };
  const client = new HostClient(identity, async (r) => {
    requests.push(r);
    return transport
      ? transport(r)
      : ack(
          r,
          r.operation.type === "subscribe"
            ? r.operation.query === "selection"
              ? ["layer"]
              : {}
            : [],
        );
  });
  const host = new EffectsHost(client, adapter);
  const model = new EffectsModel({
    apply: (item, target) => host.apply(item, target),
    reveal: (item, target) => host.reveal(item, target),
    persist: (key, value) => host.persist(key, value),
    search: workerSearch,
    report: (error) => errors.push(error),
  });
  return {
    client,
    host,
    model,
    requests,
    errors,
    adapter,
    registry: (registry: Parameters<typeof publishRegistry>[0]) =>
      publishRegistry(registry),
    stops: () => stops,
  };
}
test("connect uses real host query/settings envelopes, live registry updates and reverse cleanup", async () => {
  const f = setup();
  await f.host.connect(f.model, "fx:1");
  expect(f.requests.map((r) => r.operation.type)).toEqual([
    "settings.get",
    "settings.get",
    "subscribe",
    "subscribe",
  ]);
  expect(f.model.target).toEqual({ id: "layer", effects: ["blur"] });
  f.registry({ items: [...items, { ...items[0], id: "late" }], groups: [] });
  expect(f.model.installedCount()).toBe(3);
  await f.host.dispose();
  expect(f.stops()).toBe(1);
  expect(
    f.requests
      .filter((r) => r.operation.type === "unsubscribe")
      .map((r) => r.operation),
  ).toEqual([
    { type: "unsubscribe", subscription: "fx:1:selection" },
    { type: "unsubscribe", subscription: "fx:1:project" },
  ]);
});
test("surface model edits/reveal/persistence route to HostClient without invented commands", async () => {
  const f = setup();
  await f.host.connect(f.model, "fx:1");
  expect(await f.model.apply("blur")).toBe(true);
  await f.model.settingsSettled();
  expect(
    f.requests.find((r) => r.operation.type === "edit")?.operation,
  ).toEqual({
    type: "edit",
    label: "Add Gaussian Blur",
    commands: [
      {
        command: "fixture.edit",
        args: { effect: "blur", target: "layer", kind: "effect" },
      },
    ],
  });
  expect(
    f.requests.find((r) => r.operation.type === "settings.set")?.operation,
  ).toEqual({ type: "settings.set", key: "pm.fx.recent", value: ["blur"] });
  await f.model.contextAction("blur", "reveal");
  expect(f.requests.at(-1)?.operation).toEqual({
    type: "command",
    command: "fixture.reveal",
    args: { effect: "blur", target: "layer" },
  });
  f.model.dispose();
  await f.host.dispose();
});
test("native service routes panel requests and rejected edits through actual Rust router/data service", async () => {
  const native = new OwnerHost();
  const adapter = setup().adapter;
  const host = new EffectsHost(native.client, adapter);
  const model = new EffectsModel({
    apply: (i, t) => host.apply(i, t),
    reveal: (i, t) => host.reveal(i, t),
    persist: (key, value) => host.persist(key, value),
    search: workerSearch,
    report: () => {},
  });
  try {
    native.fixture("project", {});
    native.fixture("selection", ["layer"]);
    await host.connect(model, "fx:native");
    native.rejectNextEdit();
    expect(await model.apply("blur")).toBe(false);
    expect(model.recent).toEqual([]);
    expect(await model.apply("blur")).toBe(true);
    await model.settingsSettled();
    expect((await native.client.getSetting("pm.fx.recent")).value).toEqual([
      "blur",
    ]);
    native.fixture("selection", []);
    expect(native.publish()).toEqual([true, true]);
    expect(model.target).toBeNull();
  } finally {
    model.dispose();
    await host.dispose();
    await native.retire();
  }
});
test("settings hydration round-trips the legacy recent key without rewriting unknown source IDs", async () => {
  const f = setup(async (r) =>
    ack(
      r,
      r.operation.type === "settings.get" && r.operation.key === "pm.fx.recent"
        ? '["unloaded","glow"]'
        : [],
    ),
  );
  await f.host.connect(f.model, "fx:1");
  expect(f.model.recent).toEqual(["unloaded", "glow"]);
  expect(f.requests.some((r) => r.operation.type === "settings.set")).toBe(
    false,
  );
  await f.host.dispose();
});
test("lifecycle cleanup runs after a subscription rejection; no active registry remains", async () => {
  const f = setup(async (r) =>
    r.operation.type === "subscribe" && r.operation.query === "selection"
      ? { ...ack(r), accepted: false, error: "denied" }
      : ack(r, {}),
  );
  await expect(f.host.connect(f.model, "fx:1")).rejects.toThrow("denied");
  expect(f.stops()).toBe(1);
  expect(f.model.target).toBeNull();
  expect(f.requests.at(-1)?.operation.type).toBe("unsubscribe");
});
test("retiring during pending registration releases a late acknowledged subscription", async () => {
  const pending = deferred<HostAck>(),
    reached = deferred<void>();
  let request!: HostRequest;
  const f = setup(async (r) => {
    if (r.operation.type === "subscribe") {
      request = r;
      reached.resolve();
      return pending.promise;
    }
    return ack(r, []);
  });
  const connecting = f.host.connect(f.model, "fx:1");
  await reached.promise;
  expect(request.operation.type).toBe("subscribe");
  await f.host.dispose();
  pending.resolve(ack(request, {}));
  await connecting;
  expect(f.requests.at(-1)?.operation).toEqual({
    type: "unsubscribe",
    subscription: "fx:1:project",
  });
  expect(
    f.requests.filter((r) => r.operation.type === "subscribe"),
  ).toHaveLength(1);
});
test("failed unsubscribe stays retryable and remaining cleanup is still attempted", async () => {
  let reject = true;
  const f = setup(async (r) => {
    if (
      r.operation.type === "unsubscribe" &&
      r.operation.subscription.endsWith(":selection") &&
      reject
    ) {
      reject = false;
      return { ...ack(r), accepted: false, error: "retry" };
    }
    return ack(r, []);
  });
  await f.host.connect(f.model, "fx:1");
  await expect(f.host.dispose()).rejects.toThrow("did not retire");
  await f.host.dispose();
  expect(f.stops()).toBe(1);
  expect(
    f.requests.filter((r) => r.operation.type === "unsubscribe"),
  ).toHaveLength(3);
});

test("a rejected late subscription detach stays available for retirement retry", async () => {
  const pending = deferred<HostAck>(),
    reached = deferred<void>();
  let request!: HostRequest,
    rejectDetach = true;
  const f = setup(async (r) => {
    if (r.operation.type === "subscribe") {
      request = r;
      reached.resolve();
      return pending.promise;
    }
    if (r.operation.type === "unsubscribe" && rejectDetach)
      return { ...ack(r), accepted: false, error: "detach unavailable" };
    return ack(r, []);
  });
  const connecting = f.host.connect(f.model, "fx:late");
  await reached.promise;
  await f.host.dispose();
  pending.resolve(ack(request, {}));
  await expect(connecting).rejects.toThrow("connection and cleanup failed");
  rejectDetach = false;
  await f.host.dispose();
  expect(
    f.requests.filter((r) => r.operation.type === "unsubscribe"),
  ).toHaveLength(3);
});
test("extension entry mounts a six-op surface, exposes headless IDs, and retires both lifecycles", async () => {
  const f = setup();
  const commits: unknown[] = [];
  const extension = await createEffectsExtension({
    host: f.client,
    adapter: f.adapter,
    bindings: { search: "local-search", scope: "local-scope" },
    subscriptionPrefix: "fx:entry",
    viewport: { start: 0, size: 8 },
    transport: async (commit) => {
      commits.push(commit);
      return { accepted: true, revision: commit.baseRevision + 1 };
    },
    onError: (error) => {
      throw error;
    },
  });
  expect(extension.inspect().panel).toBe("fxbrowser");
  expect(extension.inspect().title).toBe("Effects");
  expect(
    extension.root
      .inspect()
      .nodes.some((node) => node.props.id === "fxbrowser:search"),
  ).toBe(true);
  expect(commits.length).toBeGreaterThan(0);
  await extension.dispose();
  expect(f.stops()).toBe(1);
});
test("rejected surface mount cleans host subscriptions instead of leaving a hidden panel", async () => {
  const f = setup();
  await expect(
    createEffectsExtension({
      host: f.client,
      adapter: f.adapter,
      bindings: { search: "local-search", scope: "local-scope" },
      subscriptionPrefix: "fx:entry",
      viewport: { start: 0, size: 8 },
      transport: async () => ({ accepted: false, revision: 0 }),
      onError: () => {},
    }),
  ).rejects.toThrow("rejected");
  expect(f.stops()).toBe(1);
  expect(
    f.requests.filter((r) => r.operation.type === "unsubscribe"),
  ).toHaveLength(2);
});

test("mount failure preserves cleanup failures and exposes an explicit retry", async () => {
  let rejectCleanup = true;
  const f = setup(async (r) => {
    if (r.operation.type === "unsubscribe" && rejectCleanup)
      return { ...ack(r), accepted: false, error: "temporary cleanup refusal" };
    return ack(r, []);
  });
  let failure: unknown;
  try {
    await createEffectsExtension({
      host: f.client,
      adapter: f.adapter,
      bindings: { search: "local-search", scope: "local-scope" },
      subscriptionPrefix: "fx:entry",
      viewport: { start: 0, size: 8 },
      transport: async () => ({ accepted: false, revision: 0 }),
      onError: () => {},
    });
  } catch (error) {
    failure = error;
  }
  expect(failure).toBeInstanceOf(EffectsMountError);
  rejectCleanup = false;
  await (failure as EffectsMountError).retryCleanup();
  expect(f.stops()).toBe(1);
  expect(
    f.requests.filter((r) => r.operation.type === "unsubscribe"),
  ).toHaveLength(4);
});
