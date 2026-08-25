import { defineConfig } from 'electron-vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
  // A sandboxed preload must be CommonJS; pin the format so a future
  // `"type": "module"` in package.json cannot silently flip it to .mjs.
  main: {
    build: {
      outDir: 'out/main',
      rollupOptions: { output: { format: 'cjs', entryFileNames: '[name].js' } }
    }
  },
  preload: {
    build: {
      outDir: 'out/preload',
      rollupOptions: { output: { format: 'cjs', entryFileNames: '[name].js' } }
    }
  },
  renderer: {
    root: 'src/renderer',
    plugins: [svelte()],
    build: {
      outDir: 'out/renderer'
    }
  }
});
