import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { SafeStorageLike } from '../env/store';
import { createCloudSession } from './session';

const ORIGIN = 'https://cloud.example.test';
const ME = {
  user: { id: '3f1c9b1e-8f55-4d8f-9d0a-6f1d1c1b2a3e', name: 'Jude', email: 'jude@example.test', image: null },
  publisher: { id: '7d2a0b6e-1c3f-4a5b-8c9d-0e1f2a3b4c5d', handle: 'jude', tombstoned: false },
  settings: { rememberInstalls: true }
};

/* Reversible but not plaintext, so a test can tell sealed bytes from JSON. */
const safeStorage: SafeStorageLike = {
  isEncryptionAvailable: () => true,
  encryptString: (text) => Buffer.from(Buffer.from(text, 'utf8').map((byte) => byte ^ 0x5a)),
  decryptString: (buffer) => Buffer.from(buffer.map((byte) => byte ^ 0x5a)).toString('utf8')
};

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

async function harness(options: { fetch?: typeof fetch; origin?: () => string; now?: () => number } = {}) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-cloud-session-'));
  roots.push(root);
  const dir = path.join(root, 'cloud');
  const make = () => createCloudSession({
    dir,
    safeStorage: () => safeStorage,
    origin: options.origin ?? (() => ORIGIN),
    appVersion: '1.0.0',
    ...(options.fetch ? { fetch: options.fetch } : {}),
    ...(options.now ? { now: options.now } : {})
  });
  return { dir, make };
}

const future = () => new Date(Date.now() + 86_400_000).toISOString();
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

describe('cloud session', () => {
  it('seals the token with safeStorage and reads it back', async () => {
    const { dir, make } = await harness();
    const session = make();
    await session.save({ token: 'bearer-abc', expiresAt: future() }, 'github');
    expect(session.currentToken()).toBe('bearer-abc');

    const sealed = await fs.readFile(path.join(dir, 'session.bin'));
    expect(sealed.toString('utf8')).not.toContain('bearer-abc');
    expect(JSON.parse(safeStorage.decryptString(sealed))).toMatchObject({ origin: ORIGIN, token: 'bearer-abc' });
    const cache = JSON.parse(await fs.readFile(path.join(dir, 'session.json'), 'utf8'));
    expect(cache).toMatchObject({ origin: ORIGIN, me: null, provider: 'github' });
    expect(JSON.stringify(cache)).not.toContain('bearer-abc');

    const reloaded = make();
    await reloaded.load();
    expect(reloaded.currentToken()).toBe('bearer-abc');
    expect(reloaded.provider()).toBe('github');
  });

  it('clear removes both files', async () => {
    const { dir, make } = await harness();
    const session = make();
    await session.save({ token: 'bearer-abc', expiresAt: future() });
    await session.clear();
    await expect(fs.stat(path.join(dir, 'session.bin'))).rejects.toThrow();
    await expect(fs.stat(path.join(dir, 'session.json'))).rejects.toThrow();
    expect(session.currentToken()).toBeNull();
    const reloaded = make();
    await reloaded.load();
    expect(reloaded.currentToken()).toBeNull();
  });

  it('caches GET /v1/me and serves it from disk after a restart', async () => {
    const fetch = vi.fn(async () => json(ME)) as unknown as typeof globalThis.fetch;
    const { make } = await harness({ fetch });
    const session = make();
    await session.save({ token: 'bearer-abc', expiresAt: future() });
    expect(await session.refreshMe()).toEqual(ME);
    const reloaded = make();
    await reloaded.load();
    expect(reloaded.me()).toEqual(ME);
  });

  it('a 401 from /v1/me signs out', async () => {
    const fetch = vi.fn(async () => json({ error: 'unauthorized' }, 401)) as unknown as typeof globalThis.fetch;
    const { dir, make } = await harness({ fetch });
    const session = make();
    await session.save({ token: 'bearer-abc', expiresAt: future() });
    expect(await session.refreshMe()).toBeNull();
    expect(session.currentToken()).toBeNull();
    await expect(fs.stat(path.join(dir, 'session.bin'))).rejects.toThrow();
  });

  it('binds the token to its registry origin', async () => {
    let origin = ORIGIN;
    const { make } = await harness({ origin: () => origin });
    const session = make();
    await session.save({ token: 'bearer-abc', expiresAt: future() });
    origin = 'https://self-hosted.example.test';
    expect(session.currentToken()).toBeNull();
    expect(session.me()).toBeNull();
  });

  it('drops an expired session on load', async () => {
    let now = Date.now();
    const { dir, make } = await harness({ now: () => now });
    await make().save({ token: 'bearer-abc', expiresAt: new Date(now + 1000).toISOString() });
    now += 2000;
    const reloaded = make();
    await reloaded.load();
    expect(reloaded.currentToken()).toBeNull();
    await expect(fs.stat(path.join(dir, 'session.bin'))).rejects.toThrow();
  });

  it('sign out posts to the registry, then clears even when that fails', async () => {
    const fetch = vi.fn(async () => { throw new TypeError('offline'); }) as unknown as typeof globalThis.fetch;
    const { make } = await harness({ fetch });
    const session = make();
    await session.save({ token: 'bearer-abc', expiresAt: future() });
    await session.signOut();
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(String((fetch as unknown as { mock: { calls: unknown[][] } }).mock.calls[0]![0])).toBe(`${ORIGIN}/v1/auth/sign-out`);
    expect(session.currentToken()).toBeNull();
  });
});
