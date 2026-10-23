import { mkdtemp, readdir, rm } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { setTimeout } from 'node:timers/promises';
import { expect, launchApp, test } from './helpers/app';

test('test launches do not create runtime downloads after the automatic refresh delay', async () => {
  test.setTimeout(60_000);
  const tmpDir = await mkdtemp(path.join(os.tmpdir(), 'runtime-update-e2e-'));
  const scratchDirs = async () => (await readdir(tmpDir)).filter(name => /^powermove-(claude|codex)-/.test(name));
  try {
    const session = await launchApp({ env: { TMPDIR: tmpDir, TMP: tmpDir, TEMP: tmpDir } });
    try {
      expect(await session.app.evaluate(() => process.env.POWERMOVE_DISABLE_RUNTIME_UPDATES)).toBe('1');
      await session.page.evaluate(() => {
        const bridge = (window as any).__powermoveTestBridge;
        void bridge.chatgpt.models().catch(() => {});
        void bridge.claude.models().catch(() => {});
      });
      // Observe beyond refreshRuntime's 20-second delay while the app is alive.
      const deadline = Date.now() + 21_000;
      while (Date.now() < deadline) {
        expect(await scratchDirs()).toEqual([]);
        await setTimeout(100);
      }
    } finally {
      await session.close();
    }
    expect(await scratchDirs()).toEqual([]);
  } finally {
    await rm(tmpDir, { recursive: true, force: true });
  }
});
