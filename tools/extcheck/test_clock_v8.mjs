// Actual worker regression. Node and Bun are test/build tools only.
import assert from "node:assert/strict";
import { test } from "node:test";
import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, writeFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { CLOCK_SOURCE } from "./clock-source.mjs";

const runner = process.env.PM_EXTCHECK_RUNNER;
assert(runner, "Set PM_EXTCHECK_RUNNER to the built native pm-compat-runner");
const root = await mkdtemp(path.join(os.tmpdir(), "pm-extcheck-clock-"));
try {
  await writeFile(path.join(root, "clock.js"), CLOCK_SOURCE);
  const extension = path.join(root, "extension");
  await mkdir(extension);
  await writeFile(
    path.join(extension, "manifest.json"),
    JSON.stringify({ id: "synthetic-clock", name: "Clock", version: "1.0.0", apiVersion: 3 }),
  );
  await writeFile(
    path.join(extension, "index.ts"),
    `export default function(){
    globalThis.activationClock=[Date.now(),performance.now(),Math.random(),Array.from(crypto.getRandomValues(new Uint8Array(8)))];
  }`,
  );
  const record = () =>
    JSON.parse(
      execFileSync(runner, [], {
        encoding: "utf8",
        input: JSON.stringify({
          clock_fixture: path.join(root, "clock.js"),
          extension,
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
