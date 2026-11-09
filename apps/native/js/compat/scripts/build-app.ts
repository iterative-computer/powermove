/** Build-time only: embedded realm bundles for the native app. Run with Bun. */
import { join, dirname } from "node:path";
import { readFile, mkdir, writeFile, realpath } from "node:fs/promises";
const output = process.argv[2];
if (!output) throw Error("Pass the output directory");
await mkdir(output, { recursive: true });
const root = join(import.meta.dir, "..");
const svelteRoot = await realpath(join(root, "node_modules/svelte"));
const sveltePackage = JSON.parse(
  await readFile(join(svelteRoot, "package.json"), "utf8"),
);
const browserModule = (name: string) => {
  let target =
    sveltePackage.exports[name === "svelte" ? "." : "." + name.slice(6)];
  while (typeof target === "object" && target)
    target =
      target.browser ?? target.production ?? target.import ?? target.default;
  if (typeof target !== "string") throw Error(`Unsupported ${name}`);
  return join(svelteRoot, target);
};
const shared: Bun.BunPlugin = {
  name: "one-host-svelte-runtime",
  setup(build) {
    build.onResolve({ filter: /^svelte(?:\/.*)?$/ }, ({ path }) => ({
      path: browserModule(path),
    }));
  },
};
const compiler = await import(join(svelteRoot, "compiler/index.js"));
const transpiler = new Bun.Transpiler({ loader: "ts", target: "browser" });
const styles = "powermove-control-styles";
const controls: Bun.BunPlugin = {
  name: "frozen-reference-controls",
  setup(build) {
    build.onResolve({ filter: new RegExp(`^${styles}$`) }, () => ({
      path: "styles",
      namespace: styles,
    }));
    build.onLoad({ filter: /.*/, namespace: styles }, () => ({
      contents:
        "export const styles=[];export const add=css=>styles.push(css);",
      loader: "js",
    }));
    build.onLoad({ filter: /\.svelte(?:\.[tj]s)?$/ }, async ({ path }) => {
      const source = await readFile(path, "utf8");
      const compiled = path.endsWith(".svelte")
        ? compiler.compile(source, {
            filename: path,
            generate: "client",
            css: "external",
            runes: true,
          })
        : compiler.compileModule(
            path.endsWith(".ts") ? transpiler.transformSync(source) : source,
            { filename: path, generate: "client", runes: true },
          );
      const css = compiled.css?.code;
      return {
        contents:
          compiled.js.code +
          (css
            ? `\nimport {add as __add} from '${styles}';__add(${JSON.stringify(css)});`
            : ""),
        loader: "js",
        resolveDir: dirname(path),
      };
    });
    // v1 swapped each <select> for a DOM trigger and hid the select with app
    // CSS this realm doesn't load, so both showed ("Photos ⌄ Photos"). Here the
    // select itself is the control: the page draws it as a native pop-up.
    build.onLoad(
      { filter: /frozen\/reference\/controls\/select\/enhance\.ts$/ },
      () => ({
        contents: [
          "export const enhanceSelect = () => () => {};",
          "export const refreshSelect = () => {};",
          "export const fancySelect = () => ({ update() {}, destroy() {} });",
          "export const autoEnhanceSelects = () => () => {};",
        ].join("\n"),
        loader: "js",
      }),
    );
    build.onLoad(
      { filter: /frozen\/reference\/.*\.css$/ },
      async ({ path }) => ({
        contents: `import {add} from '${styles}';add(${JSON.stringify(await readFile(path, "utf8"))});`,
        loader: "js",
        resolveDir: dirname(path),
      }),
    );
  },
};
const virtual = (name: string, code: string, resolveDir: string) => ({
  name: `${name}-entry`,
  setup(build: Bun.PluginBuilder) {
    build.onResolve({ filter: new RegExp(`^${name}$`) }, () => ({
      path: "entry",
      namespace: name,
    }));
    build.onLoad({ filter: /.*/, namespace: name }, () => ({
      contents: code,
      loader: "ts",
      resolveDir,
    }));
  },
});
const write = async (name: string, result: any) => {
  if (!result.success)
    throw Error(result.logs.map((log: any) => log.message).join("\n"));
  await writeFile(join(output, name), await result.outputs[0].text());
};
const options = {
  target: "browser",
  format: "iife",
  conditions: ["browser", "production"] as string[],
  define: { "process.env.NODE_ENV": '"production"' },
} as const;
const modules = [
  "svelte",
  "svelte/animate",
  "svelte/attachments",
  "svelte/easing",
  "svelte/events",
  "svelte/internal/client",
  "svelte/motion",
  "svelte/reactivity",
  "svelte/reactivity/window",
  "svelte/store",
  "svelte/transition",
];
// The compiler admits exactly these imports; the realms provide them.
const { ADAPTED_MODULES } = await import(join(root, "src/module-graph.ts"));
await writeFile(
  join(output, "runtime-modules.txt"),
  ["powermove", ...modules, ...Object.keys(ADAPTED_MODULES)].join("\n"),
);
const moduleImports = modules
  .map((name, index) => `import * as m${index} from ${JSON.stringify(name)};`)
  .join("\n");
const moduleTable = modules
  .map((name, index) => `${JSON.stringify(name)}:m${index}`)
  .join(",");
const controlNames = [
  "NumField",
  "ColorField",
  "FillField",
  "RampField",
  "PointField",
  "SliderField",
  "FontField",
  "SelectField",
  "TextField",
  "ToggleField",
  "Row",
  "Section",
  "Segmented",
  "Disclosure",
];
const reference = join(root, "src/frozen/reference");
const realm = `import * as compat from ${JSON.stringify(join(root, "src/index.ts"))};
import {RemoteDOM} from ${JSON.stringify(join(root, "../remote-dom/src/shim.ts"))};
import {installLegacyRuntime} from ${JSON.stringify(join(root, "src/legacy/runtime.ts"))};
import {createLoader} from ${JSON.stringify(join(root, "src/loader.ts"))};
import {cloneAcceptingViews} from ${JSON.stringify(join(root, "src/mirror.ts"))};
${controlNames.map((name) => `import ${name} from ${JSON.stringify(join(reference, `controls/${name}.svelte`))};`).join("\n")}
import {doc} from ${JSON.stringify(join(reference, "state/document.svelte.ts"))};
import {setSelection} from ${JSON.stringify(join(reference, "state/selection.svelte.ts"))};
import {transport,updateControlTime} from ${JSON.stringify(join(reference, "state/transport.svelte.ts"))};
import {styles} from '${styles}';
${moduleImports}
// Extensions clone project records (structuredClone(PM.L(id))); PM.proj hands
// them views of those records, which a plain structured clone refuses.
if(globalThis.structuredClone)globalThis.structuredClone=cloneAcceptingViews(globalThis.structuredClone);
globalThis.PowermoveCompat={...compat,RemoteDOM,installLegacyRuntime,createLoader,controls:{${controlNames.join(",")}},controlStores:{doc,setSelection,transport,updateControlTime},controlStyles:styles,runtimeModules:{${moduleTable}}};`;
await write(
  "compat.js",
  await Bun.build({
    ...options,
    entrypoints: ["powermove-realm"],
    plugins: [shared, controls, virtual("powermove-realm", realm, root)],
  }),
);
const pure = [
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
const store = `import {createSandboxAPI,sandboxControl,sandboxReporter} from ${JSON.stringify(join(root, "src/frozen/host/store-api.js"))};
import {RemoteDOM} from ${JSON.stringify(join(root, "../remote-dom/src/shim.ts"))};
import * as helpers from ${JSON.stringify(join(root, "src/frozen/reference/kernel/editor-helpers.ts"))};
${moduleImports}
import {definitionSnapshots} from ${JSON.stringify(join(root, "src/definitions.ts"))};
import {panelStyle} from ${JSON.stringify(join(root, "src/panels.ts"))};
import {linkRequest} from ${JSON.stringify(join(root, "src/module-graph.ts"))};
const pure=new Set(${JSON.stringify(pure)});
const powermove=Object.fromEntries(Object.keys(helpers).map(name=>[name,pure.has(name)?helpers[name]:(...args)=>{
  StoreCompat.report?.('permission','powermove.'+name);
  const error=new Error(name+' requires full access. Use project.apply or commands in a sandboxed extension.');
  error.name='PermissionError';error.code='full-access';throw error;
}]));
const registry=list=>({entries:()=>(list??[]).map(item=>({id:item.id,ownerId:"store",item}))});
const definitions=({effects,transitions,layers})=>definitionSnapshots({effects:registry(effects),transitions:registry(transitions),layerTypes:registry(layers),ownerOrigin:()=>"store"});
globalThis.StoreCompat={createSandboxAPI,sandboxControl,sandboxReporter,definitions,panelStyle,RemoteDOM,linkRequest,runtimeModules:{powermove,${moduleTable}}};`;
await write(
  "store.js",
  await Bun.build({
    ...options,
    entrypoints: ["powermove-store"],
    plugins: [shared, virtual("powermove-store", store, root)],
  }),
);
await write(
  "page.js",
  await Bun.build({
    entrypoints: [join(root, "../remote-dom/src/browser-applier.ts")],
    target: "browser",
    format: "iife",
  }),
);
