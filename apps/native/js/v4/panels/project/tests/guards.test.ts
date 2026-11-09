import { expect, test } from "bun:test";
import { mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import ts from "typescript";
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
test("panel styles have no visual literals; only kit tokens and structural keywords", async () => {
  const text = await Bun.file(join(import.meta.dir, "../index.tsx")).text();
  const file = ts.createSourceFile(
    "index.tsx",
    text,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const styles: ts.ObjectLiteralExpression[] = [];
  function visit(node: ts.Node) {
    if (
      ts.isJsxAttribute(node) &&
      node.name.getText(file) === "style" &&
      node.initializer &&
      ts.isJsxExpression(node.initializer) &&
      node.initializer.expression &&
      ts.isObjectLiteralExpression(node.initializer.expression)
    )
      styles.push(node.initializer.expression);
    if (
      ts.isVariableDeclaration(node) &&
      ["text", "muted", "target"].includes(node.name.getText(file)) &&
      node.initializer &&
      ts.isObjectLiteralExpression(node.initializer)
    )
      styles.push(node.initializer);
    ts.forEachChild(node, visit);
  }
  visit(file);
  const structural = new Set([
    "flex",
    "row",
    "column",
    "center",
    "end",
    "space-between",
    "nowrap",
    "hidden",
    "ellipsis",
    "tabular-nums",
  ]);
  for (const style of styles)
    for (const p of style.properties) {
      if (!ts.isPropertyAssignment(p)) continue;
      // Flex factors are layout structure, not visuals (the kit validator agrees).
      const flex = ["flexGrow", "flexShrink"].includes(p.name.getText(file));
      expect(ts.isNumericLiteral(p.initializer) && !flex).toBe(false);
      if (ts.isStringLiteral(p.initializer))
        expect(structural.has(p.initializer.text)).toBe(true);
    }
  expect(styles.length).toBeGreaterThan(10);
  expect(text).not.toContain("data-pm-component="); // No invented UI-kit recipe.
});

test("selected rows use the selection-aware status inks (4.5:1 on selected_fill)", () => {
  const missing = { overlay: "Missing", tone: "danger" as const };
  expect(trailingInk(missing, false)).toBe("colors.danger");
  expect(trailingInk(missing, true)).toBe("colors.selected_danger");
  expect(trailingInk(null, false)).toBe("colors.muted");
  expect(trailingInk(null, true)).toBe("colors.selected_secondary");
});
