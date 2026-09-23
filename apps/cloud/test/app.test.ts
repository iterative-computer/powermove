import { expect, test } from 'bun:test';
import { createApp } from '../src/app';
import { withData } from './db';
import { makeEnv } from './env';
test('authorization, client hint and health errors', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    let r = await app.request('/v1/me', {}, env);
    expect(r.status).toBe(401);
    expect(await r.json() as any).toEqual({ error: 'unauthorized' });
    r = await app.request('/v1/auth/email/send', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: 'bad' }),
    }, env);
    expect(r.status).toBe(400);
    expect((await r.json() as any).error).toBe('bad_request');
    r = await app.request('/health', { headers: { 'X-Powermove-Client': 'desktop/0.0.0' } }, env);
    expect(r.status).toBe(200);
    r = await app.request('/health', { headers: { 'X-Powermove-Client': 'desktop/broken' } }, env);
    expect(r.status).toBe(200);
    const failing = createApp({
      data: () => {
        throw new Error('test error');
      },
    });
    const old = console.error;
    console.error = () => {
    };
    try {
      r = await failing.request('/v1/me', {}, env);
    } finally {
      console.error = old;
    }
    expect(r.status).toBe(500);
    expect(await r.json() as any).toEqual({ error: 'internal' });
  }));
test('auth passthrough denies one-time-token and native OTP endpoints', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    for (
      const path of [
        '/v1/auth/one-time-token/verify',
        '/v1/auth/email-otp/send-verification-otp',
        '/v1/auth/sign-in/email-otp',
      ]
    ) {
      const response = await app.request(path, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{}',
      }, env);
      expect(response.status).toBe(404);
      expect(await response.json() as any).toEqual({ error: 'not_found' });
    }
  }));
test('malformed JSON and settings validation use the API error shape', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    for (const path of ['/v1/auth/email/send', '/v1/auth/email/verify', '/v1/auth/desktop/exchange']) {
      const response = await app.request(path, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: '{',
      }, env);
      expect(response.status).toBe(400);
      expect(
        (await response.json() as {
          error: string;
        }).error,
      ).toBe('bad_request');
    }
    const { seedSession } = await import('./helpers');
    const session = await seedSession(data, env);
    const response = await app.request('/v1/me/settings', {
      method: 'PATCH',
      headers: { ...session.headers, 'Content-Type': 'application/json' },
      body: '{"rememberInstalls":"yes"}',
    }, env);
    expect(response.status).toBe(400);
    expect(
      (await response.json() as {
        error: string;
      }).error,
    ).toBe('bad_request');
  }));
