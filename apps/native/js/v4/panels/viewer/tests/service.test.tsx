import { test, expect } from "bun:test";
import { HostClient, type HostRequest } from "../../../src";
import { ViewerService } from "../service";
import { ViewerPanel } from "../panel";
import { mountViewer } from "../index";
import { snapshot, type ViewerSnapshot } from "../contract";
import { ack, fixture, modelWorker, recorder } from "./helpers";
const identity = { realm: "viewer-test", generation: 1, project: "p" };

test("UI -> v4 host data -> Rust model -> actual kernel ack -> publication", async () => {
  const native = await modelWorker();
  const requests: HostRequest[] = [];
  const initial = fixture();
  initial.model = native.model;
  const host = new HostClient(identity, async (request) => {
    requests.push(request);
    if (request.operation.type === "subscribe")
      return ack(request, initial as any);
    if (request.operation.type === "command") {
      expect(request.operation.command).toBe("viewer.chrome.drive");
      const result = await native.request(request.operation.args);
      if (!result.accepted)
        return { ...ack(request), accepted: false, error: result.error };
      initial.model = result.model;
      return ack(request, result.model);
    }
    return ack(request);
  });
  const service = new ViewerService(host);
  const { root } = recorder();
  let state!: ViewerSnapshot;
  let pending = Promise.resolve();
  const actions = {
    open() {},
    drive(element: string, argument?: any) {
      pending = service.drive(element, argument).then(() => {});
    },
  };
  try {
    await service.start("viewer", (value) => {
      state = value;
    });
    expect(requests[0].operation).toEqual({
      type: "subscribe",
      subscription: "viewer",
      query: "project",
      channels: ["viewer.chrome"],
    });
    await root.render(<ViewerPanel state={state} actions={actions} />);
    const zoom = root
      .inspect()
      .nodes.find((n) => n.props.id === "viewer.zoom")!;
    await root.drive(zoom.id, "change", { value: 2 });
    await pending;
    // Acknowledged intent alone does not optimistically alter the observed UI.
    expect(service.observed!.model!.fit).toBe(true);
    expect(service.observed!.model!.shownZoom).toBeCloseTo(0.54);
    let sequence = 0;
    const publish = () =>
      host.publish({
        ...identity,
        version: 1,
        subscription: "viewer",
        revision: 0,
        sequence: ++sequence,
        value: initial as any,
      });
    expect(publish()).toBe(true);
    expect(state.model!.fit).toBe(false);
    expect(state.model!.shownZoom).toBe(2);
    await root.render(<ViewerPanel state={state} actions={actions} />);
    const quality = root
      .inspect()
      .nodes.find((n) => n.props.id === "viewer.quality")!;
    await root.drive(quality.id, "change", { value: "0.25" });
    await pending;
    expect(publish()).toBe(true);
    expect(state.model!.quality).toBe("0.25");
    expect(state.model!.effectiveQuality).toBe(0.25);
    // Replaying a publication's stale ticket cannot overwrite the model.
    await expect(
      host.command("viewer.chrome.drive", {
        ticket: "1",
        element: "viewer.fit",
        argument: null,
      }),
    ).rejects.toThrow("Stale");
    expect(service.observed!.model!.quality).toBe("0.25");
    await service.stop();
    await service.stop();
    expect(
      requests.filter((r) => r.operation.type === "unsubscribe"),
    ).toHaveLength(1);
    expect(publish()).toBe(false);
    await expect(service.drive("viewer.fit")).rejects.toThrow("unavailable");
  } finally {
    await root.dispose();
    await service.stop();
    await native.close();
  }
});

test("invalid and foreign publications preserve last observed state", async () => {
  const host = new HostClient(identity, async (r) => ack(r, fixture() as any));
  const service = new ViewerService(host);
  await service.start("viewer", () => {});
  const before = service.observed;
  for (const value of [
    { ...fixture(), model: { ...fixture().model, project: "other" } },
    { ...fixture(), model: { ...fixture().model, shownZoom: 100 } },
  ]) {
    expect(() =>
      host.publish({
        ...identity,
        version: 1,
        subscription: "viewer",
        revision: 0,
        sequence: value.model.project === "other" ? 1 : 2,
        value: value as any,
      }),
    ).toThrow();
    expect(service.observed).toEqual(before);
  }
  const copy = service.observed!;
  copy.model!.name = "mutated";
  expect(service.observed!.model!.name).toBe("Launch Film");
  await service.stop();
});

test("stop during subscribe waits for unsubscribe and drops late initial delivery", async () => {
  let reply!: (value: any) => void;
  let subscribe!: HostRequest;
  const requests: HostRequest[] = [];
  const host = new HostClient(identity, (r) => {
    requests.push(r);
    if (r.operation.type === "subscribe") {
      subscribe = r;
      return new Promise((resolve) => {
        reply = resolve;
      });
    }
    return Promise.resolve(ack(r));
  });
  const service = new ViewerService(host);
  let deliveries = 0;
  const start = service.start("viewer", () => {
    deliveries++;
  });
  await Promise.resolve();
  const stop = service.stop();
  reply(ack(subscribe, fixture() as any));
  await Promise.all([start, stop]);
  expect(deliveries).toBe(0);
  expect(requests.map((r) => r.operation.type)).toEqual([
    "subscribe",
    "unsubscribe",
  ]);
  await expect(service.start("again", () => {})).rejects.toThrow("lifecycle");
});

test("mount renders initial publication, reports rejected model actions and retires surface", async () => {
  const errors: unknown[] = [];
  const host = new HostClient(identity, async (r) =>
    r.operation.type === "command"
      ? { ...ack(r), accepted: false, error: "stale model" }
      : ack(r, fixture() as any),
  );
  let retired = 0;
  const mounted = await mountViewer({
    host,
    subscription: "viewer",
    transport: async (c) => ({ accepted: true, revision: c.baseRevision + 1 }),
    rootOptions: {
      generation: 1,
      retire: async () => {
        retired++;
      },
    },
    open() {},
    onError: (e) => {
      errors.push(e);
    },
  });
  const zoom = mounted.root
    .inspect()
    .nodes.find((n) => n.props.id === "viewer.zoom")!;
  await mounted.root.drive(zoom.id, "change", { value: "fit" });
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
  expect(errors.some((e) => String(e).includes("stale model"))).toBe(true);
  expect(mounted.service.observed!.model!.fit).toBe(true);
  await mounted.dispose();
  expect(retired).toBe(1);
});

test("contract rejects malformed state atomically without erasing authored names", () => {
  for (const change of [
    (s: any) => {
      s.model.ticket = "01";
    },
    (s: any) => {
      s.model.selectionCount = -1;
    },
    (s: any) => {
      s.model.shownZoom = Infinity;
    },
    (s: any) => {
      s.model.quality = "draft";
    },
    (s: any) => {
      s.host.frame.resource = 2;
    },
    (s: any) => {
      s.host.source = {};
    },
    (s: any) => {
      s.host.presentation = "invented";
    },
    (s: any) => {
      s.host.frame.failed = null;
    },
  ]) {
    const s = fixture();
    change(s);
    expect(() => snapshot(s)).toThrow();
  }
  const s = fixture();
  s.model!.name = "";
  s.model!.ticket = "18446744073709551615";
  expect(snapshot(s).model!.name).toBe("");
  expect(snapshot(s).model!.ticket).toBe("18446744073709551615");
  const none = fixture();
  none.model = null;
  expect(snapshot(none).model).toBeNull();
});
