import { createAuth } from '../src/auth';
import { expect, test } from 'bun:test';
import { createApp } from '../src/app';
import { desktopAuth } from '../src/db/schema';
import { eq } from 'drizzle-orm';
import { withData } from './db';
import { makeEnv } from './env';
import { seedSession } from './helpers';
const b64 = (bytes: Uint8Array) =>
  btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
async function setup(
  data: Parameters<typeof makeEnv>[0],
  env: CloudflareBindings,
  browserHeaders: Record<string, string>,
  expired = false,
) {
  const verifier = b64(crypto.getRandomValues(new Uint8Array(32)));
  const bytes = Uint8Array.from(atob(verifier.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
  const challenge = b64(new Uint8Array(await crypto.subtle.digest('SHA-256', bytes)));
  const state = crypto.randomUUID().replaceAll('-', '');
  await data.db.insert(desktopAuth).values({
    state,
    challenge,
    expiresAt: new Date(Date.now() + (expired ? -1000 : 600000)),
  });
  const app = createApp({ data: () => data });
  const done = await app.request(`/v1/auth/desktop/done?state=${state}`, { headers: browserHeaders }, env);
  if (expired) {
    expect(done.status).toBe(401);
    return { app, state, verifier, token: '' };
  }
  expect(done.status).toBe(200);
  const html = await done.text();
  const match = html.match(/powermove:\/\/auth\?state=[^"<]+/);
  expect(match).not.toBeNull();
  const url = new URL(match![0].replaceAll('&amp;', '&'));
  return { app, state, verifier, token: url.searchParams.get('token')! };
}
test('desktop exchange creates separate bearer session and consumes state', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    let otp = '';
    env.OTP_SENDER = async (_email, code) => {
      otp = code;
    };
    await app.request('/v1/auth/email/send', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: 'desktop@example.com' }),
    }, env);
    expect(otp).toBeTruthy();
    const login = await createAuth(data, env).api.signInEmailOTP({
      body: { email: 'desktop@example.com', otp },
      returnHeaders: true,
    });
    const cookie = login.headers?.get('set-cookie')?.split(';')[0];
    expect(cookie).toBeDefined();
    const browser = login.response as {
      user: {
        id: string;
      };
      token: string;
    };
    const flow = await setup(data, env, { Cookie: cookie! });
    const body = { state: flow.state, verifier: flow.verifier, token: flow.token };
    const exchange = () =>
      flow.app.request('/v1/auth/desktop/exchange', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      }, env);
    const first = await exchange();
    expect(first.status).toBe(200);
    const result = await first.json() as any;
    expect(result.token).not.toBe(browser.token);
    expect(new Date(result.expiresAt).getTime()).toBeGreaterThan(Date.now());
    const me = await flow.app.request('/v1/me', { headers: { Authorization: `Bearer ${result.token}` } }, env);
    expect(me.status).toBe(200);
    expect((await me.json() as any).user.id).toBe(browser.user.id);
    expect((await exchange()).status).toBe(401);
  }));
test('wrong verifier consumes the desktop row', async () =>
  withData(async (data) => {
    const env = makeEnv(data),
      browser = await seedSession(data, env),
      flow = await setup(data, env, { Authorization: `Bearer ${browser.token}` });
    const exchange = (verifier: string) =>
      flow.app.request('/v1/auth/desktop/exchange', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ state: flow.state, token: flow.token, verifier }),
      }, env);
    expect((await exchange(b64(new Uint8Array(32)))).status).toBe(401);
    expect((await exchange(flow.verifier)).status).toBe(401);
  }));
test('expired desktop row is rejected and consumed', async () =>
  withData(async (data) => {
    const env = makeEnv(data),
      browser = await seedSession(data, env),
      flow = await setup(data, env, { Authorization: `Bearer ${browser.token}` }, true);
    const res = await flow.app.request('/v1/auth/desktop/exchange', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ state: flow.state, token: 'invalid', verifier: flow.verifier }),
    }, env);
    expect(res.status).toBe(401);
    const [row] = await data.db.select().from(desktopAuth).where(eq(desktopAuth.state, flow.state));
    expect(row.consumedAt).not.toBeNull();
  }));
test('desktop social start carries a state cookie accepted by Better Auth verification', () =>
  withData(async (data) => {
    const env = makeEnv(data);
    env.GOOGLE_CLIENT_ID = 'test-google-client';
    env.GOOGLE_CLIENT_SECRET = 'test-google-secret';
    const app = createApp({ data: () => data });
    const state = crypto.randomUUID().replaceAll('-', '');
    const challenge = b64(crypto.getRandomValues(new Uint8Array(32)));
    const start = await app.request(`/v1/auth/desktop?provider=google&state=${state}&challenge=${challenge}`, {}, env);
    expect(start.status).toBe(302);
    const cookie = start.headers.getSetCookie().find((value) => value.includes('.state='));
    expect(cookie).toBeDefined();
    const location = new URL(start.headers.get('Location')!);
    const oauthState = location.searchParams.get('state');
    expect(oauthState).toBeTruthy();
    const { getSignedCookie } = await import('hono/cookie');
    const packageEntry = import.meta.resolve('better-auth');
    const stateModule = await import(packageEntry.replace(/\/dist\/index\.mjs$/, '/dist/state.mjs')) as {
      parseGenericState: (ctx: unknown, state: string) => Promise<{
        callbackURL: string;
      }>;
    };
    const authContext = await createAuth(data, env).$context;
    const requestContext = {
      req: { raw: new Request(env.APP_ORIGIN, { headers: { Cookie: cookie!.split(';')[0]! } }) },
    };
    const parsed = await stateModule.parseGenericState({
      context: authContext,
      getSignedCookie: (name: string, secret: string) => getSignedCookie(requestContext as never, secret, name),
      setCookie: () => {
      },
    }, oauthState!);
    expect(parsed.callbackURL).toContain(`/v1/auth/desktop/done?state=${state}`);
  }));
test('duplicate desktop state is a bad request and done page is not cacheable', () =>
  withData(async (data) => {
    const env = makeEnv(data), browser = await seedSession(data, env), app = createApp({ data: () => data });
    env.GOOGLE_CLIENT_ID = 'test-google-client';
    env.GOOGLE_CLIENT_SECRET = 'test-google-secret';
    const state = crypto.randomUUID().replaceAll('-', '');
    const challenge = b64(crypto.getRandomValues(new Uint8Array(32)));
    const path = `/v1/auth/desktop?provider=google&state=${state}&challenge=${challenge}`;
    expect((await app.request(path, {}, env)).status).toBe(302);
    const duplicate = await app.request(path, {}, env);
    expect(duplicate.status).toBe(400);
    expect(await duplicate.json() as any).toEqual({ error: 'bad_request' });
    const doneState = crypto.randomUUID().replaceAll('-', '');
    await data.db.insert(desktopAuth).values({ state: doneState, challenge, expiresAt: new Date(Date.now() + 600000) });
    const done = await app.request(`/v1/auth/desktop/done?state=${doneState}`, { headers: browser.headers }, env);
    expect(done.status).toBe(200);
    expect(done.headers.get('Cache-Control')).toBe('no-store');
    expect(done.headers.get('Referrer-Policy')).toBe('no-referrer');
  }));

test('desktop handoff token is not written to application logs', () =>
  withData(async (data) => {
    const env = makeEnv(data), browser = await seedSession(data, env);
    const calls: unknown[][] = [];
    const previous = console.log;
    console.log = (...args) => {
      calls.push(args);
    };
    let token: string;
    try {
      token = (await setup(data, env, browser.headers)).token;
    } finally {
      console.log = previous;
    }
    expect(token!).toBeTruthy();
    expect(JSON.stringify(calls)).not.toContain(token!);
  }));
