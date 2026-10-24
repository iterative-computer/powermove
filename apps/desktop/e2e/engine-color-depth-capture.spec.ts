/** Build/prepare outside the shared lease, then run this opt-in capture batch
 * with playwright.prebuilt.config.ts and --workers=1 under pm-electron-lock.
 * Each capture owns and removes its isolated Electron profile in finally.
 * Persistent evidence is written only to the supplied external directory.
 */
import { execFile } from 'node:child_process';
import path from 'node:path';
import { promisify } from 'node:util';
import { test } from '@playwright/test';

const run = promisify(execFile);
const root = process.env.PM_COLOR_DEPTH_CAPTURE_ROOT;
const repo = path.resolve(__dirname, '../../..');
test.skip(!root, 'Set PM_COLOR_DEPTH_CAPTURE_ROOT to prepared external captures');

for (const [name, script, directory, args] of [
  ['detailed video and 3D', 'color-depth-media.ts', 'media', []],
  ['transparent PNG with blur', 'color-depth-alpha.ts', 'alpha', []],
  ['fresh text, source and retention', 'color-depth.ts', 'text', ['--text-only']],
] as const) {
  test(name, async () => {
    test.setTimeout(600_000);
    const { stdout, stderr } = await run('bun', [
      path.join(repo, 'engine/corpus/src', script), path.join(root!, directory), ...args,
    ], { cwd: repo, timeout: 540_000, maxBuffer: 16 * 1024 * 1024 });
    process.stdout.write(stdout);
    process.stderr.write(stderr);
  });
}

test('today composition counters on the seven-project slice', async () => {
  test.skip(!process.env.PM_COLOR_DEPTH_PARITY_SLICE, 'Set PM_COLOR_DEPTH_PARITY_SLICE to the seven-project capture');
  test.setTimeout(600_000);
  const { stdout, stderr } = await run('bun', [
    path.join(repo, 'engine/corpus/src/color-depth-today-counters.ts'), process.env.PM_COLOR_DEPTH_PARITY_SLICE!,
  ], { cwd: repo, timeout: 540_000, maxBuffer: 16 * 1024 * 1024 });
  process.stdout.write(stdout);
  process.stderr.write(stderr);
});
