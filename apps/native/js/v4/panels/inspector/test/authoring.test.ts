import { expect, test } from "bun:test";
import ts from "typescript";
import vm from "node:vm";
import { fileURLToPath } from "node:url";

test("panel browser bundle mounts with no Node, DOM or Chromium globals", async () => {
  const build = await Bun.build({
    entrypoints: [
      fileURLToPath(new URL("./nodefree-fixture.tsx", import.meta.url)),
    ],
    target: "browser",
    format: "iife",
  });
  expect(build.success).toBe(true);
  const context = vm.createContext({
    structuredClone,
    TextEncoder,
    TextDecoder,
    setTimeout,
    clearTimeout,
    queueMicrotask,
  });
  vm.runInContext(await build.outputs[0].text(), context);
  expect(await context.inspectorResult).toEqual({
    title: "Properties",
    value: 100,
    controls: 1,
  });
  expect(
    vm
      .runInContext(
        "[typeof process, typeof Buffer, typeof window, typeof document]",
        context,
      )
      .join(","),
  ).toBe("undefined,undefined,undefined,undefined");
});

test("view visuals are tokens or structural keywords, with no primitive control substitutes", async () => {
  const source = await Bun.file(new URL("../view.tsx", import.meta.url)).text();
  const ast = ts.createSourceFile(
    "view.tsx",
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const forbidden: string[] = [];
  const visual = new Set([
    "width",
    "height",
    "minWidth",
    "minHeight",
    "maxWidth",
    "maxHeight",
    "padding",
    "paddingLeft",
    "paddingRight",
    "paddingTop",
    "paddingBottom",
    "gap",
    "borderRadius",
    "color",
    "background",
    "fontSize",
    "fontWeight",
    "symbolSize",
    "duration",
  ]);
  const walk = (node: ts.Node) => {
    if (
      ts.isPropertyAssignment(node) &&
      visual.has(node.name.getText(ast).replaceAll('"', ""))
    ) {
      const value = node.initializer;
      if (
        ts.isNumericLiteral(value) ||
        (ts.isStringLiteral(value) &&
          (value.text.startsWith("#") || /^\d/.test(value.text)))
      )
        forbidden.push(node.getText(ast));
    }
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const tag = node.tagName.getText(ast);
      if (
        ["button", "input", "textarea", "canvas", "webview", "Glass"].includes(
          tag,
        )
      )
        forbidden.push(tag);
    }
    ts.forEachChild(node, walk);
  };
  walk(ast);
  expect(forbidden).toEqual([]);
});
