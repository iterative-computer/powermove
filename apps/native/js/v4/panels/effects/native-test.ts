import type { Commit } from "../../src";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
function buildProbe() {
  const manifest = fileURLToPath(
    new URL("./conformance/Cargo.toml", import.meta.url),
  );
  const target = resolve(
    process.env.CARGO_TARGET_DIR ??
      fileURLToPath(new URL("./conformance/target", import.meta.url)),
  );
  const build = Bun.spawnSync(
    [
      "cargo",
      "build",
      "--locked",
      "--manifest-path",
      manifest,
      "--target-dir",
      target,
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  if (build.exitCode !== 0) throw new Error(build.stderr.toString());
  return `${target}/debug/pm-panel-effects-conformance`;
}
// Build during module setup, outside an individual test's timeout. Cold Rust
// compilation on either macOS or Linux is not panel interaction time.
const binary = buildProbe();
export function replay(commits: readonly Commit[]) {
  const result = Bun.spawnSync([binary], {
    stdin: new TextEncoder().encode(
      commits.map((commit) => JSON.stringify(commit)).join("\n") + "\n",
    ),
    stdout: "pipe",
    stderr: "pipe",
  });
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  return result.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
}
