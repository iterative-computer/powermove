import { test, expect } from "bun:test";
import { HostClient } from "../../src/host-api";
import type { HostRequest, HostAck } from "../../src/host-api";
import { HostAgentModel, type AgentRegistration } from "./host";
import type { Publication } from "./contract";
import { fixture } from "./fixtures";

function setup() {
  let publish = (p: Publication) => {};
  let stopped = 0;
  const requests: HostRequest[] = [];
  let rejected = false;
  let mismatch = false;
  let beforeRead: (() => void) | undefined;
  const client = new HostClient(
    { realm: "synthetic-agent", project: "synthetic-project", generation: 1 },
    async (request) => {
      requests.push(request);
      if (
        request.operation.type === "command" &&
        request.operation.command === "registered.read"
      )
        beforeRead?.();
      return {
        ...request,
        accepted: !rejected,
        revision: 0,
        generation: mismatch ? 2 : 1,
        value:
          request.operation.type === "command" &&
          request.operation.command === "registered.read"
            ? (fixture() as unknown as HostAck["value"])
            : null,
        error: rejected ? "synthetic admission refused" : undefined,
      };
    },
  );
  const registration: AgentRegistration = {
    read: "registered.read",
    dispatch: "registered.dispatch",
    subscribe: (callback) => {
      publish = callback;
      return () => {
        stopped++;
      };
    },
  };
  return {
    client,
    registration,
    requests,
    publish: (p: Publication) => publish(p),
    get stopped() {
      return stopped;
    },
    reject: () => {
      rejected = true;
    },
    mismatch: () => {
      mismatch = true;
    },
    beforeRead: (callback: () => void) => {
      beforeRead = callback;
    },
  };
}
test("no registered agent model means a visible connection failure, never a fake local model", async () => {
  const client = new HostClient(
    { realm: "synthetic-agent", project: "synthetic-project", generation: 1 },
    async (request) => ({
      ...request,
      accepted: true,
      revision: 0,
      value: null,
    }),
  );
  await expect(
    HostAgentModel.connect(client, {
      read: "unregistered.read",
      dispatch: "unregistered.dispatch",
      subscribe: () => () => {},
    }),
  ).rejects.toThrow("Invalid agent publication");
  client.retire();
});
test("public HostClient sends registered ids and exact project/thread/run/sequence without optimistic mutation", async () => {
  const f = setup();
  const m = await HostAgentModel.connect(f.client, f.registration);
  const before = m.inspect();
  await m.dispatch({ type: "submit" });
  expect(f.requests.at(-1)?.operation).toEqual({
    type: "command",
    command: "registered.dispatch",
    args: {
      sequence: 1,
      target: { project: "synthetic-project", thread: "thread", run: null },
      action: { type: "submit" },
    },
  });
  expect(m.inspect()).toEqual(before);
  m.dispose();
  expect(f.stopped).toBe(1);
});
test("publication racing the read ack wins, and same-sequence publications cannot regress the UI", async () => {
  const f = setup();
  const newer = fixture();
  newer.sequence = 2;
  newer.snapshot.draft.text = "Authored";
  newer.snapshot.draft.selection = [8, 8];
  f.beforeRead(() => f.publish(newer));
  const m = await HostAgentModel.connect(f.client, f.registration);
  expect(m.inspect().sequence).toBe(2);
  let updates = 0;
  const stop = m.subscribe(() => updates++);
  f.publish(fixture());
  expect(updates).toBe(0);
  const p = m.inspect();
  p.sequence = 3;
  f.publish(p);
  expect(updates).toBe(1);
  stop();
  m.dispose();
});
test("caller mutations cannot alter published state or queued authored commands", async () => {
  const f = setup();
  const m = await HostAgentModel.connect(f.client, f.registration);
  const p = m.inspect();
  p.snapshot.threadId = "foreign";
  const action = {
    type: "draft" as const,
    text: "日本🙂",
    selection: [4, 4] as [number, number],
  };
  const pending = m.dispatch(action);
  action.text = "changed";
  await pending;
  expect(m.inspect().snapshot.threadId).toBe("thread");
  expect(f.requests.at(-1)?.operation).toMatchObject({
    args: { action: { text: "日本🙂" } },
  });
  m.dispose();
});
test("rejected commands preserve native draft and allow an explicit corrected request", async () => {
  const f = setup();
  const m = await HostAgentModel.connect(f.client, f.registration);
  f.reject();
  await expect(m.dispatch({ type: "submit" })).rejects.toThrow("refused");
  expect(m.inspect().snapshot.draft.text).toBe("");
  m.dispose();
});
test("foreign generation acknowledgements fence later work", async () => {
  const f = setup();
  const m = await HostAgentModel.connect(f.client, f.registration);
  f.mismatch();
  await expect(m.dispatch({ type: "stop" })).rejects.toThrow("acknowledgement");
  await expect(m.dispatch({ type: "submit" })).rejects.toThrow("ambiguous");
  m.dispose();
});
test("retirement detaches publication subscription, blocks new work and drops queued work", async () => {
  const f = setup();
  const m = await HostAgentModel.connect(f.client, f.registration);
  const pending = m.dispatch({ type: "submit" });
  m.dispose();
  m.dispose();
  await expect(pending).rejects.toThrow("unavailable");
  await expect(m.dispatch({ type: "stop" })).rejects.toThrow("unavailable");
  expect(f.requests).toHaveLength(1);
  expect(f.stopped).toBe(1);
  f.publish(fixture());
});
for (const [name, mutate] of Object.entries({
  foreignProject: (p: Publication) => {
    p.snapshot.projectId = "foreign";
  },
  foreignGeneration: (p: Publication) => {
    p.generation = 2;
  },
  malformedRange: (p: Publication) => {
    p.snapshot.rows.range = [1, 0];
  },
  countWithoutRows: (p: Publication) => {
    p.snapshot.rows.range = [0, 1];
    p.snapshot.rows.count = 1;
  },
  missingNativeFacts: (p: Publication) => {
    delete (p.native as Partial<Publication["native"]>).bindings;
  },
  invalidSelection: (p: Publication) => {
    p.snapshot.draft.selection = [0, 100];
  },
}))
  test(`malformed ${name} cannot replace a native publication`, async () => {
    const f = setup();
    const m = await HostAgentModel.connect(f.client, f.registration);
    const p = fixture();
    p.sequence = 2;
    mutate(p);
    f.publish(p);
    expect(() => m.inspect()).toThrow("Invalid agent publication");
    await expect(m.dispatch({ type: "submit" })).rejects.toThrow();
    m.dispose();
  });
test("malformed native publications notify mounted subscribers of the failure", async () => {
  const f = setup();
  const m = await HostAgentModel.connect(f.client, f.registration);
  let failures = 0;
  m.subscribe(() => {
    try {
      m.inspect();
    } catch {
      failures++;
    }
  });
  const bad = fixture();
  bad.generation = 99;
  f.publish(bad);
  expect(failures).toBe(1);
  m.dispose();
});
