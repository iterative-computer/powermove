import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
// Portable paths from this owned file. No /workspace assumptions, external
// spikes, temp path identity, GUI, media tools or fixtures from another lane.
const workspace = fileURLToPath(new URL("../../../../../", import.meta.url));
const target = resolve(
  process.env.CARGO_TARGET_DIR ??
    fileURLToPath(new URL("../../../../../target/", import.meta.url)),
);
export const inspectorWire = join(target, "debug/examples/inspector_wire");
const child = Bun.spawnSync(
  [
    "cargo",
    "build",
    "--locked",
    "-p",
    "pm-panel-inspector",
    "--example",
    "inspector_wire",
    "--target-dir",
    target,
  ],
  {
    cwd: workspace,
    stdout: "pipe",
    stderr: "pipe",
  },
);
if (child.exitCode !== 0)
  throw new Error(new TextDecoder().decode(child.stderr));
