import { fileURLToPath } from "node:url";
import { join, resolve } from "node:path";

let built = false;
/** Development/test utility only. Both examples use the real Rust validators. */
export function nativeWire() {
  const manifest = fileURLToPath(
    new URL("../../../crates/surface/standalone/Cargo.toml", import.meta.url),
  );
  const target = resolve(
    process.env.CARGO_TARGET_DIR ??
      fileURLToPath(
        new URL("../../../crates/surface/standalone/target", import.meta.url),
      ),
  );
  if (!built) {
    const result = Bun.spawnSync(
      [
        "cargo",
        "build",
        "--locked",
        "--manifest-path",
        manifest,
        "--target-dir",
        target,
        "--examples",
      ],
      { stderr: "pipe", stdout: "pipe" },
    );
    if (result.exitCode !== 0)
      throw new Error(new TextDecoder().decode(result.stderr));
    built = true;
  }
  const base = join(target, "debug/examples");
  return {
    surface: join(base, "wire"),
    kit: join(base, "ui_kit_wire"),
    host: join(base, "host_wire"),
  };
}
