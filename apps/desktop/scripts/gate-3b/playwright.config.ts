import { defineConfig } from '@playwright/test';

import base from '../../playwright.config';

/**
 * The Phase 3b gate's app measurements (docs/engine/phase3b-gate.md): the
 * same hidden harness as the e2e suite (`e2e/helpers/app.ts`, one worker,
 * the production build from global setup), with this folder as the test
 * directory so measurement-only specs stay out of the ledger run. Run under
 * the Electron lock:
 *
 *   pm-electron-lock bunx playwright test --config scripts/gate-3b/playwright.config.ts --workers=1
 */
export default defineConfig({
  ...base,
  testDir: '.',
  globalSetup: '../../e2e/global-setup.ts',
  outputDir: '../../e2e/test-results/gate-3b',
  reporter: [['list']],
  globalTimeout: 0,
  timeout: 1_800_000
});
