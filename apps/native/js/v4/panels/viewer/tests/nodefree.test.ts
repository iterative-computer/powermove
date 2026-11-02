import { test, expect } from "bun:test";
import { runInNewContext } from "node:vm";
import { fileURLToPath } from "node:url";
test("viewer mounts in a browser-target worker with no Node, Chromium or DOM globals", async () => {
  const result = await Bun.build({
    entrypoints: [
      fileURLToPath(new URL("./nodefree-fixture.tsx", import.meta.url)),
    ],
    target: "browser",
    format: "iife",
    minify: true,
  });
  expect(result.success).toBe(true);
  const context: {
    structuredClone: typeof structuredClone;
    viewerProbe?: Promise<unknown>;
  } = { structuredClone };
  runInNewContext(await result.outputs[0].text(), context);
  expect(await context.viewerProbe).toEqual({
    revision: 1,
    process: "undefined",
    document: "undefined",
    window: "undefined",
    buffer: "undefined",
  });
});
