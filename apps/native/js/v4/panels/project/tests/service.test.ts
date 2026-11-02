import { expect, test } from "bun:test";
import { HostClient, type HostRequest, type Json } from "../../../src";
import { decodeReply, ProjectService } from "../service";
import { asset, owner, projection } from "./fixtures";
import type { Input, Stamp } from "../contract";
function fixture() {
  const requests: HostRequest[] = [];
  let stamp = { ...owner };
  let delay: Promise<void> | undefined;
  let reply: Json | undefined;
  let reject = false;
  const host = new HostClient(
    { realm: "fixture", project: "P", generation: 1 },
    async (request) => {
      requests.push(request);
      if (request.operation.type === "subscribe")
        return {
          ...request,
          revision: stamp.revision,
          accepted: true,
          value: { stamp } as unknown as Json,
        };
      if (request.operation.type === "command") {
        const sent = request.operation.args as unknown as { owner: Stamp };
        if (sent.owner.revision !== stamp.revision)
          throw new Error("stale captured revision");
        const capturedStamp = { ...stamp };
        await delay;
        if (reject)
          return {
            ...request,
            revision: stamp.revision,
            accepted: false,
            error: "Synthetic failure",
          };
        const next = { ...capturedStamp, revision: capturedStamp.revision + 1 };
        if (stamp.document === capturedStamp.document) stamp = next;
        const view = projection([asset()]);
        view.page.stamp = next;
        return {
          ...request,
          accepted: true,
          revision: next.revision,
          value: reply ?? ({ view } as unknown as Json),
        };
      }
      return { ...request, accepted: true, revision: stamp.revision };
    },
  );
  const service = new ProjectService(host, {
    command: "fixture.media",
    subscription: "fixture.project",
    projectStamp: (value) => (value as unknown as { stamp: Stamp }).stamp,
  });
  return {
    host,
    service,
    requests,
    replace(s: Stamp) {
      stamp = s;
    },
    setDelay(p: Promise<void>) {
      delay = p;
    },
    setReply(v: Json) {
      reply = v;
    },
    reject() {
      reject = true;
    },
  };
}
test("subscribes through host data; uses actual acks before publishing", async () => {
  const f = fixture();
  await f.service.start();
  expect(f.service.view!.page.stamp.revision).toBe(2);
  let release!: () => void;
  f.setDelay(new Promise((resolve) => (release = resolve)));
  const pending = f.service.input("Clear");
  await Promise.resolve();
  expect(f.service.view!.page.stamp.revision).toBe(2);
  release();
  await pending;
  expect(f.service.view!.page.stamp.revision).toBe(3);
  await f.service.dispose();
  expect(f.requests.at(-1)!.operation.type).toBe("unsubscribe");
});
test("concurrent edits capture the latest actual owner when dequeued", async () => {
  const f = fixture();
  await f.service.start();
  await Promise.all([f.service.input("Clear"), f.service.input("FilesEnter")]);
  expect(f.service.view!.page.stamp.revision).toBe(4);
  expect(
    f.requests
      .filter((r) => r.operation.type === "command")
      .map((r) => (r.operation as any).args.owner.revision),
  ).toEqual([1, 2, 3]);
  await f.service.dispose();
});
test("failed commands retain the last accepted view and inspectable error", async () => {
  const f = fixture();
  await f.service.start();
  const before = structuredClone(f.service.view);
  f.reject();
  f.service.run(f.service.input("FilesEnter"));
  await f.service.idle();
  expect(f.service.view).toEqual(before);
  expect(String(f.service.error)).toContain("Synthetic failure");
  await f.service.dispose();
});
test("document replacement fences an in-flight result", async () => {
  const f = fixture();
  await f.service.start();
  let release!: () => void;
  f.setDelay(new Promise((resolve) => (release = resolve)));
  const pending = f.service.input("Clear");
  for (let i = 0; i < 20 && f.requests.length < 3; i++) await Promise.resolve();
  const stamp = { ...owner, document: 2, revision: 2 };
  f.replace(stamp);
  f.host.publish({
    ...f.host.identity,
    version: 1,
    subscription: "fixture.project",
    revision: 2,
    sequence: 1,
    value: { stamp } as unknown as Json,
  });
  expect(f.service.view).toBeNull();
  release();
  expect(await pending).toBeNull();
  await f.service.idle();
  await f.service.dispose();
});
test("dispose fences late replies and releases host subscription", async () => {
  const f = fixture();
  await f.service.start();
  let release!: () => void;
  f.setDelay(new Promise((resolve) => (release = resolve)));
  const pending = f.service.input("Clear");
  await Promise.resolve();
  const disposed = f.service.dispose();
  release();
  expect(await pending).toBeNull();
  await disposed;
  expect(f.service.view).toBeNull();
  expect(await f.service.input("Clear")).toBeNull();
});
test("command arguments are copied at submission", async () => {
  const f = fixture();
  await f.service.start();
  const input: Input = { BeginRename: "authored" };
  const pending = f.service.input(input);
  input.BeginRename = "mutated";
  await pending;
  expect(
    (f.requests.at(-1)!.operation as any).args.action.Input.BeginRename,
  ).toBe("authored");
  await f.service.dispose();
});
test("invalid, foreign, unsafe-integer and inconsistent projections refuse", () => {
  const view = projection([asset()]);
  for (const broken of [
    null,
    {
      view: {
        ...view,
        page: { ...view.page, stamp: { ...owner, project: "other" } },
      },
    },
    {
      view: {
        ...view,
        page: {
          ...view.page,
          stamp: { ...owner, document: Number.MAX_SAFE_INTEGER + 1 },
        },
      },
    },
    { view: { ...view, page: { ...view.page, total: 0 } } },
    { view: { ...view, page: { ...view.page, rows: [asset(), asset()] } } },
  ])
    expect(() => decodeReply(broken, owner)).toThrow();
});
test("bounded page windows reject before a host request", async () => {
  const f = fixture();
  await f.service.start();
  const n = f.requests.length;
  for (const [offset, limit] of [
    [-1, 1],
    [0, 4097],
    [0, 0.5],
  ])
    await expect(f.service.page(offset, limit)).rejects.toThrow();
  expect(f.requests.length).toBe(n);
  await f.service.dispose();
});

test("unknown future statuses and missing native fields cannot invent ready media", () => {
  for (const state of [
    "Changed",
    { Cloud: { provider: "Fixture" } },
    { Ready: true },
  ]) {
    const row = asset();
    if ("Asset" in row.data) row.data.Asset[1].availability = state as never;
    expect(() => decodeReply({ view: projection([row]) }, owner)).toThrow();
  }
  const row = asset();
  if ("Asset" in row.data) delete (row.data.Asset[0] as any).duration;
  expect(() => decodeReply({ view: projection([row]) }, owner)).toThrow();
});

test("queued inputs retain their original document and never retarget on switch", async () => {
  const f = fixture();
  await f.service.start();
  let release!: () => void;
  f.setDelay(new Promise((resolve) => (release = resolve)));
  const first = f.service.input("Clear");
  for (let i = 0; i < 20 && f.requests.length < 3; i++) await Promise.resolve();
  const queued = f.service.input("FilesEnter");
  const stamp = { ...owner, document: 2, revision: 2 };
  f.replace(stamp);
  f.host.publish({
    ...f.host.identity,
    version: 1,
    subscription: "fixture.project",
    revision: 2,
    sequence: 1,
    value: { stamp } as unknown as Json,
  });
  release();
  expect(await first).toBeNull();
  expect(await queued).toBeNull();
  await f.service.idle();
  expect(
    f.requests
      .filter((r) => r.operation.type === "command")
      .some((r) => (r.operation as any).args.action.Input === "FilesEnter"),
  ).toBe(false);
  await f.service.dispose();
});
test("old rendered controls and native menus refuse stale revision intent", async () => {
  const f = fixture();
  await f.service.start();
  const captured = f.service.view!.page.stamp;
  await f.service.input("Clear");
  const count = f.requests.length;
  expect(await f.service.input("FilesEnter", captured)).toBeNull();
  expect(await f.service.menu(null, captured)).toBeNull();
  expect(f.requests.length).toBe(count);
  await f.service.dispose();
});

test("late same-document publications cannot regress an acknowledged projection", async () => {
  const f = fixture();
  await f.service.start();
  const count = f.requests.length;
  f.host.publish({
    ...f.host.identity,
    version: 1,
    subscription: "fixture.project",
    revision: 1,
    sequence: 1,
    value: { stamp: owner } as unknown as Json,
  });
  await f.service.idle();
  expect(f.service.view!.page.stamp.revision).toBe(2);
  expect(f.requests.length).toBe(count);
  await f.service.dispose();
});
