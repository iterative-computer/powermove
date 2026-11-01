import { fileURLToPath } from "node:url";
// Portable paths from this owned file. No /workspace assumptions, external
// spikes, temp path identity, GUI, media tools or fixtures from another lane.
const workspace = fileURLToPath(new URL("../../../../../", import.meta.url));
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
    new URL("target/", new URL("../../../../../", import.meta.url)).pathname,
  ],
  {
    cwd: workspace,
    stdout: "pipe",
    stderr: "pipe",
  },
);
if (child.exitCode !== 0)
  throw new Error(new TextDecoder().decode(child.stderr));
