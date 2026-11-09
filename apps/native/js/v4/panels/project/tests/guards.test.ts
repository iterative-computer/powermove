import { expect, test } from "bun:test";
import { mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { trailingInk } from "../index";
test("panel browser bundle mounts without Node or DOM globals", async () => {
  const dir = mkdtempSync(join(realpathSync(tmpdir()), "pm-project-bundle-"));
  try {
    const build = await Bun.build({
      entrypoints: [join(import.meta.dir, "nodefree-fixture.tsx")],
      target: "browser",
      format: "esm",
      outdir: dir,
    });
    expect(build.success).toBe(true);
    const run = Bun.spawnSync([process.execPath, build.outputs[0].path], {
      stdout: "pipe",
      stderr: "pipe",
    });
    expect(run.stderr.toString()).toBe("");
    expect(run.exitCode).toBe(0);
    expect(run.stdout.toString()).toContain(
      "project panel without Node/DOM globals",
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});
test("selected rows use the selection-aware status inks (4.5:1 on selected_fill)", () => {
  const missing = { overlay: "Missing", tone: "danger" as const };
  expect(trailingInk(missing, false)).toBe("colors.danger");
  expect(trailingInk(missing, true)).toBe("colors.selected_danger");
  expect(trailingInk(null, false)).toBe("colors.muted");
  expect(trailingInk(null, true)).toBe("colors.selected_secondary");
});
