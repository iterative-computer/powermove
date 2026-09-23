import * as fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { afterEach, describe, expect, it, vi } from 'vitest';

import type { SafeStorageLike } from '../env/store';
import { challengeFor, createCloudAuth, parseAuthLink, PENDING_SIGN_IN_MS } from './auth';
import { createCloudSession } from './session';

/* The server's algorithm (apps/cloud/src/routes/auth-desktop.ts `challengeOf`),
   copied as an oracle: base64url(SHA-256(verifier bytes)). */
function decode(value: string): Uint8Array | null {
  if (!/^[A-Za-z0-9_-]+$/.test(value)) return null;
  return Uint8Array.from(atob(value.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
}
function b64url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
async function challengeOf(verifier: string): Promise<string | null> {
  const bytes = decode(verifier);
  if (!bytes || bytes.length !== 32) return null;
  return b64url(new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes))));
}

const ORIGIN = 'https://cloud.example.test';
const ME = {
  user: { id: '3f1c9b1e-8f55-4d8f-9d0a-6f1d1c1b2a3e', name: 'Jude', email: 'jude@example.test', image: null },
  publisher: null,
  settings: { rememberInstalls: true }
};
const safeStorage: SafeStorageLike = {
  isEncryptionAvailable: () => true,
  encryptString: (text) => Buffer.from(`sealed:${text}`),
  decryptString: (buffer) => buffer.toString().slice('sealed:'.length)
};
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map((dir) => fs.rm(dir, { recursive: true, force: true })));
});

type Call = { url: string; method: string; body: unknown };

async function harness(options: { exchange?: () => Response; confirm?: boolean } = {}) {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'powermove-cloud-auth-'));
  roots.push(root);
  const calls: Call[] = [];
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input);
    calls.push({ url, method: init?.method ?? 'GET', body: init?.body ? JSON.parse(String(init.body)) : undefined });
    if (url.endsWith('/v1/auth/desktop/exchange')) return options.exchange?.() ?? json({ token: 'bearer-new', expiresAt: new Date(Date.now() + 86_400_000).toISOString() });
    if (url.endsWith('/v1/me') && init?.method === 'DELETE') return new Response(null, { status: 204 });
    if (url.endsWith('/v1/me')) return json(ME);
    return json({ error: 'not_found' }, 404);
  }) as unknown as typeof globalThis.fetch;
  let now = 1_000_000;
  const session = createCloudSession({ dir: path.join(root, 'cloud'), safeStorage: () => safeStorage, origin: () => ORIGIN, appVersion: '1.0.0', fetch, now: () => Date.now() });
  const opened: string[] = [];
  const notifyAccount = vi.fn();
  const notifySignInFailed = vi.fn();
  const confirmDelete = vi.fn(async () => options.confirm ?? false);
  const log = vi.fn();
  const auth = createCloudAuth({
    session,
    openExternal: async (url) => { opened.push(url); },
    notifyAccount,
    notifySignInFailed,
    confirmDelete,
    now: () => now,
    log
  });
  return { auth, session, calls, opened, notifyAccount, notifySignInFailed, confirmDelete, log, advance: (ms: number) => { now += ms; } };
}

function started(opened: string[]): { provider: string; state: string; challenge: string } {
  const url = new URL(opened.at(-1)!);
  expect(`${url.origin}${url.pathname}`).toBe(`${ORIGIN}/v1/auth/desktop`);
  return { provider: url.searchParams.get('provider')!, state: url.searchParams.get('state')!, challenge: url.searchParams.get('challenge')! };
}

describe('challenge', () => {
  it('matches the server’s challengeOf for random verifiers', async () => {
    for (let i = 0; i < 8; i++) {
      const verifier = Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString('base64url');
      expect(challengeFor(verifier)).toBe(await challengeOf(verifier));
    }
  });
});

describe('browser sign-in', () => {
  it('opens the registry with a hex state and a challenge the server accepts', async () => {
    const h = await harness();
    await h.auth.beginSocialSignIn('google');
    const { provider, state, challenge } = started(h.opened);
    expect(provider).toBe('google');
    expect(state).toMatch(/^[a-f0-9]{32}$/);
    expect(challenge).toMatch(/^[A-Za-z0-9_-]{43}$/);
    expect(h.auth.hasPending()).toBe(true);

    await h.auth.handleDeepLink(`powermove://auth?state=${state}&token=one-time`);
    const exchange = h.calls.find((call) => call.url.endsWith('/v1/auth/desktop/exchange'))!;
    const verifier = (exchange.body as { verifier: string }).verifier;
    expect(await challengeOf(verifier)).toBe(challenge);
  });

  it('ignores a link with the wrong state and keeps waiting', async () => {
    const h = await harness();
    await h.auth.beginSocialSignIn('google');
    await h.auth.handleDeepLink('powermove://auth?state=deadbeefdeadbeefdeadbeefdeadbeef&token=stolen');
    expect(h.calls).toHaveLength(0);
    expect(h.auth.hasPending()).toBe(true);
    expect(h.notifyAccount).not.toHaveBeenCalled();
    expect(h.log).toHaveBeenCalled();
    expect(h.log.mock.calls.flat().join(' ')).not.toContain('stolen');
  });

  it('exchanges { state, token, verifier } for the right state, then clears the pending sign-in', async () => {
    const h = await harness();
    await h.auth.beginSocialSignIn('google');
    const { state } = started(h.opened);
    await h.auth.handleDeepLink(`powermove://auth?state=${state}&token=one-time-token`);

    const exchange = h.calls[0]!;
    expect(exchange.url).toBe(`${ORIGIN}/v1/auth/desktop/exchange`);
    expect(exchange.method).toBe('POST');
    expect(Object.keys(exchange.body as object).sort()).toEqual(['state', 'token', 'verifier']);
    expect(exchange.body).toMatchObject({ state, token: 'one-time-token' });
    expect(h.auth.hasPending()).toBe(false);
    expect(h.session.currentToken()).toBe('bearer-new');
    expect(h.session.provider()).toBe('google');
    expect(h.notifyAccount).toHaveBeenCalledWith(ME);

    // The same link again does nothing: the pending sign-in is spent.
    await h.auth.handleDeepLink(`powermove://auth?state=${state}&token=one-time-token`);
    expect(h.calls.filter((call) => call.url.endsWith('/exchange'))).toHaveLength(1);
  });

  it('clears the pending sign-in when the exchange fails, and says so', async () => {
    const h = await harness({ exchange: () => json({ error: 'unauthorized' }, 401) });
    await h.auth.beginSocialSignIn('google');
    const { state } = started(h.opened);
    await h.auth.handleDeepLink(`powermove://auth?state=${state}&token=t`);
    expect(h.auth.hasPending()).toBe(false);
    expect(h.session.currentToken()).toBeNull();
    expect(h.notifySignInFailed).toHaveBeenCalledWith({ error: 'unauthorized' });
    expect(h.notifyAccount).not.toHaveBeenCalled();
  });

  it('ignores a link once the pending sign-in expired', async () => {
    const h = await harness();
    await h.auth.beginSocialSignIn('google');
    const { state } = started(h.opened);
    h.advance(PENDING_SIGN_IN_MS + 1);
    await h.auth.handleDeepLink(`powermove://auth?state=${state}&token=t`);
    expect(h.calls).toHaveLength(0);
    expect(h.auth.hasPending()).toBe(false);
  });

  it('a new start replaces the pending one', async () => {
    const h = await harness();
    await h.auth.beginSocialSignIn('google');
    const first = started(h.opened).state;
    await h.auth.beginSocialSignIn('google');
    const second = started(h.opened).state;
    expect(second).not.toBe(first);
    await h.auth.handleDeepLink(`powermove://auth?state=${first}&token=t`);
    expect(h.calls).toHaveLength(0);
    await h.auth.handleDeepLink(`powermove://auth?state=${second}&token=t`);
    expect(h.calls[0]!.url).toBe(`${ORIGIN}/v1/auth/desktop/exchange`);
  });

  it('parses only powermove://auth links', () => {
    expect(parseAuthLink('powermove://auth?state=ab&token=cd')).toEqual({ state: 'ab', token: 'cd' });
    expect(parseAuthLink('powermove://other?state=ab&token=cd')).toBeNull();
    expect(parseAuthLink('https://auth?state=ab&token=cd')).toBeNull();
    expect(parseAuthLink('powermove://auth?state=ab')).toBeNull();
    expect(parseAuthLink('not a url')).toBeNull();
  });
});

describe('account actions', () => {
  it('delete asks first and does nothing when cancelled', async () => {
    const h = await harness({ confirm: false });
    await h.session.save({ token: 'bearer', expiresAt: new Date(Date.now() + 86_400_000).toISOString() });
    expect(await h.auth.deleteAccount()).toBe(false);
    expect(h.confirmDelete).toHaveBeenCalledTimes(1);
    expect(h.calls).toHaveLength(0);
    expect(h.session.currentToken()).toBe('bearer');
  });

  it('delete sends DELETE /v1/me after the confirmation and signs out', async () => {
    const h = await harness({ confirm: true });
    await h.session.save({ token: 'bearer', expiresAt: new Date(Date.now() + 86_400_000).toISOString() });
    expect(await h.auth.deleteAccount()).toBe(true);
    expect(h.calls.map((call) => `${call.method} ${call.url}`)).toEqual([`DELETE ${ORIGIN}/v1/me`]);
    expect(h.session.currentToken()).toBeNull();
    expect(h.notifyAccount).toHaveBeenCalledWith(null);
  });
});
