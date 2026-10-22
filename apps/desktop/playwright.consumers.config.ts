import { defineConfig } from '@playwright/test';
import base from './playwright.config';

/** Build the app and native modules before taking pm-electron-lock. This
 * gate runs the full suite without the ledger's quarantine exclusions. */
export default defineConfig({
  ...base,
  globalSetup: undefined,
  // Allow the complete serial suite to finish; individual test budgets stay at base values.
  globalTimeout: 4 * 60 * 60_000,
  workers: 1,
  outputDir: process.env.POWERMOVE_E2E_OUTPUT_DIR ?? base.outputDir,
  reporter: [['line'], ['json']],
});
