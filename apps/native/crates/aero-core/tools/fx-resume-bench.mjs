// fx's journal restore on the journal aero_bench measures resume on, so
// both sides of the resume row read the same bytes (Sol's m2 item 8).
//
//   node fx-resume-bench.mjs <fx checkout, built> <journal.json> [samples] > fx-resume.json
//
// <journal.json> is aero_bench's --resume-journal file (the libfx journal
// events after an empty checkpoint, one JSON array). fx opens it the way
// bench-durable.mjs times journal_restore_create_ms: createFxEngine with a
// store whose load() returns it, on fx's native backend. Prints the journal's
// sha256 and the restore times; the checker reads them as $AERO_FX_RESUME.
// Run it under orch/measure.lock, like aero_bench.
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const [fxRoot, journalPath, samplesArg] = process.argv.slice(2);
if (!fxRoot || !journalPath) {
  console.error("usage: node fx-resume-bench.mjs <fx checkout> <journal.json> [samples]");
  process.exit(2);
}
const samples = Number(samplesArg ?? 50);
const { createFxEngine } = await import(pathToFileURL(join(fxRoot, "sdk/node.js")).href);
const bytes = readFileSync(journalPath);
const events = JSON.parse(bytes.toString("utf8"));
const data = new TextEncoder().encode(JSON.stringify({ format: "libfx-journal-v1", events }));
const tools = Array.from({ length: 12 }, (_, i) => ({
  name: `t${i}`,
  description: "x",
  inputSchema: { type: "object" },
  idempotent: true,
  execute: () => "ok",
}));
const options = (persistence) => ({
  backend: "native",
  nativeAddon: join(fxRoot, "zig-out/lib/libfx.node"),
  apiKey: "k",
  gatewayChatUrl: "http://127.0.0.1:9/chat",
  model: "fake/model",
  tools,
  fetch: () => { throw new Error("no fetch"); },
  persistence,
});
const store = () => {
  let cursor = 1;
  return {
    async load() { return { journal: [{ cursor: "1", data }] }; },
    async append() { cursor += 1; return { cursor: String(cursor) }; },
  };
};

const ms = [];
for (let index = 0; index < samples + 3; index += 1) {
  const started = performance.now();
  const engine = await createFxEngine(options(store()));
  const elapsed = performance.now() - started;
  await engine.close();
  if (index >= 3) ms.push(elapsed);
}
ms.sort((a, b) => a - b);
const at = (q) => ms[Math.min(ms.length - 1, Math.floor(q * ms.length))];
console.log(JSON.stringify({
  journal_sha256: createHash("sha256").update(bytes).digest("hex"),
  journal_bytes: bytes.length,
  journal_events: events.length,
  samples: ms.length,
  journal_restore_create_ms: { p50: at(0.5), p90: at(0.9), p99: at(0.99), min: ms[0], max: ms.at(-1) },
}));
