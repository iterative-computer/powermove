import { expect, test } from "bun:test";
import { runInNewContext } from "node:vm";
import { fixture, initial } from "./fixtures";
import { mountExportPanel } from "./index";

test("worker entry queries/subscribes via the public host service and releases its surface/subscription", async () => {
  const f = fixture();
  const mounted = await mountExportPanel({
    host: f.host,
    anchor: 0,
    subscription: "export.fixture",
    transport: async (c) => ({ accepted: true, revision: c.baseRevision + 1 }),
  });
  expect(f.requests.slice(0, 2).map((r) => r.operation)).toEqual([
    { type: "query", query: "project", channels: ["export"] },
    {
      type: "subscribe",
      subscription: "export.fixture",
      query: "project",
      channels: ["export"],
    },
  ]);
  expect(mounted.root.host.root.children).toHaveLength(0);
  await mounted.key("e", true, true);
  expect(mounted.root.inspect().nodes.length).toBeGreaterThan(1);
  await mounted.key("Escape");
  expect(mounted.root.host.root.children).toHaveLength(0);
  await mounted.dispose();
  expect(f.requests.at(-1)!.operation).toMatchObject({ type: "unsubscribe" });
  expect(await f.publish(initial())).toBe(false);
});
test("whole panel bundles and executes without Node/Chromium/DOM globals", async () => {
  const build = await Bun.build({
    entrypoints: [new URL("./nodefree.ts", import.meta.url).pathname],
    target: "browser",
    format: "iife",
    minify: true,
  });
  expect(build.success).toBe(true);
  const context: {
    exportProbe?: Promise<Record<string, unknown>>;
    structuredClone: typeof structuredClone;
  } = { structuredClone };
  runInNewContext(
    "globalThis.structuredClone = value => JSON.parse(JSON.stringify(value));\n" +
      (await build.outputs[0].text()),
    context,
  );
  const result = await context.exportProbe!;
  expect(result.nodes).toBeGreaterThan(1);
  expect(result).toMatchObject({
    process: "undefined",
    document: "undefined",
    window: "undefined",
    buffer: "undefined",
  });
});
