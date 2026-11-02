/** Development/test only. Production bundles never import this module. */
import { fileURLToPath } from "node:url";
let cached: { model: string; inspect: string } | undefined;
export function binaries() {
  if (cached) return cached;
  const manifest = fileURLToPath(
    new URL("../native/Cargo.toml", import.meta.url),
  );
  const targetRoot = new URL("../../../../../target/", import.meta.url);
  const result = Bun.spawnSync(
    [
      "cargo",
      "build",
      "--locked",
      "--manifest-path",
      manifest,
      "--bins",
      "--target-dir",
      fileURLToPath(targetRoot),
    ],
    { stdout: "pipe", stderr: "pipe" },
  );
  if (result.exitCode) throw new Error(result.stderr.toString());
  const suffix = process.platform === "win32" ? ".exe" : "";
  const target = new URL("debug/", targetRoot);
  return (cached = {
    model: fileURLToPath(new URL(`project-model-host${suffix}`, target)),
    inspect: fileURLToPath(new URL(`project-inspect-wire${suffix}`, target)),
  });
}
if (import.meta.main) console.log(binaries());
