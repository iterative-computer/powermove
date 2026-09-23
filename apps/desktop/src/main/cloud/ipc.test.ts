import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ApiError } from '@powermove/registry/wire';

import { CLOUD_IPC } from '../../shared/cloud-ipc';
import type { SafeStorageLike } from '../env/store';
import { createCloudAuth, type CloudAuth } from './auth';
import { registerCloudIpc } from './ipc';
import { createCloudSession, type CloudSession } from './session';

const safeStorage: SafeStorageLike = {
  isEncryptionAvailable: () => true,
  encryptString: (text) => Buffer.from(`sealed:${text}`),
  decryptString: (buffer) => buffer.toString().slice('sealed:'.length)
};

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

function fakeAuth(): { [K in keyof CloudAuth]: ReturnType<typeof vi.fn> } {
  return {
    beginSocialSignIn: vi.fn(async () => undefined),
    handleDeepLink: vi.fn(async () => undefined),
    sendEmailCode: vi.fn(async () => undefined),
    verifyEmailCode: vi.fn(async () => null),
    claimHandle: vi.fn(async () => null),
    setRememberInstalls: vi.fn(async () => null),
    signOut: vi.fn(async () => undefined),
    deleteAccount: vi.fn(async () => false),
    hasPending: vi.fn(() => false)
  };
}

function fakeSession(): CloudSession {
  return {
    dir: '/nowhere',
    load: vi.fn(async () => undefined),
    save: vi.fn(async () => undefined),
    clear: vi.fn(async () => undefined),
    currentToken: () => null,
    me: () => null,
    provider: () => undefined,
    client: () => { throw new Error('no client in this test'); },
    refreshMe: vi.fn(async () => null),
    signOut: vi.fn(async () => undefined)
  };
}

function fakeRegistry(answer = true) {
  let origin = 'https://cloud.trypowermove.com';
  return {
    get: () => origin,
    change: vi.fn(async (next: string) => {
      if (answer) origin = new URL(next).origin;
      return answer;
    })
  };
}

function register(auth: CloudAuth, session: CloudSession, trusted = true, registry = fakeRegistry()) {
  const handlers = new Map<string, (event: unknown, payload: unknown) => Promise<unknown>>();
  const ipcMain = { handle: (channel: string, handler: (event: unknown, payload: unknown) => Promise<unknown>) => void handlers.set(channel, handler) };
  registerCloudIpc(ipcMain as never, { auth, session, registry, isTrusted: () => trusted });
  const call = (channel: string, payload?: unknown) => handlers.get(channel)!({ sender: {}, senderFrame: null }, payload);
  return { handlers, call };
}

describe('cloud IPC', () => {
  it('registers every invoke channel', () => {
    const { handlers } = register(fakeAuth() as unknown as CloudAuth, fakeSession());
    expect([...handlers.keys()].sort()).toEqual(
      Object.values(CLOUD_IPC).filter((channel) => channel !== CLOUD_IPC.accountChanged && channel !== CLOUD_IPC.signInFailed).sort()
    );
  });

  it('rejects malformed payloads before anything runs', async () => {
    const auth = fakeAuth();
    const { call } = register(auth as unknown as CloudAuth, fakeSession());
    const bad: Array<[string, unknown]> = [
      [CLOUD_IPC.signInSocial, { provider: 'apple' }],
      [CLOUD_IPC.signInSocial, { provider: 'google', extra: true }],
      [CLOUD_IPC.signInSocial, 'google'],
      [CLOUD_IPC.emailSend, { email: 'not-an-email' }],
      [CLOUD_IPC.emailSend, { email: `${'a'.repeat(250)}@example.test` }],
      [CLOUD_IPC.emailVerify, { email: 'a@example.test', otp: '12345' }],
      [CLOUD_IPC.emailVerify, { email: 'a@example.test', otp: 'abcdef' }],
      [CLOUD_IPC.claimHandle, { handle: 'Jude' }],
      [CLOUD_IPC.claimHandle, { handle: '-jude' }],
      [CLOUD_IPC.claimHandle, { handle: 'j' }],
      [CLOUD_IPC.setRememberInstalls, { value: 'yes' }],
      [CLOUD_IPC.setRememberInstalls, {}],
      [CLOUD_IPC.signOut, { now: true }],
      [CLOUD_IPC.deleteAccount, { confirmed: true }],
      [CLOUD_IPC.accountGet, [1]],
      [CLOUD_IPC.registryUrl, { origin: 'x' }],
      [CLOUD_IPC.setRegistryUrl, { origin: 'file:///etc/passwd' }],
      [CLOUD_IPC.setRegistryUrl, { origin: 'javascript:alert(1)' }],
      [CLOUD_IPC.setRegistryUrl, { origin: 'not a url' }],
      [CLOUD_IPC.setRegistryUrl, { origin: `https://${'a'.repeat(2000)}.test` }],
      [CLOUD_IPC.setRegistryUrl, { origin: 'https://registry.example.test', extra: 1 }]
    ];
    for (const [channel, payload] of bad) {
      await expect(call(channel, payload), `${channel} ${JSON.stringify(payload)}`).rejects.toThrow(/cloud:/);
    }
    for (const fn of Object.values(auth)) expect(fn).not.toHaveBeenCalled();
  });

  it('refuses untrusted senders', async () => {
    const auth = fakeAuth();
    const { call } = register(auth as unknown as CloudAuth, fakeSession(), false);
    await expect(call(CLOUD_IPC.signInSocial, { provider: 'google' })).rejects.toThrow(/untrusted sender/);
    await expect(call(CLOUD_IPC.accountGet)).rejects.toThrow(/untrusted sender/);
    expect(auth.beginSocialSignIn).not.toHaveBeenCalled();
  });

  it('returns registry errors as results instead of throwing', async () => {
    const auth = fakeAuth();
    auth.claimHandle.mockRejectedValueOnce(new ApiError({ error: 'handle_taken' }));
    const { call } = register(auth as unknown as CloudAuth, fakeSession());
    await expect(call(CLOUD_IPC.claimHandle, { handle: 'jude' })).resolves.toEqual({ ok: false, error: { error: 'handle_taken' } });
    await expect(call(CLOUD_IPC.emailSend, { email: 'a@example.test' })).resolves.toEqual({ ok: true, value: null });
    expect(auth.sendEmailCode).toHaveBeenCalledWith('a@example.test');
  });

  it('delete-account runs only after the native confirmation', async () => {
    const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-cloud-ipc-'));
    roots.push(root);
    const fetch = vi.fn(async () => new Response(null, { status: 204 })) as unknown as typeof globalThis.fetch;
    const session = createCloudSession({ dir: path.join(root, 'cloud'), safeStorage: () => safeStorage, origin: () => 'https://cloud.example.test', appVersion: '1.0.0', fetch });
    await session.save({ token: 'bearer', expiresAt: new Date(Date.now() + 86_400_000).toISOString() });
    let answer = false;
    const confirmDelete = vi.fn(async () => answer);
    const auth = createCloudAuth({ session, openExternal: async () => undefined, notifyAccount: vi.fn(), notifySignInFailed: vi.fn(), confirmDelete });
    const { call } = register(auth, session);

    await expect(call(CLOUD_IPC.deleteAccount)).resolves.toEqual({ ok: true, value: { deleted: false } });
    expect(confirmDelete).toHaveBeenCalledTimes(1);
    expect(fetch).not.toHaveBeenCalled();
    expect(session.currentToken()).toBe('bearer');

    answer = true;
    await expect(call(CLOUD_IPC.deleteAccount)).resolves.toEqual({ ok: true, value: { deleted: true } });
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(session.currentToken()).toBeNull();
  });

  it('account-get answers from the cache when the registry is unreachable', async () => {
    const session = fakeSession();
    const cached = { user: { id: 'x', name: 'Jude', email: 'j@example.test', image: null }, publisher: null, settings: { rememberInstalls: true } };
    const offline: CloudSession = { ...session, currentToken: () => 'bearer', me: () => cached as never, provider: () => 'email', refreshMe: vi.fn(async () => { throw new ApiError({ error: 'internal' }); }) };
    const { call } = register(fakeAuth() as unknown as CloudAuth, offline);
    await expect(call(CLOUD_IPC.accountGet)).resolves.toEqual({ me: cached, provider: 'email' });
  });

  it('changes the registry only through main’s confirmation', async () => {
    const registry = fakeRegistry(false);
    const { call } = register(fakeAuth() as unknown as CloudAuth, fakeSession(), true, registry);
    await expect(call(CLOUD_IPC.registryUrl)).resolves.toEqual({ origin: 'https://cloud.trypowermove.com', isDefault: true });
    await expect(call(CLOUD_IPC.setRegistryUrl, { origin: 'https://registry.example.test/path' }))
      .resolves.toEqual({ ok: true, value: { origin: 'https://cloud.trypowermove.com', isDefault: true, changed: false } });
    expect(registry.change).toHaveBeenCalledWith('https://registry.example.test/path');

    const accepting = fakeRegistry(true);
    const next = register(fakeAuth() as unknown as CloudAuth, fakeSession(), true, accepting);
    await expect(next.call(CLOUD_IPC.setRegistryUrl, { origin: 'https://registry.example.test' }))
      .resolves.toEqual({ ok: true, value: { origin: 'https://registry.example.test', isDefault: false, changed: true } });
  });
});
