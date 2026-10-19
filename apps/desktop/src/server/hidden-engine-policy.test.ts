import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { expect, it } from 'vitest';
import { requireHiddenHostRuntime } from './hidden-engine-policy';

it('never starts the Chromium-only macOS host for an enabled native profile', async () => {
  const root = await mkdtemp(path.join(os.tmpdir(), 'pm-hidden-policy-'));
  try {
    await expect(requireHiddenHostRuntime(root, {}, 'darwin')).resolves.toBeUndefined();
    await mkdir(path.join(root, 'store'));
    await writeFile(path.join(root, 'store/useNewEngine.json'), 'true');
    await expect(requireHiddenHostRuntime(root, {}, 'darwin')).rejects.toThrow('native-hidden-host-unavailable');
    await expect(requireHiddenHostRuntime(root, { POWERMOVE_NEW_ENGINE: '0' }, 'darwin')).resolves.toBeUndefined();
    await expect(requireHiddenHostRuntime(root, { POWERMOVE_NEW_ENGINE: '1' }, 'darwin')).rejects.toThrow('shader pins');
    await expect(requireHiddenHostRuntime(root, { POWERMOVE_NEW_ENGINE: '1' }, 'linux')).resolves.toBeUndefined();
    await writeFile(path.join(root, 'store/useNewEngine.json'), 'broken');
    await expect(requireHiddenHostRuntime(root, {}, 'darwin')).rejects.toThrow('native-engine-setting-unreadable');
  } finally { await rm(root, { recursive: true, force: true }); }
});
