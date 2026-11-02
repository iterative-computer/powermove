/** Test-only tooling. The extension entry never imports Node or process APIs. */
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { afterAll } from "bun:test";
import type { Commit } from "../../src/protocol";
let executable: string | undefined;
let temporary: string | undefined;
afterAll(() => {
  if (temporary) rmSync(temporary, { recursive: true, force: true });
});
function nativeAX() {
  if (executable) return executable;
  // preload setup.ts builds these actual library sources with --locked.
  const validator = process.env.PM_UI_KIT_WIRE;
  if (!validator) throw new Error("Missing locked native validator build");
  const deps = join(dirname(dirname(validator)), "deps");
  const built = Bun.spawnSync([
    "cargo",
    "build",
    "--locked",
    "--manifest-path",
    fileURLToPath(
      new URL(
        "../../../../crates/surface/standalone/Cargo.toml",
        import.meta.url,
      ),
    ),
    "--target-dir",
    dirname(dirname(deps)),
    "--examples",
    "--message-format=json",
  ]);
  if (built.exitCode !== 0) throw new Error(built.stderr.toString());
  const artifacts = built.stdout
    .toString()
    .trim()
    .split("\n")
    .map((line) => JSON.parse(line));
  const library = (name: string): string => {
    const artifact = artifacts.find(
      (a) =>
        a.reason === "compiler-artifact" &&
        a.target.name === name &&
        a.target.kind.includes("lib"),
    );
    const path = artifact?.filenames.find((file: string) =>
      file.endsWith(".rlib"),
    );
    if (!path) throw new Error(`Missing locked ${name} artifact`);
    return path;
  };
  temporary = mkdtempSync(join(tmpdir(), "pm-settings-ax-"));
  executable = join(temporary, "settings-ax");
  const result = Bun.spawnSync([
    "rustc",
    "--edition=2024",
    "-D",
    "warnings",
    fileURLToPath(new URL("./validation.rs", import.meta.url)),
    "-L",
    `dependency=${deps}`,
    "--extern",
    `pm_surface=${library("pm_surface")}`,
    "--extern",
    `pm_design=${library("pm_design")}`,
    "--extern",
    `serde_json=${library("serde_json")}`,
    "-o",
    executable,
  ]);
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  return executable;
}
export interface AXNode {
  id: number;
  role: string;
  label: string | null;
  value: string | null;
  help: string | null;
  selected: boolean | null;
  disabled: boolean;
  rowCount: number | null;
  children: number[];
}
export function accessibility(commits: Commit[]): {
  ax: AXNode[];
  focusOrder: number[];
} {
  const result = Bun.spawnSync([nativeAX()], {
    stdin: Buffer.from(commits.map((c) => JSON.stringify(c)).join("\n") + "\n"),
  });
  if (result.exitCode !== 0) throw new Error(result.stderr.toString());
  return JSON.parse(result.stdout.toString().trim().split("\n").at(-1)!);
}
