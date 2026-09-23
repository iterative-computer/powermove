import { afterAll, expect, test } from 'bun:test';
import { createServer } from 'node:net';
import { createEmulator, type Emulator } from 'emulate';
import { createApp } from '../src/app';
import { withData } from './db';
import { makeEnv } from './env';

let emulator: Emulator | undefined;
afterAll(async () => { if (emulator) await emulator.close(); });

async function waitForDiscovery(url: string): Promise<void> {
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      if ((await fetch(url)).ok) return;
    } catch { /* listener may not be ready yet */ }
    await Bun.sleep(20);
  }
  throw new Error('Google emulator did not start');
}

async function freePort(): Promise<number> {
  const server = createServer();
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address();
  if (!address || typeof address === 'string') throw new Error('missing port');
  await new Promise<void>((resolve) => server.close(() => resolve()));
  return address.port;
}

function b64(bytes: Uint8Array): string {
  return Buffer.from(bytes).toString('base64url');
}

test('Google emulator completes desktop OAuth, handoff, and one-use exchange', async () =>
  withData(async (data) => {
    const port = await freePort();
    const origin = 'http://localhost:8787';
    emulator = await createEmulator({
      service: 'google', port,
      seed: { google: {
        users: [
          { email: 'jude@example.com', name: 'Jude', email_verified: true },
          { email: 'mara@example.com', name: 'Mara', email_verified: true },
        ],
        oauth_clients: [{
          client_id: 'powermove-local', client_secret: 'powermove-local-secret',
          name: 'Powermove Local',
          redirect_uris: [`${origin}/v1/auth/oauth2/callback/google`],
        }],
      } },
    });
    await waitForDiscovery(`${emulator.url}/.well-known/openid-configuration`);
    const env = makeEnv(data);
    env.APP_ORIGIN = origin as typeof env.APP_ORIGIN;
    env.GOOGLE_CLIENT_ID = 'powermove-local';
    env.GOOGLE_CLIENT_SECRET = 'powermove-local-secret';
    env.GOOGLE_DISCOVERY_URL = `${emulator.url}/.well-known/openid-configuration`;
    const app = createApp({ data: () => data });
    const verifier = b64(crypto.getRandomValues(new Uint8Array(32)));
    const challenge = b64(new Uint8Array(await crypto.subtle.digest('SHA-256', Buffer.from(verifier, 'base64url'))));
    const state = crypto.randomUUID().replaceAll('-', '');
    const start = await app.request(`/v1/auth/desktop?provider=google&state=${state}&challenge=${challenge}`, {}, env);
    expect(start.status).toBe(302);
    const authorize = new URL(start.headers.get('location')!);
    expect(`${authorize.origin}${authorize.pathname}`).toBe(`${emulator.url}/o/oauth2/v2/auth`);
    expect(authorize.searchParams.get('redirect_uri')).toBe(`${origin}/v1/auth/oauth2/callback/google`);
    const stateCookie = start.headers.getSetCookie().find((cookie) => cookie.includes('.state='))?.split(';')[0];
    expect(stateCookie).toBeTruthy();

    // The consent page renders one form per seeded user. Its hidden fields are
    // the OAuth parameters from the authorization URL plus the chosen email.
    const consent = await fetch(authorize);
    expect(consent.status).toBe(200);
    expect(await consent.text()).toContain('jude@example.com');
    const form = new URLSearchParams(authorize.searchParams);
    form.set('email', 'jude@example.com');
    const approved = await fetch(`${emulator.url}/o/oauth2/v2/auth/callback`, {
      method: 'POST', body: form, redirect: 'manual',
    });
    expect(approved.status).toBe(302);
    const callback = new URL(approved.headers.get('location')!);
    expect(`${callback.origin}${callback.pathname}`).toBe(`${origin}/v1/auth/oauth2/callback/google`);
    expect(callback.searchParams.get('code')).toBeTruthy();
    const signedIn = await app.request(`${callback.pathname}${callback.search}`, { headers: { Cookie: stateCookie! } }, env);
    expect(signedIn.status).toBe(302);
    expect(signedIn.headers.get('location')).toContain(`/v1/auth/desktop/done?state=${state}`);
    const sessionCookie = signedIn.headers.getSetCookie().find((cookie) => cookie.includes('session_token='))?.split(';')[0];
    expect(sessionCookie).toBeTruthy();
    const done = await app.request(`/v1/auth/desktop/done?state=${state}`, { headers: { Cookie: sessionCookie! } }, env);
    expect(done.status).toBe(200);
    const link = (await done.text()).match(/powermove:\/\/auth\?state=[^"<]+/)?.[0].replaceAll('&amp;', '&');
    expect(link).toBeTruthy();
    const token = new URL(link!).searchParams.get('token');
    const exchange = () => app.request('/v1/auth/desktop/exchange', {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ state, token, verifier }),
    }, env);
    const first = await exchange();
    expect(first.status).toBe(200);
    const session = await first.json() as { token: string; expiresAt: string };
    expect(session.token).toBeTruthy();
    expect(Date.parse(session.expiresAt)).toBeGreaterThan(Date.now());
    const me = await app.request('/v1/me', { headers: { Authorization: `Bearer ${session.token}` } }, env);
    expect(me.status).toBe(200);
    expect((await me.json() as { user: { email: string } }).user.email).toBe('jude@example.com');
    expect((await exchange()).status).toBe(401);
    expect((await app.request('/v1/auth/oauth2/callback/github', {}, env)).status).toBe(404);
    expect((await app.request('/v1/auth/sign-in/social', { method: 'POST' }, env)).status).toBe(404);
  }), 30_000);
