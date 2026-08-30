import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 30_000,
  // The desktop suite intentionally uses one worker so hidden Electron
  // sessions never contend for GPU/process resources. Give the complete
  // suite enough time to run instead of cutting it off after ~30 tests.
  globalTimeout: 600_000,
  expect: { timeout: 10_000 },
  outputDir: './e2e/test-results',
  reporter: [
    ['list'],
    ['html', { outputFolder: './e2e/playwright-report', open: 'never' }]
  ],
  use: {
    actionTimeout: 10_000,
    trace: 'retain-on-failure'
  }
});
