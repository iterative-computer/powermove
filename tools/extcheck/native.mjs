// Private diagnostics are consumed by run.py; no substitute renderer is accepted as WebKit.
import { readFile, writeFile } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { referenceRevision } from "./references.mjs";
import { CLOCK_FIXTURE } from "./clock-source.mjs";
import { fileURLToPath } from "node:url";
const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const request = JSON.parse(Buffer.concat(chunks).toString());
const out = request.output;
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
  execFileSync(
    "bun",
    [
      fileURLToPath(new URL("./bundle.ts", import.meta.url)),
      process.argv[2],
      request.extension,
      out,
    ],
    { stdio: ["ignore", "pipe", "pipe"] },
  );
  const manifest = JSON.parse(
    await readFile(path.join(out, "manifest.json"), "utf8"),
  );
  // Store source is compiled but never executed through a trusted facade. Its permissioned
  // signed native installation and resource dispatcher must be connected before
  // public archive bytes may enter the confined helper.
  if (request.case.startsWith("store-")) {
    status.reason = "native-store-release-admission-unavailable";
  } else {
    status.reason = "native-runner-failed";
    const reply = JSON.parse(
      execFileSync(process.argv[3], [], {
        input: JSON.stringify({
          clock_fixture: path.join(out, "clock.js"),
          compat_bundle: path.join(out, "compat.js"),
          module: path.join(out, "extension.js"),
          manifest,
          initial: JSON.parse(
            await readFile(path.join(out, "initial.json"), "utf8"),
          ),
          expressions: ["__extcheck.dispose()"],
        }),
        encoding: "utf8",
        timeout: 60000,
      }),
    );
    await writeFile(path.join(out, "probe.json"), JSON.stringify(reply));
    status.logic_executed = true;
    status.activated = reply.activation.ok === true;
    status.reason = reply.activation.ok
      ? "native-webkit-script-adapter-unavailable"
      : "native-activation-needs-backend";
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
