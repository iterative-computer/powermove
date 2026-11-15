/** Reproducible, public v1.2.0 extraction. Run from this package with Bun. */
import ts from "typescript";
import { format } from "prettier";
import { join } from "node:path";
const root = join(import.meta.dir, "../../../../..");
const reference = "07ed5fa17ee640659cb075f9cad1af2095011d60";
const base = "apps/desktop/src/renderer/src/kernel/";
const sources: Record<string, string> = {
  "ease-runtime": "apps/desktop/src/renderer/src/legacy/core/easing.ts",
  "store-api": "apps/desktop/src/renderer/sandbox/shim-api.ts",
  "sandbox-rpc": "apps/desktop/src/shared/sandbox-rpc.ts",
  icons: "apps/desktop/src/renderer/src/legacy/core/util.ts",
  "pm-facades": "apps/desktop/src/renderer/src/kernel/install.ts",
  "layout-model": "apps/desktop/src/renderer/src/layout/model.ts",
  "side-scroll": "apps/desktop/src/renderer/src/layout/side-scroll.ts",
  drop: "apps/desktop/src/renderer/src/fx/drop.ts",
  binding: "apps/desktop/src/renderer/src/controls/binding.ts",
};
export async function frozen(name: string): Promise<string> {
  const process = Bun.spawn(
    ["git", "show", `${reference}:${sources[name] ?? base + name + ".ts"}`],
    { cwd: root, stdout: "pipe", stderr: "pipe" },
  );
  const source = await new Response(process.stdout).text();
  if (await process.exited) throw new Error(`Cannot read frozen ${name}`);
  let input = source;
  if (name === "icons") {
    const tree = ts.createSourceFile(
      "util.ts",
      source,
      ts.ScriptTarget.Latest,
      true,
    );
    const install = tree.statements.find(
      (node): node is ts.FunctionDeclaration =>
        ts.isFunctionDeclaration(node) && node.name?.text === "install",
    );
    const statements = install?.body?.statements.filter(
      (node) =>
        ts.isExpressionStatement(node) &&
        ts.isBinaryExpression(node.expression) &&
        ["PM.svg", "PM.icon", "PM.ICONS"].includes(
          node.expression.left.getText(tree),
        ),
    );
    if (statements?.length !== 3) throw Error("Unsupported frozen icon source");
    input = `/* Phosphor Icons 2.1.1, regular weight. MIT licensed. */\nexport function installIcons(PM:any,window:any){${statements.map((node) => node.getText(tree)).join("\n")}}`;
  }
  if (name === "pm-facades") {
    const selected = new Set([
      "makeAnim",
      "makeModel",
      "makeSelection",
      "makeGroups",
      "makeTransport",
      "makeMedia",
      "makeRender",
      "makeUIState",
      "makeUtil",
      "makeEase",
      "makeSpace3D",
      "makeDnd",
      "makeWorkspace",
    ]);
    const tree = ts.createSourceFile(
      "install.ts",
      source,
      ts.ScriptTarget.Latest,
      true,
    );
    input = tree.statements
      .filter(
        (node): node is ts.FunctionDeclaration =>
          ts.isFunctionDeclaration(node) && selected.has(node.name?.text ?? ""),
      )
      .map((node) => "export " + node.getText(tree))
      .join("\n");
    input =
      `import { CHANNELS_3D, local3D, parent3D, world3D, is3DLayer, perspectiveAmount, planeMatrix, projectPoint, inversePlane, planeContains } from "../reference/legacy/core/space-3d";
import { ASSET_DRAG_MIME, FX_DRAG_MIME, writeAssetDrag, hasAssetDrag, hasFileDrag, hasMediaDrag, readAssetDrag, hasFxDrag, readFxDrag, applyFxDrop } from "./drop";
import { findPanel, hasPanel, removePanel, insertPanel, hidePanel, restorePanel } from "./layout-model";
const EMPTY_AFFINE=[1,0,0,1,0,0], EMPTY_MAT3=[1,0,0,0,1,0,0,0,1], EMPTY_MAT4=[1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1];\n` +
      input;
  }
  let code = ts.transpile(input, {
    target: ts.ScriptTarget.ES2023,
    module: ts.ModuleKind.ESNext,
  });
  if (name === "store-api")
    code = code
      .replace("'../../shared/sandbox-rpc'", "'./sandbox-rpc'")
      .replace("'../src/legacy/core/easing'", "'./ease-runtime'")
      .replace(
        "'../src/legacy/core/space-3d'",
        "'../reference/legacy/core/space-3d'",
      );
  if (name === "registries")
    code = code
      .replace("from './keychord'", "from '../../keychord'")
      .replace("from './registry'", "from '../../registry'")
      .replace("from './services'", "from '../../services'");
  if (name === "host") {
    // Runtime/instrumentation injection replaces app imports; closures stay local.
    code = code.replace(
      /^import .*from '(?:svelte\/reactivity|\.\/runtime-globals|\.\.\/runtime\/performance-monitor|\.\.\/runtime\/extension-load|\.\/bridge)';\n/gm,
      "",
    );
    const start = code.indexOf("const HOST_HAPTIC =");
    const end = code.indexOf("\n});", start) + 4;
    if (start < 0 || end < start)
      throw new Error("Unsupported frozen haptic source");
    code = code.slice(0, start) + code.slice(end);
    code = code.replace("from './keychord'", "from '../../keychord'");
    code = code.replace(
      "export function createExtensionAPI(kernel, record, deps, vars = {}) {",
      `export function createExtensionAPI(kernel, record, deps, vars = {}) {
    const createSubscriber = deps.createSubscriber ?? (() => () => {});
    const mountComponent = deps.mount;
    const HOST_HAPTIC = deps.haptic ?? Object.freeze({ alignment() {} });
    const performanceMonitor = { record: (...args) => deps.metrics?.record?.(...args) };
    const extensionLoad = { record: (...args) => deps.metrics?.load?.(...args) };`,
    );
    code = code.replace(
      "deps.reportRuntimeError(id, error);",
      "deps.reportRuntimeError(id, error, label);",
    );
    code = code.replaceAll("performance.now()", "(deps.now?.() ?? Date.now())");
    // N3/N2: the tag's comment promises registration disposal unmounts the panel.
    code = code.replace(
      "const guarded = { ...def };",
      "const guarded = { ...def };\n            let releaseComponent = null;",
    );
    code = code.replace(
      "collect({ dispose: release });",
      "releaseComponent = release;",
    );
    code = code.replace(
      "return contribute(() => kernel.panels.register(id, guarded));",
      "return contribute(() => { const registration = kernel.panels.register(id, guarded); return { dispose() { registration.dispose(); releaseComponent?.(); } }; });",
    );
    // N3: requested version, rather than the tag's accidental hardcoded 1.
    code = code.replace(
      "apiVersion: 1,\n        manifest,",
      "apiVersion: manifest.apiVersion,\n        manifest,",
    );
  }
  return format(
    `// Extracted from v1.2.0 ${sources[name] ?? base + name + ".ts"}.\n// Generated by scripts/freeze-host.ts; see compat-realm.md for adaptations.\n` +
      code,
    { parser: "babel", singleQuote: false, trailingComma: "all" },
  );
}
if (import.meta.main)
  for (const name of [
    "ease-runtime",
    "store-api",
    "sandbox-rpc",
    "host",
    "registries",
    "glsl",
    "extension-layers",
    "import-defaults",
    "workspace-appearance",
    "theme-state",
    "panel-frame",
    "layout-model",
    "side-scroll",
    "drop",
    "binding",
    "pm-facades",
    "icons",
  ]) {
    await Bun.write(
      join(import.meta.dir, `../src/frozen/host/${name}.js`),
      await frozen(name),
    );
  }
