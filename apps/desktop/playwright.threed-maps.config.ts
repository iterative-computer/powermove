import { defineConfig } from '@playwright/test';
import base from './playwright.config';

/** Explicit private measurement harness for 3D material maps, excluded from the ordinary e2e suite. */
export default defineConfig({
  ...base,
  testDir: './e2e/tools',
  testMatch: ['threed-maps-capture.ts'],
});
