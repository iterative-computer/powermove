import { expect, test } from "bun:test";
import { HostClient, type HostAck, type HostRequest, type Json } from "../src";
import { OwnerHost, exerciseHostData } from "../eval/host-fixture";
const identity = { realm: "test", generation: 3, project: "synthetic" };
const ack = (
  request: HostRequest,
  revision = request.expectedRevision,
): HostAck => ({
  ...identity,
  version: 1,
  requestId: request.requestId,
  revision,
  accepted: true,
});

test("public client exercises native data routing, actor rejection and independent settings", async () => {
  const host = new OwnerHost();
  try {
    await exerciseHostData(host);
  } finally {
    await host.retire();
  }
});
test("concurrent edits serialize on actual actor acknowledgements and copy submitted intent", async () => {
  const requests: HostRequest[] = [];
  const replies: ((ack: HostAck) => void)[] = [];
  const client = new HostClient(
    identity,
    (request) => {
      requests.push(request);
      return new Promise((resolve) => replies.push(resolve));
    },
    5,
  );
  const commands = [
    { command: "annotation.upsert", args: { value: "日本🙂" } },
  ];
  const first = client.edit("One", commands);
  const second = client.edit("Two", commands);
  commands[0].args.value = "changed by caller";
  await Promise.resolve();
  expect(requests).toHaveLength(1);
  expect(requests[0]).toMatchObject({
    expectedRevision: 5,
    operation: {
      type: "edit",
      commands: [{ command: "annotation.upsert", args: { value: "日本🙂" } }],
    },
  });
  replies[0](ack(requests[0], 6));
  await first;
  await Promise.resolve();
  expect(requests[1].expectedRevision).toBe(6);
  replies[1](ack(requests[1], 7));
  expect((await second).revision).toBe(7);
});
test("declined edits expose actor error, observe latest revision and allow explicit corrected request", async () => {
  const requests: HostRequest[] = [];
  const client = new HostClient(
    identity,
    async (request) => {
      requests.push(request);
      return requests.length === 1
        ? { ...ack(request, 9), accepted: false, error: "protected" }
        : ack(request, 10);
    },
    5,
  );
  await expect(
    client.edit("Edit", [{ command: "test", args: false }]),
  ).rejects.toThrow("protected");
  await client.edit("Corrected", [{ command: "test", args: null }]);
  expect(requests[1].expectedRevision).toBe(9);
});
test("subscription buffers publications until registration ack and never regresses to its initial value", async () => {
  let reply!: (ack: HostAck) => void;
  let request!: HostRequest;
  const seen: Json[] = [];
  const client = new HostClient(identity, (r) => {
    request = r;
    return new Promise((resolve) => {
      reply = resolve;
    });
  });
  const registration = client.subscribe("clock", "time", (value) =>
    seen.push(value),
  );
  await Promise.resolve();
  const publication = {
    ...identity,
    version: 1 as const,
    subscription: "clock",
    revision: 0,
    sequence: 1,
    value: 1.25,
  };
  expect(client.publish(publication)).toBe(true);
  expect(client.publish(publication)).toBe(false);
  expect(seen).toEqual([]);
  reply({ ...ack(request), value: 0 });
  await registration;
  expect(seen).toEqual([1.25]);
  expect(client.publish({ ...publication, sequence: 2, value: 2.5 })).toBe(
    true,
  );
  expect(seen).toEqual([1.25, 2.5]);
  client.retire();
  expect(client.publish({ ...publication, sequence: 3 })).toBe(false);
});
test("subscription rejection delivers no buffered callback and detaches local registration", async () => {
  let reply!: (ack: HostAck) => void;
  let request!: HostRequest;
  const seen: Json[] = [];
  const client = new HostClient(identity, (r) => {
    request = r;
    return new Promise((resolve) => {
      reply = resolve;
    });
  });
  const registration = client.subscribe("clock", "time", (value) =>
    seen.push(value),
  );
  await Promise.resolve();
  const publication = {
    ...identity,
    version: 1 as const,
    subscription: "clock",
    revision: 0,
    sequence: 1,
    value: 1.25,
  };
  client.publish(publication);
  reply({ ...ack(request), accepted: false, error: "declined" });
  await expect(registration).rejects.toThrow("declined");
  expect(seen).toEqual([]);
  expect(client.publish({ ...publication, sequence: 2 })).toBe(false);
});
test("acknowledged subscription IDs never recycle within a generation", async () => {
  const client = new HostClient(identity, async (request) => ({
    ...ack(request),
    value: [],
  }));
  const stop = await client.subscribe("selection", "selection", () => {});
  await stop();
  await expect(
    client.subscribe("selection", "selection", () => {}),
  ).rejects.toThrow("retired subscription");
  expect(
    client.publish({
      ...identity,
      version: 1,
      subscription: "selection",
      revision: 0,
      sequence: 10,
      value: ["old"],
    }),
  ).toBe(false);
});
test("retirement drops queued edits before they reach the actor", async () => {
  let submitted = 0;
  const client = new HostClient(identity, async (request) => {
    submitted++;
    return ack(request);
  });
  const pending = client.edit("Edit", [{ command: "test", args: {} }]);
  client.retire();
  await expect(pending).rejects.toThrow("retired");
  expect(submitted).toBe(0);
});
test("malformed atomic edits never reach the transport", async () => {
  let submitted = 0;
  const client = new HostClient(identity, async (request) => {
    submitted++;
    return ack(request);
  });
  for (const [label, commands] of [
    ["", [{ command: "test", args: {} }]],
    ["Edit", []],
    ["Edit", [{ command: "", args: {} }]],
  ] as const)
    await expect(client.edit(label, [...commands])).rejects.toThrow(
      "Invalid edit",
    );
  expect(submitted).toBe(0);
});
test("edit validation never invokes getters in authored commands", async () => {
  let invoked = 0;
  const client = new HostClient(identity, async (request) => ack(request));
  await expect(
    client.edit("Edit", [
      {
        get command() {
          invoked++;
          return "test";
        },
        args: {},
      },
    ]),
  ).rejects.toThrow("accessors");
  expect(invoked).toBe(0);
});
test("a throwing initial subscriber releases its acknowledged native subscription", async () => {
  const requests: HostRequest[] = [];
  const client = new HostClient(identity, async (request) => {
    requests.push(request);
    return { ...ack(request), value: [] };
  });
  await expect(
    client.subscribe("selection", "selection", () => {
      throw new Error("subscriber failed");
    }),
  ).rejects.toThrow("subscriber failed");
  expect(requests.map((request) => request.operation.type)).toEqual([
    "subscribe",
    "unsubscribe",
  ]);
  expect(
    client.publish({
      ...identity,
      version: 1,
      subscription: "selection",
      revision: 0,
      sequence: 1,
      value: [],
    }),
  ).toBe(false);
});
test("malformed publications do not consume cursors or invoke accessors", async () => {
  const seen: Json[] = [];
  let invoked = 0;
  const client = new HostClient(identity, async (request) => ({
    ...ack(request),
    value: [],
  }));
  await client.subscribe("selection", "selection", (value) => seen.push(value));
  const publication = {
    ...identity,
    version: 1 as const,
    subscription: "selection",
    revision: 0,
    sequence: 1,
    value: ["valid"],
  };
  expect(() =>
    client.publish({
      ...publication,
      get value() {
        invoked++;
        return [];
      },
    }),
  ).toThrow("accessors");
  expect(client.publish({ ...publication, sequence: 0 })).toBe(false);
  expect(client.publish(publication)).toBe(true);
  expect(invoked).toBe(0);
  expect(seen).toEqual([[], ["valid"]]);
});
