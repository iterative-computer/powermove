import { defineConfig } from 'vitest/config';

// The legacy suite under tests/ runs with `node --test` (npm test); vitest only
// owns the new TypeScript modules under src/.
export default defineConfig({
  test: {
    include: ['src/**/*.{test,spec}.ts'],
    exclude: ['**/node_modules/**', 'out/**', 'tests/**', 'spikes/**'],
    passWithNoTests: true
  }
});
