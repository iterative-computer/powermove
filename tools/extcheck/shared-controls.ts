// Build tool only: compile unchanged controls and state from the pinned checkout.
import path from "node:path";
import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

export const SHARED_CONTROLS = [
  "SelectField",
  "TextField",
  "ToggleField",
  "Row",
  "Section",
  "Segmented",
  "Disclosure",
] as const;

export function controlImports(reference: string): string {
  const renderer = path.resolve(reference, "apps/desktop/src/renderer/src");
  const from = (file: string) => JSON.stringify(path.join(renderer, file));
  return `${SHARED_CONTROLS.map((name) => `import ${name} from ${from(`controls/${name}.svelte`)};`).join("\n")}
import {doc} from ${from("state/document.svelte.ts")};
import {setSelection} from ${from("state/selection.svelte.ts")};
import {transport,updateControlTime} from ${from("state/transport.svelte.ts")};
import {styles} from 'extcheck:control-styles';
const controls={${SHARED_CONTROLS.join(",")}};
const controlStores={doc,setSelection,transport,updateControlTime};`;
}

export async function controlPlugin(reference: string): Promise<Bun.BunPlugin> {
  const compiler = await import(
    pathToFileURL(
      path.resolve(
        reference,
        "apps/desktop/node_modules/svelte/compiler/index.js",
      ),
    ).href
  );
  const transpiler = new Bun.Transpiler({ loader: "ts", target: "browser" });
  return {
    name: "frozen-shared-controls",
    setup(build) {
      build.onResolve({ filter: /^extcheck:control-styles$/ }, () => ({
        path: "styles",
        namespace: "control-styles",
      }));
      build.onLoad({ filter: /.*/, namespace: "control-styles" }, () => ({
        contents:
          "export const styles=[];export const add=css=>styles.push(css);",
        loader: "js",
      }));
      build.onLoad(
        { filter: /\.svelte(?:\.[tj]s)?$/ },
        async ({ path: file }) => {
          const source = await readFile(file, "utf8");
          const compiled = file.endsWith(".svelte")
            ? compiler.compile(source, {
                filename: file,
                generate: "client",
                css: "external",
                runes: true,
              })
            : compiler.compileModule(
                file.endsWith(".ts")
                  ? transpiler.transformSync(source)
                  : source,
                { filename: file, generate: "client", runes: true },
              );
          const css = compiled.css?.code;
          return {
            contents:
              compiled.js.code +
              (css
                ? `\nimport {add as __add} from 'extcheck:control-styles';__add(${JSON.stringify(css)});`
                : ""),
            loader: "js",
            resolveDir: path.dirname(file),
          };
        },
      );
      build.onLoad({ filter: /\.css$/ }, async ({ path: file }) => ({
        contents: `import {add} from 'extcheck:control-styles';add(${JSON.stringify(await readFile(file, "utf8"))});`,
        loader: "js",
        resolveDir: path.dirname(file),
      }));
    },
  };
}
