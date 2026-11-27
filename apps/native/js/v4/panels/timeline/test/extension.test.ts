import { expect, test } from "bun:test";
import { mountTimeline, timelinePanel } from "../extension";
import { HostClient } from "../../../src/host-api";
import { InspectModel } from "./fixtures";
import type { Commit } from "../../../src";

function fixture() {
  const identity = { realm: "timeline", project: "synthetic", generation: 7 };
  const host = new HostClient(identity, async (request) => ({
    ...identity,
    version: 1,
    requestId: request.requestId,
    accepted: true,
    revision: 0,
    value: null,
  }));
  const model = new InspectModel();
  const commits: Commit[] = [];
  const retired: number[] = [];
  const errors: unknown[] = [];
  return {
    model,
    commits,
    retired,
    errors,
    options: {
      host,
      model,
      window: { first: 0, visible: 10 },
      instance: "timeline:7:1",
      transport: async (commit: Commit) => {
        commits.push(structuredClone(commit));
        return { accepted: true, revision: commit.baseRevision + 1 };
      },
      onError(error: unknown) {
        errors.push(error);
      },
      async retireSurface(generation: number) {
        retired.push(generation);
      },
    },
  };
}

test("v4 built-in entry mounts accepted ops, drives by id, and awaits native/realm retirement", async () => {
  const f = fixture();
  const panel = await mountTimeline(f.options);
  expect(timelinePanel).toEqual({ id: "timeline", title: "Timeline" });
  expect(panel.root.inspect().generation).toBe(7);
  const play = panel.root
    .inspect()
    .nodes.find((node) => node.props.id === "timeline:play")!;
  await panel.root.drive(play.id, "click");
  await panel.settled();
  expect(f.model.actions).toEqual([{ type: "play" }]);
  const disposal = panel.dispose();
  expect(panel.dispose()).toBe(disposal);
  await disposal;
  expect(f.model.disposals).toBe(1);
  expect(f.retired).toEqual([7]);
  expect(panel.root.inspect().nodes).toHaveLength(1);
  expect(f.commits.at(-1)!.ops.some((op) => op.op === "remove")).toBe(true);
  expect(f.errors).toEqual([]);
});

test("mount rejection cleans native subscriptions/model and the surface generation", async () => {
  const f = fixture();
  f.options.transport = async () => ({ accepted: false, revision: 0 });
  await expect(mountTimeline(f.options)).rejects.toThrow();
  expect(f.model.disposals).toBe(1);
  expect(f.retired).toEqual([7]);
});

test("retirement errors remain visible while independent surface retirement still runs", async () => {
  const f = fixture();
  f.model.dispose = async () => {
    throw new Error("native cancellation failed");
  };
  const panel = await mountTimeline(f.options);
  await expect(panel.dispose()).rejects.toThrow(
    "Timeline panel retirement failed",
  );
  expect(f.retired).toEqual([7]);
});
