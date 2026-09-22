// electron.vite.config.ts
import { defineConfig } from "electron-vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import path from "node:path";

// scripts/player-bundle.ts
import { bundlePlayerEntry, entries } from "@powermove/player/build";
function playerBundlePlugin() {
  return {
    name: "powermove-player-bundle",
    resolveId(id) {
      if (id === "virtual:powermove-player") return "\0" + id;
    },
    async load(id) {
      if (id !== "\0virtual:powermove-player") return;
      const { text, inputs } = await bundlePlayerEntry(entries.player);
      for (const file of inputs) this.addWatchFile(file);
      return `export default ${JSON.stringify(text)};`;
    }
  };
}

// electron.vite.config.ts
var __electron_vite_injected_dirname = "/Users/chike/Documents/Work 2/Apps/Powermove/apps/desktop";
var electron_vite_config_default = defineConfig({
  // A sandboxed preload must be CommonJS; pin the format so a future
  // `"type": "module"` in package.json cannot silently flip it to .mjs.
  main: {
    build: {
      outDir: "out/main",
      rollupOptions: { input: { index: path.resolve(__electron_vite_injected_dirname, "src/main/entry.ts"), editor: path.resolve(__electron_vite_injected_dirname, "src/main/index.ts") }, output: { format: "cjs", entryFileNames: "[name].js" } }
    }
  },
  preload: {
    build: {
      outDir: "out/preload",
      rollupOptions: {
        input: {
          index: path.resolve(__electron_vite_injected_dirname, "src/preload/index.ts"),
          onboarding: path.resolve(__electron_vite_injected_dirname, "src/preload/onboarding.ts")
        },
        output: { format: "cjs", entryFileNames: "[name].js" }
      }
    }
  },
  renderer: {
    root: "src/renderer",
    publicDir: "public",
    plugins: [svelte(), playerBundlePlugin()],
    resolve: { alias: { powermove: path.resolve(__electron_vite_injected_dirname, "src/renderer/src/kernel/api.ts") } },
    build: {
      outDir: "out/renderer",
      rollupOptions: {
        input: {
          index: path.resolve(__electron_vite_injected_dirname, "src/renderer/index.html"),
          onboardingWelcome: path.resolve(__electron_vite_injected_dirname, "src/renderer/onboarding/welcome.html")
        },
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
