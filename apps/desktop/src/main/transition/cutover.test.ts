import { mkdtemp, mkdir, rm, writeFile } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, expect, it } from 'vitest';
import { cutoverFeed, readCutoverConfig } from './cutover';
import type { TRStatus } from '../../shared/transition';
let root: string;
beforeEach(async () => { root = await mkdtemp(path.join(os.tmpdir(), 'pm-tr-feed-')); await mkdir(path.join(root, 'transition')); });
afterEach(async () => { await rm(root, { recursive: true, force: true }); });
const status: TRStatus = { state: 'complete', done: 1, total: 1, mediaComplete: true, localStorageComplete: true, secretsComplete: true, cutoverReady: true };
const gate = { platform: 'darwin', darwinMajor: 23, ownerApproved: true, signedMigrationVerified: true };
const config = { version: 1 as const, enabled: true, feedURL: 'https://updates.example.invalid/native/' };
it('ships an unset, disabled feed and defaults a missing configuration to disabled', async () => {
  for (const value of [await readCutoverConfig(root), await readCutoverConfig(path.resolve('resources'))]) {
    expect(value).toEqual({ version: 1, enabled: false, feedURL: null });
    expect(cutoverFeed(value, status, gate)).toBeNull();
  }
});
it('retains an owner-supplied dedicated HTTPS feed without contacting it', async () => {
  await writeFile(path.join(root, 'transition/cutover.json'), JSON.stringify(config));
  expect(await readCutoverConfig(root)).toEqual(config);
  expect(cutoverFeed(config, status, gate)).toBe(config.feedURL);
});
it.each(['http://updates.example.invalid/', 'file:///tmp/feed', 'https://user:pass@example.invalid/', 'https://example.invalid/?token=value', 'https://example.invalid/#feed'])('rejects unsafe feed configuration %s', async feedURL => {
  await writeFile(path.join(root, 'transition/cutover.json'), JSON.stringify({ ...config, feedURL }));
  await expect(readCutoverConfig(root)).rejects.toThrow();
});
it('requires macOS 14+, fresh complete preparation, signed proof and owner approval independently', () => {
  for (const change of [{ platform: 'linux' }, { darwinMajor: 22 }, { ownerApproved: false }, { signedMigrationVerified: false }]) {
    expect(cutoverFeed(config, status, { ...gate, ...change })).toBeNull();
  }
  for (const key of ['cutoverReady', 'mediaComplete', 'localStorageComplete', 'secretsComplete']) {
    expect(cutoverFeed(config, { ...status, [key]: false }, gate)).toBeNull();
  }
});
it.each([{ ...config, enabled: 'true' }, { ...config, version: 2 }, { ...config, feedURL: null }, null])('rejects malformed or incomplete enabled configuration', async value => {
  await writeFile(path.join(root, 'transition/cutover.json'), JSON.stringify(value));
  await expect(readCutoverConfig(root)).rejects.toThrow();
});
