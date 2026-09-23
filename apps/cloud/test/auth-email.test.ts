import { expect, test } from 'bun:test';
import { createApp } from '../src/app';
import { withData } from './db';
import { makeEnv } from './env';
test('email OTP signs in and returns a usable bearer', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    let otp = '';
    env.OTP_SENDER = async (_email, code) => {
      otp = code;
    };
    {
      const sent = await app.request('/v1/auth/email/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: 'otp@example.com' }),
      }, env);
      expect(sent.status).toBe(200);
    }
    expect(otp).toBeTruthy();
    const verified = await app.request('/v1/auth/email/verify', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: 'otp@example.com', otp }),
    }, env);
    expect(verified.status).toBe(200);
    const session = await verified.json() as {
      token: string;
      expiresAt: string;
    };
    expect(session.token).toBeTruthy();
    const me = await app.request('/v1/me', { headers: { Authorization: `Bearer ${session.token}` } }, env);
    expect(me.status).toBe(200);
    const signout = await app.request('/v1/auth/sign-out', {
      method: 'POST',
      headers: { Authorization: `Bearer ${session.token}` },
    }, env);
    expect(signout.status).toBe(200);
    expect(await signout.json() as any).toEqual({ ok: true });
    const after = await app.request('/v1/me', { headers: { Authorization: `Bearer ${session.token}` } }, env);
    expect(after.status).toBe(401);
  }));
test('OTP delivery without a configured sender fails without logging a code', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    const calls: unknown[][] = [];
    const previous = console.log;
    console.log = (...args) => {
      calls.push(args);
    };
    try {
      const response = await app.request('/v1/auth/email/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: 'nodelivery@example.com' }),
      }, env);
      expect(response.status).toBe(400);
      expect(await response.json() as any).toEqual({ error: 'bad_request' });
    } finally {
      console.log = previous;
    }
    expect(calls).toHaveLength(0);
  }));

test('OTP sender failure becomes bad_request', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    env.OTP_SENDER = async () => {
      throw new Error('delivery failed');
    };
    const response = await app.request('/v1/auth/email/send', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: 'failure@example.com' }),
    }, env);
    expect(response.status).toBe(400);
    expect((await response.json() as { error: string }).error).toBe('bad_request');
  }));
