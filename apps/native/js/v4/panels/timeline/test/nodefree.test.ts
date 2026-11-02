import { expect, test } from "bun:test";
import { runInNewContext } from "node:vm";
import { fileURLToPath } from "node:url";

test("panel browser bundle renders and drives by id without Node/Chromium/global DOM", async () => {
  const build = await Bun.build({
    entrypoints: [
      fileURLToPath(new URL("./nodefree-fixture.tsx", import.meta.url)),
    ],
    target: "browser",
    format: "iife",
    minify: true,
  });
  expect(build.success).toBe(true);
  const context: {
    structuredClone: typeof structuredClone;
    timelineProbe?: Promise<unknown>;
  } = { structuredClone };
  runInNewContext(await build.outputs[0].text(), context);
  expect(await context.timelineProbe).toEqual({
    actions: [{ type: "play" }],
    globals: ["undefined", "undefined", "undefined", "undefined"],
  });
});
