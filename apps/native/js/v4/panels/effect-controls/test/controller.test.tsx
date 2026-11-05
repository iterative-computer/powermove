import { expect, test } from "bun:test";
import { fileURLToPath } from "node:url";
import {
  HostClient,
  type HostAck,
  type HostRequest,
  type Json,
  type Commit,
} from "../../../src";
import { EffectsController } from "../controller";
import { mountEffectControls } from "../extension";
import { selected } from "./fixtures";
import { platform, fixture, wire } from "./helpers";
import type { Intent } from "../contract";

function transport(value = selected()) {
  const requests: HostRequest[] = [];
  const client = new HostClient(
    { realm: "effects", generation: 1, project: "P" },
    async (r) => {
      requests.push(r);
      return {
        ...r,
        accepted: true,
        revision: value.revision,
        value: value as unknown as Json,
      };
    },
  );
  return { requests, client };
}
test("parameter bindings are acknowledged before publishing the first editable view and retired on disposal", async () => {
  const f = await fixture(selected());
  expect(f.requests).toEqual([
    {
      type: "subscribe",
      subscription: "effects",
      query: "evaluated",
      channels: ["panel:layer-effects"],
    },
    {
      type: "binding.register",
      handle: "radius",
      channels: ['{"layer":"a","path":"f1.radius"}'],
    },
  ]);
  await f.controller.dispose();
  expect(f.requests.slice(-2)).toEqual([
    { type: "unsubscribe", subscription: "effects" },
    { type: "binding.dispose", handle: "radius" },
  ]);
  await expect(f.controller.drive({ type: "clear" })).rejects.toThrow(
    "disposed",
  );
  await f.root.dispose();
});
test("registration failure never publishes an editable field with an unregistered binding", async () => {
  const p = selected(),
    kit = platform();
  const host = new HostClient(
    { realm: "effects", generation: 1, project: "P" },
    async (r) => ({
      ...r,
      revision: 0,
      accepted: r.operation.type !== "binding.register",
      error: "binding refused",
      value: p as unknown as Json,
    }),
  );
  const c = new EffectsController(host, kit.api, "effects");
  await c.start();
  expect(c.snapshot().target).toBe("NoProject");
  expect(kit.errors).toHaveLength(1);
  await c.dispose();
});
test("missing model ack projection is an error and cannot erase the accepted panel", async () => {
  const p = selected(),
    kit = platform();
  const host = new HostClient(
    { realm: "effects", generation: 1, project: "P" },
    async (r) => ({
      ...r,
      revision: 0,
      accepted: true,
      ...(r.operation.type !== "command"
        ? { value: p as unknown as Json }
        : {}),
    }),
  );
  const c = new EffectsController(host, kit.api, "effects");
  await c.start();
  await expect(c.drive({ type: "remove", id: "f1" })).rejects.toThrow(
    "Missing acknowledged model projection",
  );
  expect(c.snapshot()).toEqual(p);
  expect(kit.announcements).toEqual([]);
  await c.dispose();
});
test("queued intent is copied at submission and is fenced on a target change", async () => {
  const p = selected(),
    kit = platform();
  const operations: any[] = [];
  let finish!: (ack: HostAck) => void;
  let pending!: HostRequest;
  const host = new HostClient(
    { realm: "effects", generation: 1, project: "P" },
    async (r) => {
      operations.push(r.operation);
      if (r.operation.type === "command") {
        pending = r;
        return new Promise<HostAck>((resolve) => {
          finish = resolve;
        });
      }
      return { ...r, accepted: true, revision: 0, value: p as unknown as Json };
    },
  );
  const c = new EffectsController(host, kit.api, "effects");
  await c.start();
  const intent: Intent = { type: "remove", id: "f1" };
  const job = c.drive(intent);
  intent.id = "f2";
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  expect(operations.at(-1)).toMatchObject({
    args: { documentId: p.documentId, intent: { type: "remove", id: "f1" } },
  });
  finish({
    ...pending,
    accepted: true,
    revision: 0,
    value: p as unknown as Json,
  });
  await job;
  await c.dispose();
});
test("menu results never retarget after a layer change", async () => {
  const p = selected(),
    { client } = transport(p),
    kit = platform();
  let choose!: (definition: string) => void;
  kit.api.effectMenu = () =>
    new Promise((resolve) => {
      choose = resolve;
    });
  const c = new EffectsController(client, kit.api, "effects");
  await c.start();
  const opening = c.add();
  const changed = { ...p, layer: "b", documentId: 2 };
  client.publish({
    ...client.identity,
    version: 1,
    subscription: "effects",
    revision: 0,
    sequence: 1,
    value: changed as unknown as Json,
  });
  await c.settled();
  choose("glow");
  await opening;
  expect(c.snapshot().layer).toBe("b");
  expect(kit.announcements).toEqual([]);
  await c.dispose();
});
test("v4 extension mounts and disposes an acknowledged surface with the standard lifecycle", async () => {
  const { client, requests } = transport(),
    kit = platform();
  const commits: Commit[] = [];
  const panel = await mountEffectControls({
    host: client,
    platform: kit.api,
    subscriptionId: "mount-1",
    rowHeight: 24,
    transport: async (commit) => {
      commits.push(commit);
      return { accepted: true, revision: commit.baseRevision + 1 };
    },
  });
  expect(
    panel.root.inspect().nodes.some((n) => n.props.id === "layer-effects"),
  ).toBe(true);
  expect(wire(commits).diagnostics).toEqual([]);
  await panel.dispose();
  expect(requests.some((r) => r.operation.type === "unsubscribe")).toBe(true);
});
test("production panel browser bundle carries no Node or DOM runtime imports", async () => {
  const result = await Bun.build({
    entrypoints: [fileURLToPath(new URL("../index.ts", import.meta.url))],
    target: "browser",
    minify: true,
  });
  expect(result.success).toBe(true);
  const source = await result.outputs[0].text();
  for (const symbol of [
    "node:",
    'require("fs")',
    "window.",
    "document.",
    "process.env",
    "Buffer.from",
  ])
    expect(source).not.toContain(symbol);
});

test("a queued action cannot follow an acknowledgement onto a different target", async () => {
  const p = selected(),
    kit = platform();
  const requests: HostRequest[] = [];
  let finish!: (ack: HostAck) => void;
  let pending!: HostRequest;
  const host = new HostClient(
    { realm: "effects", generation: 1, project: "P" },
    async (r) => {
      requests.push(r);
      if (r.operation.type === "command") {
        pending = r;
        return new Promise<HostAck>((resolve) => {
          finish = resolve;
        });
      }
      return { ...r, accepted: true, revision: 0, value: p as unknown as Json };
    },
  );
  const c = new EffectsController(host, kit.api, "effects");
  await c.start();
  const first = c.drive({ type: "clear" });
  const second = c.drive({ type: "remove", id: "f1" });
  // Attach rejection handling before releasing the first acknowledgement.
  const refused = second.then(
    () => undefined,
    (error) => error,
  );
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  finish({
    ...pending,
    accepted: true,
    revision: 0,
    value: { ...p, layer: "b" } as unknown as Json,
  });
  await first;
  expect((await refused)?.message).toBe("Effect target changed");
  expect(requests.filter((r) => r.operation.type === "command")).toHaveLength(
    1,
  );
  await c.dispose();
});
