// Build tool only. The resulting scripts run without Bun/Node in pm-runtime-v8.
import path from "node:path";
import { readFile, writeFile, realpath } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { initial } from "./initial";
import { CLOCK_SOURCE } from "./clock-source.mjs";
import { controlImports, controlPlugin } from "./shared-controls";
const [reference, input, output] = process.argv.slice(2);
await writeFile(path.join(output!, "clock.js"), CLOCK_SOURCE);
await writeFile(path.join(output!, "initial.json"), JSON.stringify(initial()));
const rawManifest = JSON.parse(
  await readFile(path.join(input!, "manifest.json"), "utf8"),
);
const { parseManifest } = await import(
  pathToFileURL(path.join(reference!, "apps/desktop/src/shared/extensions.ts"))
    .href
);
const parsed = parseManifest(rawManifest);
if (!parsed.ok) throw Error("FROZEN_MANIFEST_INVALID: " + parsed.error);
const manifest = parsed.manifest;
await writeFile(path.join(output!, "manifest.json"), JSON.stringify(manifest));
const { compileExtension } = await import(
  pathToFileURL(
    path.join(reference!, "apps/desktop/src/main/extensions/compiler.ts"),
  ).href
);
// Reuse the frozen compiler's import fence, Svelte settings, style acquisition and export table.
const compiled = await compileExtension({
  dir: input,
  entry: manifest.entry ?? "index.ts",
  outDir: path.join(output!, "compile"),
});
if (!compiled.ok) throw Error(compiled.error);
const extension = await Bun.build({
  entrypoints: ["extcheck:extension"],
  target: "browser",
  format: "iife",
  plugins: [
    {
      name: "extension-global",
      setup(build) {
        build.onResolve({ filter: /^extcheck:extension$/ }, () => ({
          path: "entry",
          namespace: "extension",
        }));
        build.onLoad({ filter: /.*/, namespace: "extension" }, () => ({
          contents: `import * as module from ${JSON.stringify(compiled.bundlePath)}; globalThis.ExtCheckModule=module;`,
          loader: "js",
        }));
      },
    },
  ],
});
if (!extension.success) throw Error("Extension IIFE build failed");
await writeFile(
  path.join(output!, "extension.js"),
  await extension.outputs[0]!.text(),
);
// Loading unused browser modules can itself require unavailable window capabilities
// (e.g. motion's eager media query). Include the extension's actual import closure.
const compiledSource = await readFile(compiled.bundlePath, "utf8");
const requested = new Set<string>([
  "svelte",
  "svelte/reactivity",
  ...Array.from(
    compiledSource.matchAll(
      /__powermove_runtime\[["'](svelte(?:\/[^"']*)?)["']\]/g,
    ),
    (match) => match[1]!,
  ),
]);
const modules = [...requested];
const compat = path.resolve(
  import.meta.dir,
  "../../apps/native/js/compat/src/index.ts",
);
const dom = path.resolve(
  import.meta.dir,
  "../../apps/native/js/remote-dom/src/shim.ts",
);
const svelteRoot = await realpath(
  path.join(reference!, "apps/desktop/node_modules/svelte"),
);
const sveltePackage = JSON.parse(
  await readFile(path.join(svelteRoot, "package.json"), "utf8"),
);
const browserModule = (name: string) => {
  let target =
    sveltePackage.exports[name === "svelte" ? "." : "." + name.slice(6)];
  while (typeof target === "object" && target)
    target =
      target.browser ?? target.production ?? target.import ?? target.default;
  if (typeof target !== "string")
    throw Error("Unsupported shared Svelte module");
  return path.resolve(svelteRoot, target);
};
const sharedSveltePlugin: Bun.BunPlugin = {
  name: "one-host-svelte-runtime",
  setup(build) {
    build.onResolve({ filter: /^svelte(?:\/.*)?$/ }, ({ path: name }) => ({
      path: browserModule(name),
    }));
  },
};
const code = `import * as compat from ${JSON.stringify(compat)};
import {RemoteDOM} from ${JSON.stringify(dom)};
${controlImports(reference!)}
${modules.map((name, index) => `import * as m${index} from ${JSON.stringify(browserModule(name))};`).join("\n")}
globalThis.PowermoveCompat={...compat,RemoteDOM,controls,controlStores,controlStyles:styles,runtimeModules:{${modules.map((name, index) => `${JSON.stringify(name)}:m${index}`).join(",")}}};`;
const bundle = await Bun.build({
  entrypoints: ["extcheck:entry"],
  target: "browser",
  format: "iife",
  conditions: ["browser", "production"],
  define: { "process.env.NODE_ENV": '"production"' },
  plugins: [
    sharedSveltePlugin,
    await controlPlugin(reference!),
    {
      name: "compat-runner-entry",
      setup(build) {
        build.onResolve({ filter: /^extcheck:entry$/ }, () => ({
          path: "entry",
          namespace: "extcheck",
        }));
        build.onLoad({ filter: /.*/, namespace: "extcheck" }, () => ({
          contents: code,
          loader: "ts",
          resolveDir: path.join(reference!, "apps/desktop"),
        }));
      },
    },
  ],
});
if (!bundle.success)
  throw Error(bundle.logs.map((log) => log.message).join("\n"));
await writeFile(
  path.join(output!, "compat.js"),
  await bundle.outputs[0]!.text(),
);
// Store gets the frozen asynchronous sandbox facade, not CompatRealm or PM.
const storeShim = path.resolve(
  import.meta.dir,
  "../../apps/native/js/compat/src/frozen/host/store-api.js",
);
const helpers = path.resolve(
  import.meta.dir,
  "../../apps/native/js/compat/src/frozen/kernel/editor-helpers.ts",
);
const helperExports = path.resolve(
  reference!,
  "apps/desktop/src/shared/extension-runtime.ts",
);
const pureStoreHelpers = [
  "isProperty",
  "canAnimateContent",
  "contentLabel",
  "structuredProperties",
  "pathTargets",
  "validMatteSource",
  "MATTE_MODES",
  "expressionDiagnostic",
  "EXPRESSION_NAMES",
  "axisContentKey",
  "axisPath",
  "isAxisTag",
  "CHANNELS_3D",
  "projectPoint",
  "inversePlane",
  "propertyShortcuts",
  "TEXT_ANIMATOR_PROPERTIES",
  "animatorMode",
  "countTextUnits",
  "staggerLength",
  "staggerWindow",
];
const storeCode = `import {createSandboxAPI,sandboxControl,sandboxReporter} from ${JSON.stringify(storeShim)};
import * as helpers from ${JSON.stringify(helpers)};
import {EDITOR_HELPER_EXPORTS} from ${JSON.stringify(helperExports)};
${modules.map((name, index) => `import * as m${index} from ${JSON.stringify(browserModule(name))};`).join("\n")}
const pure=new Set(${JSON.stringify(pureStoreHelpers)});
const powermove=Object.fromEntries(EDITOR_HELPER_EXPORTS.map(name=>[name,pure.has(name)?helpers[name]:(...args)=>{
  StoreCompat.report?.('permission','powermove.'+name);
  const error=new Error(name+' requires full access. Use project.apply or commands in a sandboxed extension.');
  error.name='PermissionError';error.code='full-access';throw error;
}]));
globalThis.StoreCompat={createSandboxAPI,sandboxControl,sandboxReporter,runtimeModules:{powermove,${modules.map((name, index) => `${JSON.stringify(name)}:m${index}`).join(",")}}};`;
const storeBundle = await Bun.build({
  entrypoints: ["extcheck:store"],
  target: "browser",
  format: "iife",
  conditions: ["browser", "production"],
  define: { "process.env.NODE_ENV": '"production"' },
  plugins: [
    sharedSveltePlugin,
    {
      name: "frozen-store-facade",
      setup(build) {
        build.onResolve({ filter: /^extcheck:store$/ }, () => ({
          path: "store",
          namespace: "store-entry",
        }));
        build.onLoad({ filter: /.*/, namespace: "store-entry" }, () => ({
          contents: storeCode,
          loader: "ts",
          resolveDir: path.join(reference!, "apps/desktop"),
        }));
      },
    },
  ],
});
if (!storeBundle.success) throw Error("Frozen Store facade build failed");
await writeFile(
  path.join(output!, "store.js"),
  await storeBundle.outputs[0]!.text(),
);
const page = await Bun.build({
  entrypoints: [
    path.resolve(
      import.meta.dir,
      "../../apps/native/js/remote-dom/src/browser-applier.ts",
    ),
  ],
  target: "browser",
  format: "iife",
});
if (!page.success) throw Error("Page applier build failed");
await writeFile(path.join(output!, "page.js"), await page.outputs[0]!.text());
