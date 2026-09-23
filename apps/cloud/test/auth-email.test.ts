import { expect, test } from 'bun:test';
import { createApp } from '../src/app';
import { withData } from './db';
import { FakeEmail, makeEnv } from './env';
test('email OTP signs in and returns a usable bearer', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    {
      const sent = await app.request('/v1/auth/email/send', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: 'otp@example.com' }),
      }, env);
      expect(sent.status).toBe(200);
    }
    const message = (env.EMAIL as FakeEmail).outbox[0]!;
    expect(message.to).toBe('otp@example.com');
    expect(message.from).toBe(env.EMAIL_FROM);
    expect(message.subject).toBe('Your Powermove sign-in code');
    const otp = message.text.match(/\b\d{6}\b/)?.[0];
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
    delete (env as Partial<CloudflareBindings>).EMAIL;
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
      expect(await response.json() as any).toEqual({ error: 'bad_request', detail: "Couldn't send the code. Try again." });
    } finally {
      console.log = previous;
    }
    expect(calls).toHaveLength(0);
  }));

test('OTP sender failure becomes bad_request', () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    env.EMAIL = { send: async () => { throw Object.assign(new Error('delivery failed'), { code: 'E_RATE_LIMIT_EXCEEDED' }); } };
    const response = await app.request('/v1/auth/email/send', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ email: 'failure@example.com' }),
    }, env);
    expect(response.status).toBe(400);
    expect((await response.json() as { error: string }).error).toBe('bad_request');
  }));
