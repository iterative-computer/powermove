import { expect, test } from "bun:test";

/** The panels and the runtime type-check as they are built: a type error is a
 * failed test, so every gate that runs `bun test` catches it (a variant the
 * host no longer reads once reached integration unnoticed). */
test("tsc --noEmit finds no errors", () => {
  const root = `${import.meta.dir}/..`;
  const run = Bun.spawnSync(
    [`${root}/node_modules/.bin/tsc`, "--noEmit", "-p", root],
    {
      stdout: "pipe",
      stderr: "pipe",
    },
  );
  expect(`${run.stdout}${run.stderr}`.trim()).toBe("");
  expect(run.exitCode).toBe(0);
}, 180_000);
