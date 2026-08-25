import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  globalSetup: './e2e/global-setup.ts',
  fullyParallel: false,
  workers: 1,
  retries: 0,
  timeout: 30_000,
  globalTimeout: 120_000,
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
