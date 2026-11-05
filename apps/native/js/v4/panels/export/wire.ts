/** Test-only bridge to the real Rust surface, kit and AccessKit projections. */
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { beforeAll, expect } from "bun:test";
import type { Commit } from "../../src";
let executable: string | undefined;
function buildNative() {
  if (!executable) {
    const native = fileURLToPath(new URL("../../../../", import.meta.url));
    const target = resolve(
      process.env.CARGO_TARGET_DIR ??
        fileURLToPath(new URL("../../../../target", import.meta.url)),
    );
    const build = Bun.spawnSync(
      [
        "cargo",
        "build",
        "--locked",
        "-p",
        "pm-export-panel-wire",
        "--target-dir",
        target,
      ],
      { cwd: native },
    );
    if (build.exitCode !== 0) throw new Error(build.stderr.toString());
    executable = join(target, "debug/pm-export-panel-wire");
  }
  return executable;
}
// Native compilation is setup, outside the UI interaction timeout.
beforeAll(() => {
  buildNative();
}, 120_000);
export function replay(commits: Commit[], appearance = 0, focus?: number) {
  const child = Bun.spawnSync([buildNative()], {
    stdin: Buffer.from(
      commits
        .map((commit) => JSON.stringify({ commit, appearance, focus }))
        .join("\n") + "\n",
    ),
  });
  expect(child.exitCode).toBe(0);
  return child.stdout
    .toString()
    .trim()
    .split("\n")
    .map((l) => JSON.parse(l));
}
