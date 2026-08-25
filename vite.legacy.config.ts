import { defineConfig } from 'vite';

export default defineConfig({
  publicDir: false,
  build: {
    outDir: 'host',
    emptyOutDir: false,
    lib: {
      entry: 'src/renderer/src/legacy/bootstrap.ts',
      name: 'PowermoveLegacy',
      formats: ['iife'],
      fileName: () => 'legacy-bundle.js'
    },
    rollupOptions: {
      output: { inlineDynamicImports: true }
    }
  }
});
