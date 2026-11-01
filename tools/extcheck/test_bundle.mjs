// End-to-end build-tool -> actual V8 CLI test with an original public synthetic extension.
import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
const [reference, runner] = process.argv.slice(2);
assert(
  reference && runner,
  "Pass the prepared reference checkout and native runner",
);
const root = await mkdtemp("/tmp/pm-extcheck/bundle-test-");
try {
  const input = path.join(root, "input"),
    out = path.join(root, "output");
  await mkdir(input);
  await mkdir(out);
  // No entry property: the reference registry's default is index.ts.
  await writeFile(
    path.join(input, "manifest.json"),
    JSON.stringify({
      id: "synthetic-build",
      name: "Synthetic",
      version: "1.0.0",
      apiVersion: 3,
    }),
  );
  await writeFile(
    path.join(input, "index.ts"),
    `export default function(api){api.storage.set('value',17);api.commands.register({id:'synthetic',label:'Synthetic',run(){return api.storage.get('value')}})}`,
  );
  execFileSync(
    "bun",
    [
      fileURLToPath(new URL("./bundle.ts", import.meta.url)),
      reference,
      input,
      out,
    ],
    { stdio: "pipe" },
  );
  const result = JSON.parse(
    execFileSync(runner, [], {
      encoding: "utf8",
      input: JSON.stringify({
        compat_bundle: path.join(out, "compat.js"),
        module: path.join(out, "extension.js"),
        manifest: JSON.parse(
          await readFile(path.join(out, "manifest.json"), "utf8"),
        ),
        expressions: [
          "__extcheck.realm.kernel.commands.get('synthetic').run()",
          "__extcheck.dispose()",
        ],
      }),
    }),
  );
  assert.equal(result.logic, "runtime-v8");
  assert.equal(result.activation.ok, true);
  assert.deepEqual(result.results, [17, true]);
  assert.deepEqual(result.diagnostics.missing, []);
  assert.equal(result.gates.webkit_panels, false);
  console.log(
    "PASS: default entry, compiled module global, shared compat+DOM bundle and actual V8 activation",
  );
} finally {
  await rm(root, { recursive: true, force: true });
}
