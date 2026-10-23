import { mkdir, mkdtemp, readdir, rm, utimes, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { setTimeout } from 'node:timers/promises';
import { expect, launchApp, test } from './helpers/app';

/* Startup reclaims runtime downloads a killed process left in the temp
   directory (main/runtime-updates.ts sweepRuntimeScratch), without touching a
   live install or an editor-run directory that shares the old prefix. */
test('startup reclaims abandoned runtime downloads and keeps everything else', async () => {
  test.setTimeout(60_000);
  const tmpDir = await mkdtemp(path.join(os.tmpdir(), 'runtime-scratch-e2e-'));
  const old = new Date(Date.now() - 2 * 60 * 60 * 1000);
  const seed = async (name: string, files: Record<string, string>, aged = true) => {
    const dir = path.join(tmpDir, name);
    await mkdir(dir);
    for (const [file, text] of Object.entries(files)) await writeFile(path.join(dir, file), text);
    if (!aged) return;
    for (const file of Object.keys(files)) await utimes(path.join(dir, file), old, old);
    await utimes(dir, old, old);
  };
  // A fresh directory whose owner is gone (macOS pids stay below 100000).
  await seed('powermove-runtime-claude-crashed', { 'owner.pid': '999999', 'package.tgz': 'partial' }, false);
  // A download from a build before the runtime prefix, idle for two hours.
  await seed('powermove-claude-legacy', { 'package.tgz': 'partial' });
  // An install another live process (this test runner) owns.
  await seed('powermove-runtime-codex-live', { 'owner.pid': String(process.pid), 'package.tgz': 'partial' });
  // An editor-run directory that shares the old prefix.
  await seed('powermove-codex-editor-inputs', { 'schema.json': '{}', 'package.tgz': 'not ours' });
  const scratch = async () => (await readdir(tmpDir)).filter(name => name.startsWith('powermove-')).sort();
  const kept = ['powermove-codex-editor-inputs', 'powermove-runtime-codex-live'];
  try {
    const session = await launchApp({ env: { TMPDIR: tmpDir, TMP: tmpDir, TEMP: tmpDir } });
    try {
      // The sweep starts in the background at boot; give it a moment.
      const deadline = Date.now() + 20_000;
      while ((await scratch()).length > kept.length && Date.now() < deadline) await setTimeout(100);
      expect(await scratch()).toEqual(kept);
    } finally {
      await session.close();
    }
  } finally {
    await rm(tmpDir, { recursive: true, force: true });
  }
});
