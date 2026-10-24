import { defineConfig } from '@playwright/test';
import config from './playwright.config';

/** Build before taking the shared app lease; this config only runs tests. */
export default defineConfig({ ...config, globalSetup: undefined });
