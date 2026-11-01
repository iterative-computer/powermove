/** Test-only bridge to the real Rust surface, kit and AccessKit projections. */
import { fileURLToPath } from "node:url";
import { expect } from "bun:test";
import type { Commit } from "../../src";
let executable: string | undefined;
export function replay(commits: Commit[], appearance = 0, focus?: number) {
  if (!executable) {
    const native = fileURLToPath(new URL("../../../../", import.meta.url));
    const target = fileURLToPath(
      new URL("../../../../target", import.meta.url),
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
    executable = fileURLToPath(
      new URL("../../../../target/debug/pm-export-panel-wire", import.meta.url),
    );
  }
  const child = Bun.spawnSync([executable], {
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
