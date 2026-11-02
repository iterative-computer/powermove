// Build tool only. The resulting scripts run without Bun/Node in pm-runtime-v8.
import path from "node:path";
import { readFile, writeFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import { initial } from "./initial";
const [reference, input, output] = process.argv.slice(2);
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
const svelteRoot = path.join(reference!, "apps/desktop/node_modules/svelte");
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
const code = `import * as compat from ${JSON.stringify(compat)};
import {RemoteDOM} from ${JSON.stringify(dom)};
${modules.map((name, index) => `import * as m${index} from ${JSON.stringify(browserModule(name))};`).join("\n")}
globalThis.PowermoveCompat={...compat,RemoteDOM,runtimeModules:{${modules.map((name, index) => `${JSON.stringify(name)}:m${index}`).join(",")}}};`;
const bundle = await Bun.build({
  entrypoints: ["extcheck:entry"],
  target: "browser",
  format: "iife",
  conditions: ["browser", "production"],
  define: { "process.env.NODE_ENV": '"production"' },
  plugins: [
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
