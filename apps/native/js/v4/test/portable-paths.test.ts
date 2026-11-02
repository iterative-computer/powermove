import { expect, test } from "bun:test";
import { mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

test("candidate eval resolves from paths with spaces and URL delimiters", () => {
  // Keep the system temp root's spelling: macOS may resolve it through a symlink.
  const directory = mkdtempSync(join(tmpdir(), "pm-v4 # "));
  try {
    const candidate = join(directory, "candidate # %.ts");
    writeFileSync(
      candidate,
      `export { createRoot } from ${JSON.stringify(new URL("../src/index.ts", import.meta.url).href)};\n`,
    );
    const alias = join(directory, "linked # %.ts");
    symlinkSync(candidate, alias);
    for (const path of [
      "./candidate # %.ts",
      alias,
      pathToFileURL(alias).href,
    ]) {
      const child = Bun.spawnSync(
        [
          process.execPath,
          "--tsconfig-override",
          fileURLToPath(new URL("../tsconfig.json", import.meta.url)),
          fileURLToPath(new URL("../eval/runner.ts", import.meta.url)),
          `--candidate=${path}`,
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
    }
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
});
