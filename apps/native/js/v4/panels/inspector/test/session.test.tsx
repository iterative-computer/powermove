import { expect, test } from "bun:test";
import { fixture, snapshot } from "./fixture";
import { InspectorPanel } from "../view";
import { Probe } from "./probe";
import { mountInspector } from "../index";

test("all four host projections subscribe and each binding is acknowledged before editing", async () => {
  let release!: () => void;
  const f = fixture(snapshot(), async (request) => {
    if (request.operation.type === "binding.register")
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    return {
      ...request,
      accepted: true,
      revision: request.expectedRevision,
      value: null,
    };
  });
  const start = f.session.start();
  for (let i = 0; i < 20 && !release; i++) await Promise.resolve();
  const p = new Probe();
  try {
    await p.render(<InspectorPanel session={f.session} />);
    expect(p.node("value:opacity:0").props.disabled).toBe(true);
    release();
    await start;
    await p.flush();
    expect(p.node("value:opacity:0").props.disabled).toBe(false);
    expect(f.model.updates.map(([query]) => query)).toEqual([
      "project",
      "selection",
      "time",
      "evaluated",
    ]);
    expect(
      f.requests.filter((r) => r.operation.type === "binding.register"),
    ).toHaveLength(1);
  } finally {
    await p.dispose();
    await f.session.dispose();
  }
});

test("binding rejection leaves controls disabled and reports the actual host failure", async () => {
  const f = fixture(snapshot(), async (request) => ({
    ...request,
    accepted: request.operation.type !== "binding.register",
    error: "rejected binding",
    revision: request.expectedRevision,
    value: null,
  }));
  await f.session.start();
  expect(f.session.ready("opacity")).toBe(false);
  expect(f.errors.length).toBeGreaterThan(0);
  expect(String(f.errors[0])).toContain("rejected binding");
  await f.session.dispose();
});

test("subscription startup failure releases every acknowledged resource", async () => {
  const f = fixture(snapshot(), async (request) => ({
    ...request,
    accepted: !(
      request.operation.type === "subscribe" &&
      request.operation.query === "time"
    ),
    error: "subscription declined",
    revision: request.expectedRevision,
    value: null,
  }));
  await expect(f.session.start()).rejects.toThrow("subscription declined");
  const operations = f.requests.map((r) => r.operation);
  expect(operations).toContainEqual({
    type: "unsubscribe",
    subscription: "inspector:selection",
  });
  expect(operations).toContainEqual({
    type: "unsubscribe",
    subscription: "inspector:project",
  });
  expect(operations).toContainEqual({
    type: "binding.dispose",
    handle: "opacity",
  });
});

test("readout coalesces playback bursts and pause immediately publishes the exact stopped value", async () => {
  const f = fixture();
  await f.session.start();
  let notifications = 0;
  f.session.subscribe(() => notifications++);
  f.model.value.playing = true;
  for (let i = 0; i < 100; i++) {
    f.model.value.time = i;
    f.publish("time");
  }
  expect(f.clock.callbacks.size).toBe(1);
  expect(f.session.inspect().time).toBe(0);
  f.clock.frame();
  await f.session.settled();
  expect(f.session.inspect().time).toBe(99);
  expect(notifications).toBeLessThanOrEqual(2);
  f.model.value.time = 100;
  f.publish("time");
  expect(f.clock.callbacks.size).toBe(1);
  f.model.value.playing = false;
  f.model.value.time = 100.125;
  f.publish("time");
  expect(f.clock.callbacks.size).toBe(0);
  expect(f.session.inspect().time).toBe(100.125);
  await f.session.dispose();
});

test("rebind disposes original identity before registration, collapse and retirement clean up", async () => {
  const f = fixture();
  await f.session.start();
  const row = f.model.value.sections[0].rows[0];
  if (row.control.kind === "number")
    row.control.axes[0].binding.channels = ["replacement-channel"];
  const start = f.requests.length;
  f.publish();
  await f.session.settled();
  expect(f.requests.slice(start).map((r) => r.operation)).toEqual([
    { type: "binding.dispose", handle: "opacity" },
    {
      type: "binding.register",
      handle: "opacity",
      channels: ["replacement-channel"],
    },
  ]);
  f.model.value.sections[0].collapsed = true;
  f.publish();
  await f.session.settled();
  expect(f.session.ready("opacity")).toBe(false);
  await f.session.dispose();
  const end = f.requests.length;
  await f.session.dispose();
  expect(f.requests.length).toBe(end);
  expect(f.publish()).toBe(false);
  await expect(
    f.session.dispatch({ type: "key", row: "opacity", removeAnimation: false }),
  ).rejects.toThrow("retired");
});

test("actor rejection is never optimistically reflected and atomic edit limits are retained", async () => {
  const f = fixture(snapshot(), async (request) => ({
    ...request,
    accepted: request.operation.type !== "edit",
    error: "locked target",
    revision: request.expectedRevision,
    value: null,
  }));
  await f.session.start();
  f.model.operation = {
    type: "edit",
    label: "Change Opacity",
    commands: [{ command: "test.set", args: { value: 55 } }],
  };
  await expect(
    f.session.dispatch({ type: "key", row: "opacity", removeAnimation: false }),
  ).rejects.toThrow("locked target");
  expect(f.session.inspect().sections[0].rows[0]).toEqual(
    snapshot().sections[0].rows[0],
  );
  f.model.operation.commands = Array.from({ length: 257 }, () => ({
    command: "test.set",
    args: null,
  }));
  const count = f.requests.length;
  await expect(
    f.session.dispatch({ type: "key", row: "opacity", removeAnimation: false }),
  ).rejects.toThrow();
  expect(f.requests.length).toBe(count);
  await f.session.dispose();
});

test("locked, disabled, hidden and stale effect-target events never plan an edit", async () => {
  const f = fixture();
  await f.session.start();
  const row = f.model.value.sections[0].rows[0];
  row.locked = true;
  f.publish();
  await f.session.settled();
  expect(
    await f.session.dispatch({
      type: "key",
      row: "opacity",
      removeAnimation: false,
    }),
  ).toBe(false);
  row.locked = false;
  row.disabled = true;
  f.publish();
  await f.session.settled();
  expect(
    await f.session.dispatch({
      type: "key",
      row: "opacity",
      removeAnimation: false,
    }),
  ).toBe(false);
  f.model.value.sections[0].collapsed = true;
  f.publish();
  await f.session.settled();
  expect(
    await f.session.dispatch({
      type: "key",
      row: "opacity",
      removeAnimation: false,
    }),
  ).toBe(false);
  expect(
    await f.session.dispatch({
      type: "drop-effect",
      target: "old-layer",
      effect: {},
    }),
  ).toBe(false);
  expect(f.model.intents).toEqual([]);
  await f.session.dispose();
});

test("dispose during binding admission releases the late handle and every subscription", async () => {
  let release!: () => void;
  const f = fixture(snapshot(), async (request) => {
    if (request.operation.type === "binding.register")
      await new Promise<void>((resolve) => {
        release = resolve;
      });
    return {
      ...request,
      accepted: true,
      revision: request.expectedRevision,
      value: null,
    };
  });
  const start = f.session.start();
  for (let i = 0; i < 20 && !release; i++) await Promise.resolve();
  const dispose = f.session.dispose();
  release();
  await start;
  await dispose;
  expect(f.session.ready("opacity")).toBe(false);
  expect(f.requests.map((r) => r.operation)).toContainEqual({
    type: "binding.dispose",
    handle: "opacity",
  });
  const admitted = f.requests.filter(
    (r) => r.operation.type === "subscribe",
  ).length;
  const retired = f.requests.filter(
    (r) => r.operation.type === "unsubscribe",
  ).length;
  expect(retired).toBe(admitted);
});

test("mounted root shares host generation and independent disposal retires all resources once", async () => {
  const f = fixture(snapshot(), undefined, 7),
    retired: number[] = [];
  const panel = await mountInspector(
    f.session,
    async (commit) => ({ accepted: true, revision: commit.baseRevision + 1 }),
    {
      retire: async (generation) => {
        retired.push(generation);
      },
    },
  );
  expect(panel.root.inspect().generation).toBe(7);
  await panel.root.dispose();
  await panel.dispose();
  expect(retired).toEqual([7]);
  expect(f.session.ready("opacity")).toBe(false);
  expect(
    f.requests.filter((r) => r.operation.type === "unsubscribe"),
  ).toHaveLength(4);
});

test("fenced startup retires native generation without another surface mutation", async () => {
  const f = fixture(),
    retired: number[] = [];
  let commits = 0;
  await expect(
    mountInspector(
      f.session,
      async () => {
        commits++;
        return { accepted: false, revision: 0, error: "surface rejected" };
      },
      {
        retire: async (generation) => {
          retired.push(generation);
        },
      },
    ),
  ).rejects.toThrow("surface rejected");
  expect(commits).toBe(1);
  expect(retired).toEqual([1]);
  expect(f.session.ready("opacity")).toBe(false);
});
