import os from 'node:os';
import path from 'node:path';

import { defineConfig } from '@playwright/test';

import base from '../../playwright.config';

/** EXPORT-1 encoding parity (e2e/export-encoding-parity.spec.ts). Build the
 * desktop app and the native addon first, then run this config under
 * pm-electron-lock with --workers=1. Results and the attached exports stay
 * outside the repository; each run replaces them. */
export default defineConfig({
  ...base,
  testDir: '../../e2e',
  testMatch: '**/export-encoding-parity.spec.ts',
  globalSetup: undefined,
  workers: 1,
  retries: 0,
  timeout: 1_800_000,
  globalTimeout: 0,
  outputDir: path.join(os.tmpdir(), 'powermove-export-encoding-results'),
  reporter: [['list']],
});
