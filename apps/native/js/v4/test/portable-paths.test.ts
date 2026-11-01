import { expect, test } from "bun:test";
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

test("candidate eval resolves from paths with spaces and URL delimiters", () => {
  // Canonicalize our own synthetic directory; system temp roots may be symlinks
  // on macOS. No test assumes ownership or spelling of /tmp or /var.
  const directory = realpathSync(mkdtempSync(join(tmpdir(), "pm-v4 # ")));
  try {
    writeFileSync(
      join(directory, "candidate.ts"),
      `export { createRoot } from ${JSON.stringify(new URL("../src/index.ts", import.meta.url).href)};\n`,
    );
    const child = Bun.spawnSync(
      [
        process.execPath,
        "--tsconfig-override",
        fileURLToPath(new URL("../tsconfig.json", import.meta.url)),
        fileURLToPath(new URL("../eval/runner.ts", import.meta.url)),
        "--candidate=./candidate.ts",
        "--group=wild",
      ],
      { cwd: directory },
    );
    expect(child.exitCode, new TextDecoder().decode(child.stderr)).toBe(0);
    expect(JSON.parse(new TextDecoder().decode(child.stdout))).toMatchObject({
      passed: 10,
      failed: 0,
      pending: 0,
    });
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
