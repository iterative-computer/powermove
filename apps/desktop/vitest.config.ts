import { svelte } from '@sveltejs/vite-plugin-svelte';
import { defineConfig } from 'vitest/config';
import path from 'node:path';
import { playerBundlePlugin } from './scripts/player-bundle';

// The legacy suite under tests/ runs with `node --test` (npm test); vitest only
// owns the new TypeScript modules under src/. The svelte plugin compiles
// *.svelte.ts rune modules and components for unit tests.
export default defineConfig({
  // Keep Vite's test cache in this checkout; node_modules may be a read-only symlink.
  cacheDir: path.resolve(__dirname, '.vite'),
  plugins: [svelte({ hot: false }), playerBundlePlugin()],
  resolve: { conditions: ['browser'], alias: { powermove: path.resolve(__dirname, 'src/renderer/src/kernel/api.ts') } },
  test: {
    // The animation suite measures real frame-evaluation time. Concurrent test
    // workers otherwise turn its calibrated budget into a CPU-contention test.
    fileParallelism: false,
    include: ['src/**/*.{test,spec}.ts', 'scripts/**/*.{test,spec}.ts'],
    // scripts/gate-3b holds Playwright specs (its own playwright.config.ts).
    exclude: ['**/node_modules/**', 'out/**', 'tests/**', 'scripts/gate-3b/**'],
    passWithNoTests: true,
    // @scritto/core imports its own stylesheet; let Vite load it instead of Node.
    server: { deps: { inline: [/@scritto\//] } },
    setupFiles: ['src/test-setup/waapi.ts']
    // Renderer tests that need a DOM declare `// @vitest-environment happy-dom`.
  }
});
