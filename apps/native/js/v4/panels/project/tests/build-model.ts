/** Development/test only. Production bundles never import this module. */
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
let cached: { model: string; inspect: string } | undefined;
export function binaries() {
  if (cached) return cached;
  const manifest = fileURLToPath(
    new URL("../native/Cargo.toml", import.meta.url),
  );
  const targetRoot = resolve(
    process.env.CARGO_TARGET_DIR ??
      fileURLToPath(new URL("../../../../../target/", import.meta.url)),
  );
  const result = Bun.spawnSync(
    [
      "cargo",
      "build",
      "--locked",
      "--manifest-path",
      manifest,
      "--bins",
      "--target-dir",
      targetRoot,
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  if (result.exitCode) throw new Error(result.stderr.toString());
  const suffix = process.platform === "win32" ? ".exe" : "";
  const target = join(targetRoot, "debug");
  return (cached = {
    model: join(target, `project-model-host${suffix}`),
    inspect: join(target, `project-inspect-wire${suffix}`),
  });
}
if (import.meta.main) console.log(binaries());
