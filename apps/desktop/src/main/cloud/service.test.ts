import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

import { confirmRegistryChange, createRegistryUrlSetting, DEFAULT_REGISTRY_ORIGIN } from './registry-url';
import { createDeepLinkQueue, deepLinksIn } from './service';

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

describe('deep links', () => {
  it('holds links that arrive before the account is ready, then delivers them in order', () => {
    const queue = createDeepLinkQueue();
    queue.push('powermove://auth?state=a&token=1');
    queue.push('powermove://auth?state=b&token=2');
    const seen: string[] = [];
    queue.ready((url) => seen.push(url));
    queue.push('powermove://auth?state=c&token=3');
    expect(seen).toEqual(['powermove://auth?state=a&token=1', 'powermove://auth?state=b&token=2', 'powermove://auth?state=c&token=3']);
  });

  it('finds powermove:// links in a second instance’s argv', () => {
    expect(deepLinksIn(['/Applications/Powermove.app/Contents/MacOS/Powermove', '--flag', 'powermove://auth?state=a&token=1', '/tmp/x.pmv']))
      .toEqual(['powermove://auth?state=a&token=1']);
  });
});

describe('registry URL', () => {
  async function setting() {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-registry-'));
    roots.push(root);
    return createRegistryUrlSetting(path.join(root, 'cloud'));
  }

  it('defaults to Powermove Cloud', async () => {
    const value = await setting();
    expect(await value.load()).toBe(DEFAULT_REGISTRY_ORIGIN);
  });

  it('changes only after the native confirmation, showing the new origin', async () => {
    const value = await setting();
    const beforeChange = vi.fn(async () => undefined);
    const cancel = vi.fn(async (_options: { message: string }) => ({ response: 1 }));
    expect(await confirmRegistryChange('https://registry.example.test/some/path', { setting: value, showMessageBox: cancel, beforeChange })).toBe(false);
    expect(value.get()).toBe(DEFAULT_REGISTRY_ORIGIN);
    expect(beforeChange).not.toHaveBeenCalled();

    const accept = vi.fn(async (_options: { message: string }) => ({ response: 0 }));
    expect(await confirmRegistryChange('https://registry.example.test/some/path', { setting: value, showMessageBox: accept, beforeChange })).toBe(true);
    expect(accept.mock.calls[0]![0].message).toContain('https://registry.example.test');
    expect(beforeChange).toHaveBeenCalledTimes(1);
    expect(value.get()).toBe('https://registry.example.test');
    expect(await value.load()).toBe('https://registry.example.test');
  });

  it('refuses anything that is not http(s)', async () => {
    const value = await setting();
    await expect(confirmRegistryChange('file:///etc/passwd', { setting: value, showMessageBox: async () => ({ response: 0 }) })).rejects.toThrow();
  });
});
