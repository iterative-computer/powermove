/* App-level hardening: production refuses development settings, and API routes trust the bearer alone. */
import { expect, test } from 'bun:test';
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
      expect([path, (await app.request(path, { headers: { cookie, authorization: 'Bearer not-a-real-token' } }, env)).status]).toEqual([path, 401]);
    }
  }));
