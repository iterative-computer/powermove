import { defineConfig } from '@playwright/test';
import base from './playwright.config';

/** Explicit private measurement harness, excluded from the ordinary e2e suite. */
export default defineConfig({
  ...base,
  testDir: './e2e/tools',
  testMatch: ['svgstills-capture.ts', 'svgstills-parity.ts'],
});
