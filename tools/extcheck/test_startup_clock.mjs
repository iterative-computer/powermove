import assert from "node:assert/strict";
import { test } from "node:test";
import { withBootClock } from "./startup-clock.mjs";

test("startup progresses every realm serially and stops after readiness", async () => {
  let first = 0,
    second = 0,
    concurrent = 0,
    maximum = 0;
  const frame = (increment) => ({
    evaluate: async () => {
      maximum = Math.max(maximum, ++concurrent);
      increment();
      await new Promise((resolve) => setTimeout(resolve, 2));
      concurrent--;
      return null;
    },
  });
  const page = { frames: () => [frame(() => first++), frame(() => second++)] };
  await withBootClock(
    page,
    () => new Promise((resolve) => setTimeout(resolve, 50)),
  );
  assert(first > 0 && second === first);
  assert.equal(maximum, 1);
  const final = first;
  await new Promise((resolve) => setTimeout(resolve, 20));
  assert.equal(first, final);
});

test("timer execution errors cannot be classified as successful startup", async () => {
  const page = {
    frames: () => [{ evaluate: async () => "Synthetic clock failure" }],
  };
  await assert.rejects(
    withBootClock(
      page,
      () => new Promise((resolve) => setTimeout(resolve, 30)),
    ),
    /Synthetic clock failure/,
  );
});

test("a readiness failure retains the underlying failure", async () => {
  const failure = Error("Synthetic readiness failure");
  await assert.rejects(
    withBootClock({ frames: () => [] }, async () => {
      throw failure;
    }),
    (value) => value === failure,
  );
});
