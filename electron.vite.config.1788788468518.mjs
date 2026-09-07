// electron.vite.config.ts
import { defineConfig } from "electron-vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import path2 from "node:path";

// scripts/player-bundle.ts
import { build } from "esbuild";
import path from "node:path";
function playerBundlePlugin() {
  return {
    name: "powermove-player-bundle",
    resolveId(id) {
      if (id === "virtual:powermove-player") return "\0" + id;
    },
    async load(id) {
      if (id !== "\0virtual:powermove-player") return;
      const result = await build({
        entryPoints: [path.resolve("src/renderer/src/player/player.ts")],
        bundle: true,
        write: false,
        format: "esm",
        platform: "browser",
        target: "es2022",
        minify: true,
        legalComments: "inline",
        define: { "import.meta.hot": "false" },
        metafile: true
      });
      for (const file of Object.keys(result.metafile.inputs)) this.addWatchFile(path.resolve(file));
      return `export default ${JSON.stringify(result.outputFiles[0].text)};`;
    }
  };
}

// electron.vite.config.ts
var __electron_vite_injected_dirname = "/Users/chike/Documents/Work 2/Apps/Powermove";
var electron_vite_config_default = defineConfig({
  // A sandboxed preload must be CommonJS; pin the format so a future
  // `"type": "module"` in package.json cannot silently flip it to .mjs.
  main: {
    build: {
      outDir: "out/main",
      rollupOptions: { output: { format: "cjs", entryFileNames: "[name].js" } }
    }
  },
  preload: {
    build: {
      outDir: "out/preload",
      rollupOptions: { output: { format: "cjs", entryFileNames: "[name].js" } }
    }
  },
  renderer: {
    root: "src/renderer",
    publicDir: "public",
    plugins: [svelte(), playerBundlePlugin()],
    resolve: { alias: { powermove: path2.resolve(__electron_vite_injected_dirname, "src/renderer/src/kernel/api.ts") } },
    build: {
      outDir: "out/renderer",
      rollupOptions: {
        output: {
          assetFileNames: (assetInfo) => {
            const name = assetInfo.names[0] ?? "";
            return /\.(?:otf|ttf|woff2?)$/i.test(name) ? "assets/fonts/[name]-[hash][extname]" : "assets/[name]-[hash][extname]";
          }
        }
      }
    }
  }
});
export {
  electron_vite_config_default as default
};
