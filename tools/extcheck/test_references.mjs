import assert from "node:assert/strict";
import { test } from "node:test";
import { execFileSync } from "node:child_process";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { REFERENCES, referenceRevision } from "./references.mjs";
import { audit } from "./source-audit.mjs";

test("current-main is the default and legacy pairing is an explicit choice", () => {
  assert.equal(referenceRevision(), "c7f46a05641b23944d3692c9d80f9677c7e1ef1d");
  assert.equal(
    referenceRevision(REFERENCES.legacy),
    "07ed5fa17ee640659cb075f9cad1af2095011d60",
  );
  assert.throws(() => referenceRevision("main"));
  assert.throws(() => referenceRevision("unreviewed"));
});
test("X carry rows, compiler and synthetic project factory match the two pinned sources exactly", () => {
  const evidence = audit();
  assert.equal(evidence.native_acceptance, false);
  assert(
    Object.values(evidence.rows)
      .flat()
      .every((row) => row.equal),
  );
  assert.equal(Object.values(evidence.rows).flat().length, 8);
});
test("native driver refuses a legacy compiler under a current-main request before activation", async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), "pm-extcheck-reference-"));
  try {
    const checkout = path.join(root, "checkout"),
      output = path.join(root, "output");
    await mkdir(checkout);
    await mkdir(output);
    execFileSync("git", ["init", "--quiet", checkout]);
    // A synthetic detached HEAD exercises the driver's revision fence. No
    // source or commit is invented and no compiler/extension may run.
    await writeFile(path.join(checkout, ".git/HEAD"), REFERENCES.legacy + "\n");
    execFileSync(
      process.execPath,
      [
        fileURLToPath(new URL("./native.mjs", import.meta.url)),
        checkout,
        "missing-runner",
      ],
      {
        input: JSON.stringify({
          case: "local-corpus-0001",
          output,
          reference_revision: REFERENCES.current,
          input_sha256: "synthetic",
          script_sha256: "synthetic",
        }),
        stdio: ["pipe", "pipe", "pipe"],
      },
    );
    const status = JSON.parse(
      await readFile(path.join(output, "capture.json"), "utf8"),
    );
    assert.equal(status.status, "blocked");
    assert.equal(status.reason, "native-reference-revision-mismatch");
    assert.equal(status.logic_executed, undefined);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
