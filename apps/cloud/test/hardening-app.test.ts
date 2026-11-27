/* App-level hardening: production refuses development settings, and API routes trust the bearer alone. */
import { expect, setSystemTime, test } from 'bun:test';
import { makeSignature } from 'better-auth/crypto';
import { withData } from './db';
import { makeEnv } from './env';
import { seedSession } from './helpers';
import { createApp } from '../src/app';
import { DEVELOPMENT_ONLY, productionProblems } from '../src/production';

/** What a well-configured production Worker has. */
const production = (data: Parameters<typeof makeEnv>[0]): CloudflareBindings => ({
  ...makeEnv(data), ENVIRONMENT: 'production', TURNSTILE_ENABLED: '1', TURNSTILE_SITE_KEY: '0x4AAAAAAFEFmLgsfY1Hnm3q', TURNSTILE_SECRET_KEY: '0x4AAAAAAFlivesecretlivesecret',
  STRIPE_SECRET_KEY: 'sk_live_x', STRIPE_WEBHOOK_SECRET: 'whsec_configured', STRIPE_PRICE_AEROLITE: 'price_live_aerolite',
});

test('6. production refuses to serve with any development setting, or without its live keys and prices', async () =>
  withData(async (data) => {
    const app = createApp({ data: () => data });
    const good = production(data);
    expect(productionProblems(good)).toEqual([]);
    expect((await app.request('/health', {}, good)).status).toBe(200);
    for (const name of DEVELOPMENT_ONLY) {
      const r = await app.request('/health', {}, { ...good, [name]: name === 'OTP_SENDER' ? (async () => {}) : '1' } as CloudflareBindings);
      expect([name, r.status]).toEqual([name, 503]);
      expect(await r.json() as unknown).toEqual({ error: { code: 'misconfigured' } });
    }
    for (const [name, value] of [['STRIPE_PRICE_AEROLITE', undefined], ['STRIPE_WEBHOOK_SECRET', undefined], ['TURNSTILE_SECRET_KEY', undefined], ['TURNSTILE_SITE_KEY', '1x00000000000000000000AA'], ['STRIPE_SECRET_KEY', 'sk_test_x'], ['TURNSTILE_ENABLED', '0']] as const) {
      const r = await app.request('/v1/store/featured', {}, { ...good, [name]: value } as CloudflareBindings);
      expect([name, r.status]).toEqual([name, 503]);
    }
    // Development is unaffected.
    expect((await app.request('/health', {}, { ...good, ENVIRONMENT: 'development', DEV_LOG_OTP: '1' })).status).toBe(200);
  }));

test('11. API routes take the bearer alone: a session cookie is never a fallback, not even beside an invalid bearer', async () =>
  withData(async (data) => {
    const env = makeEnv(data);
    const app = createApp({ data: () => data });
    const user = await seedSession(data, env);
    const signed = `${user.token}.${await makeSignature(user.token, env.BETTER_AUTH_SECRET)}`;
    const cookie = [`better-auth.session_token=${encodeURIComponent(signed)}`, `__Secure-better-auth.session_token=${encodeURIComponent(signed)}`].join('; ');
    expect((await app.request('/v1/me', { headers: user.headers }, env)).status).toBe(200);
    for (const path of ['/v1/me', '/v2/billing']) {
      expect([path, (await app.request(path, { headers: { cookie } }, env)).status]).toEqual([path, 401]);
      // Bearers that better-auth would otherwise skip in favour of the cookie: empty, blank, or dotted and invalid.
      for (const authorization of ['Bearer not-a-real-token', 'Bearer ', 'Bearer  ', 'Bearer a.b']) {
        expect([path, authorization, (await app.request(path, { headers: { cookie, authorization } }, env)).status]).toEqual([path, authorization, 401]);
      }
    }
  }));

const send = (app: ReturnType<typeof createApp>, env: CloudflareBindings, email: string, ip = '203.0.113.7') =>
  app.request('/v1/auth/email/send', { method: 'POST', headers: { 'Content-Type': 'application/json', 'cf-connecting-ip': ip }, body: JSON.stringify({ email }) }, env);

test('8. sign-in codes have a daily budget per address and a global one below the mail quota', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    const start = Date.parse('2026-11-26T01:00:00Z');
    try {
      // Five a day to one address, even spread across the short (15-minute) windows.
      const statuses: number[] = [];
      for (let i = 0; i < 6; i++) {
        setSystemTime(new Date(start + i * 16 * 60_000));
        statuses.push((await send(app, env, 'budget@example.com', `203.0.113.${i + 1}`)).status);
      }
      expect(statuses).toEqual([200, 200, 200, 200, 200, 429]);
      // The global budget: EMAIL_DAILY_LIMIT (default 150) across every address and IP.
      const limited = { ...env, EMAIL_DAILY_LIMIT: '7' };
      setSystemTime(new Date(start + 3 * 3_600_000));
      const global: number[] = [];
      for (let i = 0; i < 3; i++) global.push((await send(app, limited, `someone${i}@example.com`, `198.51.100.${i + 1}`)).status);
      expect(global).toEqual([200, 200, 429]);
    } finally { setSystemTime(); }
  }));

test('8. the direct OAuth routes share the auth routes’ per-IP edge limit', async () =>
  withData(async (data) => {
    const env = { ...makeEnv(data), RL_AUTH: { limit: async () => ({ success: false }) } as RateLimit };
    const app = createApp({ data: () => data });
    expect((await app.request('/v1/auth/sign-in/oauth2', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify({ providerId: 'google' }) }, env)).status).toBe(429);
    expect((await app.request('/v1/auth/oauth2/callback/google?code=x&state=y', {}, env)).status).toBe(429);
  }));

test('without a usable AERO_PRICING only Aero refuses (503 misconfigured); health, the Store, /v1 and billing keep working', async () =>
  withData(async (data) => {
    const app = createApp({ data: () => data });
    for (const pricing of [undefined, '{"version":"x"}']) {
      for (const env of [{ ...production(data), AERO_PRICING: pricing }, { ...makeEnv(data), AERO_PRICING: pricing }] as CloudflareBindings[]) {
        expect(productionProblems(env)).toEqual([]);
        const user = await seedSession(data, env, `p${crypto.randomUUID().slice(0, 8)}@example.com`);
        expect((await app.request('/health', {}, env)).status).toBe(200);
        expect((await app.request('/v1/store/browse', {}, env)).status).toBe(200);
        expect((await app.request('/v1/me', { headers: user.headers }, env)).status).toBe(200);
        const bill = await app.request('/v2/billing', { headers: user.headers }, env);
        expect(bill.status).toBe(200);
        expect((await bill.json() as any).credits).toEqual({ included: 0, used: 0, remaining: 0 });
        for (const [path, init] of [['/v2/aero/models', {}], ['/v2/aero/chat/completions', { method: 'POST', body: '{}' }]] as const) {
          const r = await app.request(path, { ...init, headers: user.headers }, env);
          expect([path, r.status]).toEqual([path, 503]);
          expect(await r.json() as unknown).toEqual({ error: { code: 'misconfigured' } });
        }
      }
    }
  }));
