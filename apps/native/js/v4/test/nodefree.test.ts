import { fileURLToPath } from "node:url";
import { test, expect } from "bun:test";
import { runInNewContext } from "node:vm";
test("browser-target runtime bundle mounts and updates without Node, Chromium or global document", async () => {
  const build = await Bun.build({
    entrypoints: [
      fileURLToPath(new URL("./nodefree-fixture.ts", import.meta.url)),
    ],
    target: "browser",
    format: "iife",
    minify: true,
  });
  expect(build.success).toBe(true);
  const context: {
    nativeProbe?: Promise<unknown>;
    structuredClone: typeof structuredClone;
  } = { structuredClone };
  runInNewContext(await build.outputs[0].text(), context);
  expect(await context.nativeProbe).toEqual({
    text: "17",
    revision: 2,
    process: "undefined",
    document: "undefined",
  });
});
