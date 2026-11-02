import { beforeAll } from "bun:test";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createRoot, type Commit } from "../../../src";
/** Local synthetic wire recorder; never an application host or model fallback. */
export function recorder() {
  const commits: Commit[] = [];
  const root = createRoot(async (commit) => {
    commits.push(structuredClone(commit));
    return { accepted: true, revision: commit.baseRevision + 1 };
  });
  return { root, commits };
}

let binaries: { model: string; surface: string } | undefined;
export function nativeValidators() {
  if (binaries) return binaries;
  const manifest = fileURLToPath(
    new URL("../../../../../Cargo.toml", import.meta.url),
  );
  const target = resolve(
    process.env.CARGO_TARGET_DIR ??
      fileURLToPath(new URL("../../../../../target", import.meta.url)),
  );
  const result = Bun.spawnSync(
    [
      "cargo",
      "build",
      "--locked",
      "--manifest-path",
      manifest,
      "--target-dir",
      target,
      "-p",
      "pm-panel-timeline",
      "--examples",
    ],
    { stderr: "pipe", stdout: "pipe" },
  );
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  binaries = {
    model: join(target, "debug/examples/v4_timeline_wire"),
    surface: join(target, "debug/examples/v4_timeline_surface_wire"),
  };
  return binaries;
}

// Cold Rust builds (or another workspace Cargo process holding the build lock)
// are setup work, outside Bun's default five-second per-interaction timeout.
beforeAll(() => {
  nativeValidators();
}, 120_000);
