import os from 'node:os';
import path from 'node:path';
import { defineConfig } from '@playwright/test';
import base from './playwright.config';

// Private source/project pixels, DOM snapshots and failure traces stay
// outside the public worktree, including Playwright's automatic artifacts.
const output = path.join(os.tmpdir(), 'powermove-private-setup-compat/playwright');
export default defineConfig({
  ...base,
  testMatch: 'engine-private-setup-compat.spec.ts',
  outputDir: output,
  reporter: [['line']],
});
