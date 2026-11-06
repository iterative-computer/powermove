import { expect, test } from "bun:test";
import {
  HostClient,
  type HostAck,
  type HostRequest,
} from "../../../src/host-api";
import { TimelineSession, nativeEvent } from "../session";
import { announcementText, checkFrame, windowRequest } from "../model";
import { InspectModel, frame, states } from "./fixtures";

const identity = {
  realm: "timeline",
  project: "synthetic-project",
  generation: 1,
};
function fixture(rejectSubscription?: string) {
  const requests: HostRequest[] = [];
  const errors: unknown[] = [];
  const host = new HostClient(identity, async (request) => {
    requests.push(request);
    const accepted = !(
      request.operation.type === "subscribe" &&
      request.operation.query === rejectSubscription
    );
    return {
      ...identity,
      version: 1,
      requestId: request.requestId,
      revision: 0,
      accepted,
      value: null,
      ...(accepted ? {} : { error: "synthetic rejection" }),
    } satisfies HostAck;
  });
  const model = new InspectModel();
  const session = new TimelineSession(
    model,
    host,
    { first: 0, visible: 10 },
    "timeline:instance:1",
    (e) => errors.push(e),
  );
  return { requests, host, model, session, errors };
}
function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((yes) => {
    resolve = yes;
  });
  return { promise, resolve };
}

test("host-data project, selection and time subscriptions invalidate native publications", async () => {
  const f = fixture();
  await f.session.start();
  await f.session.settled();
  expect(f.requests.map((r) => r.operation)).toEqual([
    {
      type: "subscribe",
      subscription: "timeline:instance:1:1:project",
      query: "project",
    },
    {
      type: "subscribe",
      subscription: "timeline:instance:1:2:selection",
      query: "selection",
    },
    {
      type: "subscribe",
      subscription: "timeline:instance:1:3:time",
      query: "time",
    },
  ]);
  expect(f.session.frame!.document).toBe("18446744073709551614");
  const before = f.model.windows.length;
  f.model.value = states.playing();
  expect(
    f.host.publish({
      ...identity,
      version: 1,
      subscription: "timeline:instance:1:3:time",
      revision: 0,
      sequence: 1,
      value: { notADocumentReplica: true },
    }),
  ).toBe(true);
  await f.session.settled();
  expect(f.model.windows.length).toBe(before + 1);
  expect(f.session.frame!.playing).toBe(true);
  expect(f.model.actions).toEqual([]);
  await f.session.dispose();
  expect(f.requests.slice(-3).map((r) => r.operation)).toEqual([
    { type: "unsubscribe", subscription: "timeline:instance:1:3:time" },
    { type: "unsubscribe", subscription: "timeline:instance:1:2:selection" },
    { type: "unsubscribe", subscription: "timeline:instance:1:1:project" },
  ]);
});

test("awaits the native model acknowledgement before publishing; does not optimistically toggle", async () => {
  const { model, session } = fixture();
  await session.start();
  await session.settled();
  const pending = deferred();
  model.wait = pending.promise;
  const action = session.act({ type: "snap" });
  await Promise.resolve();
  expect(session.frame!.snapping).toBe(true);
  model.value = frame({ snapping: false });
  pending.resolve();
  await action;
  expect(session.frame!.snapping).toBe(false);
  expect(model.actions).toEqual([{ type: "snap" }]);
  await session.dispose();
});

test("rejected edit remains rejected, refreshes native failure feedback and preserves last accepted rows", async () => {
  const { model, session } = fixture();
  await session.start();
  await session.settled();
  const originalRows = session.frame!.window.rows;
  model.reject = new Error("synthetic collision");
  model.value = {
    ...model.value,
    announcement: { sequence: "3", value: { kind: "keyCollision" } },
  };
  await expect(
    session.act({ type: "keyToggle", layer: "headline", path: "opacity" }),
  ).rejects.toThrow("synthetic collision");
  expect(session.frame!.window.rows).toBe(originalRows);
  expect(announcementText(session.frame!.announcement!.value)).toBe(
    "Keyframe time is taken by another keyframe",
  );
  await session.dispose();
});

test("event callback failures are inspectable, not swallowed or shown with invented copy", async () => {
  const { model, session, errors } = fixture();
  await session.start();
  await session.settled();
  model.reject = new Error("native lock rejection");
  session.send({ type: "play" });
  await session.settled();
  await Promise.resolve();
  expect(session.lastError).toBe(model.reject);
  expect(errors).toEqual([model.reject]);
  await session.dispose();
});

test("actions are serialized, detached before enqueue, and retain exact live modifiers", async () => {
  const { model, session } = fixture();
  await session.start();
  await session.settled();
  const e = { key: "ArrowRight", shiftKey: true, repeat: false };
  const first = session.act({ type: "keyDown", event: e });
  e.shiftKey = false;
  e.repeat = true;
  const second = session.act({ type: "snap" });
  await Promise.all([first, second]);
  expect(model.actions).toEqual([
    {
      type: "keyDown",
      event: { key: "ArrowRight", shiftKey: true, repeat: false },
    },
    { type: "snap" },
  ]);
  await session.dispose();
});

test("range changes are bounded projection requests, never native document edits", async () => {
  const { model, session } = fixture();
  await session.start();
  await session.settled();
  model.value = states["60k-rows"]();
  await session.act({ type: "range", first: 30_000, visible: 10 });
  expect(model.windows.at(-1)).toEqual({ first: 30_000, visible: 10 });
  expect(model.actions).toEqual([]);
  await expect(
    session.act({ type: "range", first: -1, visible: 10 }),
  ).rejects.toThrow();
  expect(model.windows.at(-1)).toEqual({ first: 30_000, visible: 10 });
  await session.dispose();
});

test("retirement fences queued input immediately and awaits active cancellation exactly once", async () => {
  const { model, session } = fixture();
  await session.start();
  await session.settled();
  const pending = deferred();
  model.wait = pending.promise;
  const first = session.act({ type: "play" });
  await Promise.resolve();
  const queued = session.act({ type: "snap" });
  const result = queued.catch((e) => e);
  const dispose = session.dispose();
  expect(session.dispose()).toBe(dispose);
  expect(model.disposals).toBe(0);
  pending.resolve();
  await first;
  await dispose;
  expect((await result).message).toBe("Timeline session retired");
  expect(model.actions).toEqual([{ type: "play" }]);
  expect(model.disposals).toBe(1);
  await expect(session.act({ type: "play" })).rejects.toThrow();
});

test("late projection results do not publish after retirement", async () => {
  const { model, session } = fixture();
  await session.start();
  await session.settled();
  let publications = 0;
  session.subscribe(() => publications++);
  const gate = deferred();
  model.inspect = async () => {
    await gate.promise;
    return states.playing();
  };
  const action = session.act({ type: "play" });
  await Promise.resolve();
  const dispose = session.dispose();
  gate.resolve();
  await action;
  await dispose;
  expect(publications).toBe(1);
  expect(session.frame!.playing).toBe(false);
});

test("partial host subscription failure cleans registered subscriptions and native lease", async () => {
  const { requests, model, session } = fixture("selection");
  await expect(session.start()).rejects.toThrow("synthetic rejection");
  expect(requests.at(-1)!.operation).toEqual({
    type: "unsubscribe",
    subscription: "timeline:instance:1:1:project",
  });
  expect(model.disposals).toBe(1);
  await session.dispose();
  expect(model.disposals).toBe(1);
});

test("dispose during setup waits for and then removes registrations; start/restart fails closed", async () => {
  const { model, session } = fixture();
  const start = session.start();
  const dispose = session.dispose();
  await start;
  await dispose;
  expect(model.windows).toEqual([]);
  expect(model.disposals).toBe(1);
  await expect(session.start()).rejects.toThrow();
});

test("cleanup failures remain rejected while independent native cleanup still runs", async () => {
  const { host, model, session } = fixture();
  await session.start();
  await session.settled();
  host.retire();
  await expect(session.dispose()).rejects.toThrow("Timeline retirement failed");
  expect(model.disposals).toBe(1);
});

test("stale host generations, sequences and retired subscription IDs cannot invalidate model", async () => {
  const { model, host, session } = fixture();
  await session.start();
  await session.settled();
  const before = model.windows.length;
  const p = {
    ...identity,
    version: 1 as const,
    subscription: "timeline:instance:1:1:project",
    revision: 0,
    sequence: 1,
    value: null,
  };
  expect(host.publish({ ...p, generation: 2 })).toBe(false);
  expect(host.publish(p)).toBe(true);
  await session.settled();
  expect(host.publish(p)).toBe(false);
  await session.settled();
  expect(model.windows.length).toBe(before + 1);
  await session.dispose();
  expect(host.publish({ ...p, sequence: 2 })).toBe(false);
});

test("window input and projection checks reject unsafe counts, duplicate IDs and nondecimal stamps", () => {
  for (const first of [-1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1])
    expect(() => windowRequest(first, 10)).toThrow();
  for (const size of [0, -1, 0.5, NaN, Infinity])
    expect(() => windowRequest(0, size)).toThrow();
  for (const value of [
    frame({ revision: "1.5" }),
    frame({ document: "bad" }),
    frame({ window: { total: -1, first: 0, rows: [] } }),
    frame({ window: { total: 1, first: 2, rows: [] } }),
    frame({
      window: {
        total: 2,
        first: 0,
        rows: [frame().window.rows[0], frame().window.rows[0]],
      },
    }),
    frame({ indexDigits: 0 }),
    frame({ selection: { kind: "layers", count: Infinity } }),
    states["60k-rows"](),
  ])
    expect(() => checkFrame(value, { first: 0, visible: 1 })).toThrow();
});

test("invalid native projection never replaces the last accepted frame", async () => {
  const { model, session } = fixture();
  await session.start();
  await session.settled();
  const prior = session.frame;
  model.value = frame({ revision: "invalid" });
  await expect(session.act({ type: "play" })).rejects.toThrow(
    "Invalid native identity",
  );
  expect(session.frame).toBe(prior);
  await session.dispose();
});

test("all VoiceOver announcements use exact spec copy", () => {
  expect([
    announcementText({ kind: "markerAdded", timecode: "00:00:00:12" }),
    announcementText({ kind: "markerRemoved" }),
    announcementText({ kind: "keyCollision" }),
    announcementText({ kind: "outsideComposition" }),
    announcementText({ kind: "locked" }),
  ]).toEqual([
    "Added marker at 00:00:00:12",
    "Removed marker",
    "Keyframe time is taken by another keyframe",
    "That time is outside the composition",
    "Layer is locked",
  ]);
});

test("native event projection strips methods/routing while retaining UTF-16/drag data", () => {
  const event = {
    type: "input",
    target: 1,
    currentTarget: 1,
    stopPropagation() {},
    value: "A日本🙂B",
    selection: [3, 5] as [number, number],
    composing: true,
    composition: [1, 3] as [number, number],
  };
  expect(nativeEvent(event)).toEqual({
    value: "A日本🙂B",
    selection: [3, 5],
    composing: true,
    composition: [1, 3],
  });
  expect(() => nativeEvent({ x: NaN })).toThrow();
});
