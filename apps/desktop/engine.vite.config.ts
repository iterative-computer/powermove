import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';
import path from 'node:path';
import { playerBundlePlugin } from './scripts/player-bundle';

/* The document engine: the renderer's core built for Node (SSR target), so
   the same editing, history and agent tool code runs on the host without a
   browser. Output goes next to the serve host bundle. */
export default defineConfig({
  plugins: [svelte({ hot: false, compilerOptions: { css: 'injected' } }), playerBundlePlugin()],
  resolve: { alias: { powermove: path.resolve(__dirname, 'src/renderer/src/kernel/api.ts') } },
  build: {
    ssr: true,
    target: 'node22',
    outDir: process.env['POWERMOVE_ENGINE_OUT'] ?? 'out/engine',
    emptyOutDir: true,
    rollupOptions: {
      input: path.resolve(__dirname, 'src/engine/main.ts'),
      output: { format: 'es', entryFileNames: 'engine.mjs' },
      external: ['happy-dom', 'ws']
    },
    sourcemap: true,
    minify: false
  },
  ssr: { noExternal: true, external: ['happy-dom', 'ws'] }
});
