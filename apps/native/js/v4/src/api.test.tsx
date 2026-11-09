import { expect, test } from "bun:test";
import {
  createRoot,
  h,
  HostClient,
  type Commit,
  type HostAck,
  type HostRequest,
  type Transport,
  type SurfaceEvent,
} from ".";
import { useState } from "./hooks";
import type { JSX } from "./jsx-runtime";

const accept: Transport = async (c) => ({
  accepted: true,
  revision: c.baseRevision + 1,
});
function field(root: ReturnType<typeof createRoot>, id = "field") {
  return root.inspect().nodes.find((n) => n.props.id === id)!;
}
test("inspection separates exact native UTF-16 input from accepted value and echo emits no value op", async () => {
  const commits: Commit[] = [];
  const root = createRoot(async (c) => {
    commits.push(structuredClone(c));
    return accept(c);
  });
  function Caption() {
    const [value, set] = useState("A🙂B");
    return (
      <input id="field" value={value} onInput={(e) => set(String(e.value))} />
    );
  }
  await root.render(<Caption />);
  const id = field(root).id;
  await root.drive(id, "input", {
    value: "A日本🙂B",
    selection: [3, 5],
    composing: true,
    composition: [1, 3],
  });
  const snapshot = root.inspect();
  expect(field(root).props.value).toBe("A日本🙂B");
  expect(snapshot.acceptedNodes.find((n) => n.id === id)!.props.value).toBe(
    "A🙂B",
  );
  expect(snapshot.inputs[id]).toMatchObject({
    value: "A日本🙂B",
    selection: [3, 5],
    composition: [1, 3],
    composing: true,
  });
  expect(commits).toHaveLength(1);
  snapshot.inputs[id].selection[0] = 0;
  expect(root.inspect().inputs[id].selection).toEqual([3, 5]);
  await root.dispose();
});
test("input ranges, external values, removals and stale drives cannot mutate accepted state", async () => {
  const root = createRoot(accept, undefined, { generation: 4 });
  await root.render(<input id="field" value="🙂" />);
  const id = field(root).id;
  const before = root.inspect();
  await expect(
    root.drive(id, "input", { value: "🙂", selection: [0, 3] }),
  ).rejects.toThrow("UTF-16");
  await expect(
    root.drive(id, "input", {
      value: "🙂",
      selection: [0, 1],
      composing: false,
      composition: [0, 1],
    }),
  ).rejects.toThrow("Composition");
  await expect(root.drive(id, "input", { value: "bad" }, 0, 4)).rejects.toThrow(
    "Stale",
  );
  await expect(root.drive(id, "input", { value: "bad" }, 1, 3)).rejects.toThrow(
    "Stale",
  );
  expect(root.inspect()).toEqual(before);
  await root.drive(id, "input", {
    value: "日本🙂",
    selection: [2, 4],
    composing: true,
    composition: [0, 2],
  });
  await root.render(<input id="field" value="external" />);
  expect(root.inspect().inputs[id]).toBeUndefined();
  expect(field(root).props.value).toBe("external");
  await root.render(null);
  expect(root.inspect().inputs).toEqual({});
  await root.dispose();
});
test("accepted stylesheet is isolated and rejected replacement keeps the full snapshot", async () => {
  let reject = false;
  const root = createRoot(async (c) =>
    reject
      ? { accepted: false, revision: c.baseRevision, error: "test" }
      : accept(c),
  );
  await root.render(<button id="target" />);
  const rules = [
    {
      selector: { kind: "id" as const, value: "target" },
      style: { opacity: 0.25 },
    },
  ];
  await root.setStyleRules(rules);
  rules[0].style.opacity = 1;
  const before = root.inspect();
  expect(before.nodes[0].props.styleRules).toEqual([
    {
      selector: { kind: "id" as const, value: "target" },
      style: { opacity: 0.25 },
    },
  ]);
  reject = true;
  await expect(root.setStyleRules([])).rejects.toThrow("rejected");
  expect(root.inspect().nodes).toEqual(before.nodes);
  expect(root.inspect().revision).toBe(before.revision);
  await root.dispose();
});
test("drive plans accepted capture/bubble and typed pointer/modifier/drag details", async () => {
  const received: string[] = [];
  let observed: SurfaceEvent | undefined;
  const root = createRoot(accept);
  await root.render(
    <div
      onClickCapture={(e) => {
        received.push("capture");
        observed = e;
      }}
    >
      <button
        id="field"
        preventDefault={{ click: true }}
        onClick={(e) => {
          received.push("bubble");
          expect(e.defaultPrevented).toBe(true);
        }}
      />
    </div>,
  );
  const id = field(root).id;
  await root.drive(id, "click", {
    metaKey: true,
    ctrlKey: true,
    button: 0,
    buttons: 1,
    clickCount: 2,
    x: 0.25,
    y: -3,
    drag: { types: ["effect"], data: { effect: "test" }, effect: "copy" },
  });
  expect(received).toEqual(["capture", "bubble"]);
  expect(observed).toMatchObject({
    metaKey: true,
    ctrlKey: true,
    x: 0.25,
    y: -3,
    clickCount: 2,
    drag: { effect: "copy" },
  });
  await expect(root.drive(id, "click", { x: Infinity })).rejects.toThrow();
  root.capturePointer(7, id);
  await expect(
    root.drive(0, "pointercancel", { pointerId: 7, selection: [2, 1] }),
  ).rejects.toThrow();
  await root.drive(0, "click", { pointerId: 7 });
  expect(received).toHaveLength(4);
  await root.drive(0, "pointercancel", { pointerId: 7 });
  await root.drive(0, "click", { pointerId: 7 });
  expect(received).toHaveLength(4);
  await root.dispose();
});
test("fenced disposal retires independently, is idempotent, preserves checkpoint and accepts only reset generation ack", async () => {
  let reject = false;
  let commits = 0;
  const retired: number[] = [];
  const root = createRoot(
    async (c) => {
      commits++;
      return reject ? { accepted: false, revision: c.baseRevision } : accept(c);
    },
    undefined,
    {
      generation: 3,
      retire: async (g) => {
        retired.push(g);
      },
    },
  );
  await root.render(<button id="field" />);
  const before = root.inspect();
  reject = true;
  await expect(root.render(<text>rejected</text>)).rejects.toThrow();
  const saved = { count: 7, unknown: { exact: false } };
  const recovered = await root.recover(saved, {
    reset: async (request) => {
      expect(request).toEqual({
        generation: 3,
        nextGeneration: 4,
        state: saved,
      });
      return { accepted: true, generation: 4, revision: 0, transport: accept };
    },
  });
  await root.dispose();
  expect(commits).toBe(2);
  expect(retired).toEqual([3]);
  expect(root.inspect().acceptedNodes).toEqual(before.acceptedNodes);
  saved.count = 99;
  expect(recovered.state).toMatchObject({ count: 7 });
  expect(() => root.render(null)).toThrow("disposed");
  await recovered.root.render(<button />);
  expect(recovered.root.inspect().generation).toBe(4);
  await expect(
    recovered.root.recover(
      {},
      {
        reset: async () => ({
          accepted: true,
          generation: 4,
          revision: 0,
          transport: accept,
        }),
      },
    ),
  ).rejects.toThrow("reset rejected");
});
test("disposal unmounts hook effects despite rejected transport", async () => {
  const { useLayoutEffect } = await import("./hooks");
  let cleanups = 0;
  let reject = false;
  const root = createRoot(async (c) =>
    reject ? { accepted: false, revision: c.baseRevision } : accept(c),
  );
  function Component() {
    useLayoutEffect(
      () => () => {
        cleanups++;
      },
      [],
    );
    return <input />;
  }
  await root.render(<Component />);
  reject = true;
  await expect(root.render(<Component />)).resolves.toBeNull();
  root.fail(new Error("worker failed"));
  await root.dispose();
  expect(cleanups).toBe(1);
});
const identity = { realm: "test", project: "synthetic", generation: 2 };
const ack = (r: HostRequest): HostAck => ({
  ...identity,
  version: 1,
  requestId: r.requestId,
  revision: r.expectedRevision + 1,
  accepted: true,
  stateRevision: (r.expectedStateRevision ?? 0) + 1,
  value: false,
});
test("versioned host methods use an asynchronous identity/revision-aware kernel transport", async () => {
  const requests: HostRequest[] = [];
  const client = new HostClient(
    identity,
    async (r) => {
      requests.push(r);
      return ack(r);
    },
    3,
  );
  await client.registerBinding("x", ["layer.x"]);
  await client.query("evaluated", ["layer.x"], 1.25);
  await client.command("test", { exact: false });
  await client.setSetting("recents", ["synthetic"]);
  await client.getSetting("recents");
  await client.disposeBinding("x");
  expect(requests.map((r) => r.operation.type)).toEqual([
    "binding.register",
    "query",
    "command",
    "settings.set",
    "settings.get",
    "binding.dispose",
  ]);
  expect(requests.map((r) => r.expectedRevision)).toEqual([3, 4, 5, 6, 7, 8]);
  expect(
    requests.every(
      (r) => r.version === 1 && r.generation === 2 && r.realm === "test",
    ),
  ).toBe(true);
  client.retire();
  await expect(client.query("time")).rejects.toThrow("retired");
});
test("host rejects late/wrong/declined acknowledgements and stale publications", async () => {
  for (const change of [
    { generation: 1 },
    { project: "other" },
    { version: 2 },
    { requestId: 99 },
    { revision: 0 },
    { accepted: false, error: "declined" },
  ]) {
    const client = new HostClient(
      identity,
      async (r) => ({ ...ack(r), ...change }) as HostAck,
      3,
    );
    await expect(client.command("test", {})).rejects.toThrow();
  }
  const received: unknown[] = [];
  const client = new HostClient(identity, async (r) => ack(r), 3);
  const unsubscribe = await client.subscribe("selection", "selection", (v) =>
    received.push(v),
  );
  const publication = {
    ...identity,
    version: 1 as const,
    subscription: "selection",
    revision: 5,
    sequence: 1,
    value: ["synthetic"],
  };
  expect(client.publish(publication)).toBe(true);
  expect(client.publish(publication)).toBe(false);
  expect(client.publish({ ...publication, sequence: 2, revision: 4 })).toBe(
    false,
  );
  expect(client.publish({ ...publication, sequence: 2, generation: 1 })).toBe(
    false,
  );
  await unsubscribe();
  expect(client.publish({ ...publication, sequence: 3 })).toBe(false);
  expect(received).toEqual([false, ["synthetic"]]);
});
test("host rejects in-flight ack after retirement", async () => {
  let resolve!: (ack: HostAck) => void;
  let request!: HostRequest;
  const client = new HostClient(identity, (r) => {
    request = r;
    return new Promise((done) => {
      resolve = done;
    });
  });
  const pending = client.query("selection");
  await Promise.resolve();
  client.retire();
  resolve(ack(request));
  await expect(pending).rejects.toThrow("Stale");
});
// Declaration-only regressions: no index signature quietly accepts native capabilities.
const valid: JSX.IntrinsicElements["anchored"] = {
  anchor: { x: 0.25, y: -1 },
  metadata: { exact: false },
};
// @ts-expect-error anchor belongs to anchored
const invalidPrimitive: JSX.IntrinsicElements["div"] = { anchor: 1 };
const invalidUnknown: JSX.IntrinsicElements["path"] = {
  // @ts-expect-error extension metadata has an explicit bag
  madeUpNativeBehavior: true,
};
const invalidRange: JSX.IntrinsicElements["virtual-list"] = {
  // @ts-expect-error ranges contain exactly two offsets
  range: [1, 2, 3],
};
void [valid, invalidPrimitive, invalidUnknown, invalidRange, h];
test("host input serialization never invokes accessors or coerces acceptance", async () => {
  let invoked = 0;
  const client = new HostClient(identity, async (r) => ack(r));
  const operation = {
    type: "command" as const,
    command: "test",
    get args() {
      invoked++;
      return {};
    },
  };
  await expect(client.request(operation)).rejects.toThrow("accessors");
  expect(invoked).toBe(0);
  const malformed = new HostClient(
    identity,
    async (r) => ({ ...ack(r), accepted: "yes" }) as unknown as HostAck,
  );
  await expect(malformed.query("time")).rejects.toThrow();
});
test("native event envelopes fence old generations and reject malformed typed data", async () => {
  let called = 0;
  const root = createRoot(accept, undefined, { generation: 2 });
  await root.render(
    <button
      id="field"
      onClick={() => {
        called++;
      }}
    />,
  );
  const id = field(root).id;
  const plan = {
    revision: 1,
    target: id,
    preventDefault: false,
    deliveries: [{ id, capture: false }],
  };
  await expect(
    root.dispatchNative({ generation: 1, plan, type: "click", detail: {} }),
  ).rejects.toThrow("generation");
  for (const detail of [
    { metaKey: 1 },
    { pointerId: -1 },
    { drag: { types: [], data: {}, effect: "invalid" } },
    { selection: [2, 1] },
  ])
    await expect(
      root.dispatchNative({
        generation: 2,
        plan,
        type: "click",
        detail: detail as never,
      }),
    ).rejects.toThrow();
  expect(called).toBe(0);
  await root.dispose();
});
test("settings acknowledgements use profile revision independently of project revision", async () => {
  const requests: HostRequest[] = [];
  const client = new HostClient(
    identity,
    async (r) => {
      requests.push(r);
      return {
        ...ack(r),
        revision: 7,
        stateRevision: (r.expectedStateRevision ?? 100) + 1,
      };
    },
    7,
  );
  await client.setSetting("recents", []);
  await client.setSetting("recents", ["test"]);
  expect(
    requests.map((r) => [r.expectedRevision, r.expectedStateRevision]),
  ).toEqual([
    [7, 0],
    [7, 1],
  ]);
  const bad = new HostClient(identity, async (r) => ({
    ...ack(r),
    stateRevision: undefined,
  }));
  await expect(bad.setSetting("recents", [])).rejects.toThrow(
    "settings revision",
  );
});
test("retirement refuses new work while allowing an already queued acknowledgement to settle", async () => {
  let resolve!: (ack: { accepted: boolean; revision: number }) => void;
  let first = true;
  const root = createRoot((c) => {
    if (!first) return accept(c);
    first = false;
    return new Promise((done) => {
      resolve = done;
    });
  });
  const render = root.render(<button />);
  await Promise.resolve();
  const dispose = root.dispose();
  expect(() => root.render(<input />)).toThrow("disposed");
  resolve({ accepted: true, revision: 1 });
  await render;
  await dispose;
  expect(root.inspect().disposed).toBe(true);
});
test("lost host ack fences subsequent edits rather than replaying an ambiguous mutation", async () => {
  let submitted = 0;
  const client = new HostClient(identity, async () => {
    submitted++;
    throw new Error("lost ack");
  });
  await expect(client.command("test", {})).rejects.toThrow("lost ack");
  await expect(client.command("test", {})).rejects.toThrow("reset generation");
  expect(submitted).toBe(1);
});
