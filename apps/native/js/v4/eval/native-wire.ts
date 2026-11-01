import { fileURLToPath } from "node:url";

let built = false;
/** Development/test utility only. Both examples use the real Rust validators. */
export function nativeWire() {
  const manifest = fileURLToPath(
    new URL("../../../crates/surface/standalone/Cargo.toml", import.meta.url),
  );
  if (!built) {
    const result = Bun.spawnSync(
      ["cargo", "build", "--locked", "--manifest-path", manifest, "--examples"],
      { stderr: "pipe", stdout: "pipe" },
    );
    if (result.exitCode !== 0)
      throw new Error(new TextDecoder().decode(result.stderr));
    built = true;
  }
  const base = new URL(
    "../../../crates/surface/standalone/target/debug/examples/",
    import.meta.url,
  );
  return {
    surface: fileURLToPath(new URL("wire", base)),
    kit: fileURLToPath(new URL("ui_kit_wire", base)),
  };
}
