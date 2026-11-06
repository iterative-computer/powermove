// Actual worker regression. Node and Bun are test/build tools only.
import assert from "node:assert/strict";
import { test } from "node:test";
import { execFileSync } from "node:child_process";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { CLOCK_SOURCE } from "./clock-source.mjs";

const runner = process.env.PM_EXTCHECK_RUNNER;
assert(runner, "Set PM_EXTCHECK_RUNNER to the built native pm-compat-runner");
const root = await mkdtemp(path.join(os.tmpdir(), "pm-extcheck-clock-"));
try {
  const bundle = execFileSync(
    "bun",
    [
      "build",
      fileURLToPath(
        new URL(
          "../../apps/native/crates/compat-runner/tests/dom-bundle.ts",
          import.meta.url,
        ),
      ),
      "--target=browser",
      "--format=iife",
    ],
    { encoding: "utf8" },
  );
  await writeFile(path.join(root, "compat.js"), bundle);
  await writeFile(path.join(root, "clock.js"), CLOCK_SOURCE);
  await writeFile(
    path.join(root, "extension.js"),
    `var ExtCheckModule={default(){
    globalThis.activationClock=[Date.now(),performance.now(),Math.random(),Array.from(crypto.getRandomValues(new Uint8Array(8)))];
  }}`,
  );
  const record = () =>
    JSON.parse(
      execFileSync(runner, [], {
        encoding: "utf8",
        input: JSON.stringify({
          compat_bundle: path.join(root, "compat.js"),
          clock_fixture: path.join(root, "clock.js"),
          module: path.join(root, "extension.js"),
          manifest: { id: "synthetic-clock", apiVersion: 3 },
          expressions: [
            "activationClock",
            "(()=>{globalThis.events=[];setTimeout(()=>{events.push(['timer',performance.now()]);queueMicrotask(()=>events.push(['microtask',performance.now()]))},8);requestAnimationFrame(t=>events.push(['frame',t]));return true})()",
            "__extcheckClock.advance(16).then(()=>events)",
            "(()=>{PM.play();return [PM.time,PM.playing]})()",
            "__extcheckClock.advance(16).then(()=>[PM.time,PM.playing])",
            "(()=>{PM.pause();return [PM.time,PM.playing]})()",
            "__compat.loader.dispose().then(()=>true)",
          ],
        }),
        timeout: 30000,
      }),
    );
  const recordings = Array.from({ length: 3 }, record);
  test("three actual V8/DOM/transport recordings are byte-for-byte repeatable", () => {
    assert(
      recordings.every(
        (value) => JSON.stringify(value) === JSON.stringify(recordings[0]),
      ),
    );
  });
  test("fixture precedes extension activation and controls the production DOM frame scheduler", () => {
    const value = recordings[0];
    assert.equal(value.activation.ok, true);
    assert.deepEqual(value.results[0].slice(0, 2), [1600000060000, 60000]);
    assert.deepEqual(value.results[2], [
      ["timer", 60008],
      ["microtask", 60008],
      ["frame", 60016],
    ]);
    assert.deepEqual(value.results.slice(3), [
      [0, true],
      [0.016, true],
      [0, false],
      true,
    ]);
    assert.deepEqual(value.diagnostics.missing, []);
  });
  test("component recordings never claim WebKit paint or full-recorder acceptance", () => {
    assert.equal(recordings[0].gates.webkit_panels, false);
    assert.equal(recordings[0].gates.full_recorder, false);
  });
} finally {
  await rm(root, { recursive: true, force: true });
}
