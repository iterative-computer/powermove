import { expect, test } from "bun:test";
import { runInNewContext } from "node:vm";
import { fileURLToPath } from "node:url";
test("Effects browser bundle runs without Node/Chromium/global DOM", async () => {
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
    effectsProbe?: Promise<unknown>;
    structuredClone: typeof structuredClone;
  } = { structuredClone };
  runInNewContext(await build.outputs[0].text(), context);
  expect(await context.effectsProbe).toMatchObject({
    title: "Effects",
    rows: 1,
    process: "undefined",
    document: "undefined",
  });
});
