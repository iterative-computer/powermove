import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { nativeHiddenHostEnabled } from './hidden-engine-policy';

it('selects the native macOS host from the desktop profile and launch override', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-hidden-policy-'));
  try {
    await expect(nativeHiddenHostEnabled(root, {}, 'darwin')).resolves.toBe(false);
    await mkdir(path.join(root, 'store'));
    await writeFile(path.join(root, 'store/useNewEngine.json'), 'true');
    await expect(nativeHiddenHostEnabled(root, {}, 'darwin')).resolves.toBe(true);
    await expect(nativeHiddenHostEnabled(root, { POWERMOVE_NEW_ENGINE: '0' }, 'darwin')).resolves.toBe(false);
    await expect(nativeHiddenHostEnabled(root, { POWERMOVE_NEW_ENGINE: '1' }, 'darwin')).resolves.toBe(true);
    await expect(nativeHiddenHostEnabled(root, { POWERMOVE_NEW_ENGINE: '1' }, 'linux')).resolves.toBe(false);
    await writeFile(path.join(root, 'store/useNewEngine.json'), 'broken');
    await expect(nativeHiddenHostEnabled(root, {}, 'darwin')).rejects.toThrow('native-engine-setting-unreadable');
  } finally { await rm(root, { recursive: true, force: true }); }
});
