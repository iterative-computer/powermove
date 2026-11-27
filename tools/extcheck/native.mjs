// Private diagnostics are consumed by run.py; no substitute renderer is accepted as WebKit.
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { referenceRevision } from "./references.mjs";
import { CLOCK_FIXTURE, CLOCK_START } from "./clock-source.mjs";
import { fileURLToPath } from "node:url";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const request = JSON.parse(Buffer.concat(chunks).toString());
const out = request.output;
const header = (request, manifest) => ({
  scenario: request.case,
  synthetic: true,
  role: request.case.startsWith("store-") ? "store" : "trusted",
  apiVersions: [manifest.apiVersion ?? 1],
  driver: "extcheck-native",
  input: "scripted-interactions",
  eventScope: "synchronous-calls",
  input_sha256: request.input_sha256,
  script_sha256: request.script_sha256,
});
const status = {
  status: "blocked",
  reason: "native-extension-compile-failed",
  case: request.case,
  input_sha256: request.input_sha256,
  script_sha256: request.script_sha256,
  clock_fixture: CLOCK_FIXTURE,
};
try {
  const revision = execFileSync(
    "git",
    ["-C", process.argv[2], "rev-parse", "HEAD"],
    { encoding: "utf8" },
  ).trim();
  if (revision !== referenceRevision(request.reference_revision)) {
    status.reason = "native-reference-revision-mismatch";
    throw Error("Compiler checkout does not match the requested oracle");
  }
  status.compiler_revision = revision;
  execFileSync("bun", [fileURLToPath(new URL("./bundle.ts", import.meta.url)), out], {
    stdio: ["ignore", "pipe", "pipe"],
  });
  const manifest = JSON.parse(
    await readFile(path.join(request.extension, "manifest.json"), "utf8"),
  );
  // Store source runs in its confined helper only, never through a trusted facade,
  // so its facade calls are not recorded yet. Activation there is still measured.
  if (request.case.startsWith("store-")) {
    status.reason = "native-store-activation-failed";
    const reply = JSON.parse(
      execFileSync(process.argv[3], [], {
        input: JSON.stringify({
          store: true,
          work: path.join(out, "store"),
          extension: request.extension,
          release: request.store?.releaseId,
        }),
        encoding: "utf8",
        timeout: 60000,
        maxBuffer: 16 * 1024 * 1024,
      }),
    );
    await writeFile(path.join(out, "probe.json"), JSON.stringify(reply));
    status.activated = reply.activation.ok === true;
    if (status.activated) status.reason = "native-store-trace-driver-unavailable";
  } else {
    status.reason = "native-runner-failed";
    const reply = JSON.parse(
      execFileSync(process.argv[3], [], {
        input: JSON.stringify({
          clock_fixture: path.join(out, "clock.js"),
          extension: request.extension,
          initial: JSON.parse(
            await readFile(path.join(out, "initial.json"), "utf8"),
          ),
          driver: request.trace_only ? path.join(out, "trace.js") : undefined,
          expressions: request.trace_only
            ? [
                `__extcheckDriver.start(${JSON.stringify(header(request, manifest))},${CLOCK_START},${JSON.stringify(manifest.id)})`,
                ...request.script.steps.map(
                  (step, index) =>
                    `__extcheckDriver.step(${index},${JSON.stringify(step)})`,
                ),
                "__extcheckDriver.finish()",
              ]
            : ["__compat.loader.dispose().then(()=>true)"],
        }),
        encoding: "utf8",
        timeout: 60000,
        maxBuffer: 256 * 1024 * 1024,
      }),
    );
    await writeFile(path.join(out, "probe.json"), JSON.stringify(reply));
    status.logic_executed = true;
    status.activated = reply.activation.ok === true;
    status.reason = reply.activation.ok
      ? "native-webkit-script-adapter-unavailable"
      : "native-activation-needs-backend";
    if (request.trace_only && reply.activation.ok) {
      const { trace, interactions } = reply.results.at(-1);
      status.boot_clock = reply.results[0];
      await writeFile(path.join(out, "trace.jsonl"), trace);
      await writeFile(
        path.join(out, "interactions.json"),
        JSON.stringify(interactions),
      );
      // Trace-only: logic ran natively, but no panel or canvas was on screen.
      status.path = { logic: "runtime-v8" };
      status.status = "captured";
      delete status.reason;
    }
  }
} catch (error) {
  const diagnostic = String(error.stderr ?? error.message ?? "");
  status.compile_reason = diagnostic.includes("FROZEN_MANIFEST_INVALID")
    ? "invalid-reference-manifest"
    : diagnostic.includes("Import escapes extension directory")
      ? "import-outside-extension"
      : diagnostic.includes("Unsupported bare import")
        ? "external-runtime-import"
        : diagnostic.includes("Could not resolve")
          ? "unresolved-import"
          : diagnostic.includes("window is not defined") ||
              diagnostic.includes("matchMedia")
            ? "browser-capability-at-load"
            : "compiler-or-loader-error";
  console.error(error);
} finally {
  await writeFile(path.join(out, "capture.json"), JSON.stringify(status));
}
