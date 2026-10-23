/** Opt-in qualification against the built app. Build native replay binaries
 * outside the Electron lock; this batch only captures app references. */
import { execFile } from 'node:child_process';
import path from 'node:path';
import { promisify } from 'node:util';
import { test } from '@playwright/test';
const run = promisify(execFile);
const root = process.env.PM_COLOR_DEPTH_EXPANDED_ROOT;
const repo = path.resolve(__dirname, '../../..');
test.skip(!root, 'Set PM_COLOR_DEPTH_EXPANDED_ROOT to an external artifact directory');
for (const [name, script, directory] of [
  ['expanded depth-eight compositor', 'color-depth-expanded.ts', 'composition'],
  ['actual transparent PNG at every depth', 'color-depth-alpha.ts', 'alpha'],
] as const) {
  test(name, async () => {
    test.setTimeout(600_000);
    const { stdout, stderr } = await run('bun', [path.join(repo, 'engine/corpus/src', script), path.join(root!, directory)],
      { cwd: repo, timeout: 540_000, maxBuffer: 16 * 1024 * 1024 });
    process.stdout.write(stdout); process.stderr.write(stderr);
  });
}
