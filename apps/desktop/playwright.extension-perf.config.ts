import { existsSync } from 'node:fs';
import path from 'node:path';
import { defineConfig } from '@playwright/test';

const output = process.env.PM_EXTENSION_PERF_OUT;
if (!output) throw new Error('Set PM_EXTENSION_PERF_OUT to an evidence directory outside the repository.');
const repo = path.resolve(__dirname, '../..');
const relative = path.relative(repo, path.resolve(output));
if (!relative || (!relative.startsWith(`..${path.sep}`) && !path.isAbsolute(relative))) throw new Error('Evidence must be outside this public repository.');
if (!existsSync(path.join(__dirname, 'out/main/index.js'))) throw new Error('Build the app and native addons before acquiring the Electron lock.');

export default defineConfig({
  testDir: './e2e/perf',
  testMatch: 'paused-extension.bench.ts',
  workers: 1,
  retries: 0,
  timeout: 600_000,
  outputDir: path.join(output, 'artifacts'),
  reporter: [['list'], ['json', { outputFile: path.join(output, 'playwright.json') }]]
});
