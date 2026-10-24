import os from 'node:os';
import path from 'node:path';
import { defineConfig } from '@playwright/test';
import base from '../../playwright.config';
export default defineConfig({ ...base, testDir: '../../e2e', testMatch: '**/export-qos.spec.ts',
  globalSetup: undefined, workers: 1, retries: 0, globalTimeout: 0,
  outputDir: path.join(os.tmpdir(), 'powermove-export-qos-results'), reporter: [['list']],
});
