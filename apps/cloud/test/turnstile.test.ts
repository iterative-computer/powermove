import { expect, test, spyOn } from 'bun:test';
import { createApp } from '../src/app';
import { withData } from './db';
import { makeEnv, FakeEmail } from './env';
import { files, publisher, publishRequest, uploadTree } from './publish-fixture';

const post = (body: unknown, clearance?: string) => ({ method: 'POST', headers: { 'Content-Type': 'application/json', ...(clearance ? { 'X-Powermove-Human': clearance } : {}) }, body: JSON.stringify(body) });
const configure = (env: CloudflareBindings) => { env.TURNSTILE_SITE_KEY = 'real-site-key'; env.TURNSTILE_SECRET_KEY = 'real-secret-key'; };
const digest = async (ticket: string) => Buffer.from(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(ticket))).toString('base64url');

test('email clearance is verified, bound to identity/action, expires, and never exposes email to widget', () => withData(async data => {
  const env = makeEnv(data); configure(env);
  const app = createApp({ data: () => data });
  const request = (email: string, clearance?: string) => app.request('/v1/auth/email/send', post({ email }, clearance), env);
  const required = await request('alice@example.com');
  expect(required.status).toBe(403);
  const challenge = await required.json() as any;
  expect((env.EMAIL as unknown as FakeEmail).outbox).toHaveLength(0);
  const page = await app.request(`/v1/human?ticket=${challenge.ticket}`, {}, env);
  const html = await page.text();
  expect(html).not.toContain('alice@example.com');
  expect(html).not.toContain('real-secret-key');
  expect(html).toContain("appearance:'interaction-only'");
  expect(page.headers.get('content-security-policy')).toContain("frame-ancestors 'none'");
  const expected = { success: true, hostname: 'cloud.trypowermove.com', action: 'email_send', cdata: await digest(challenge.ticket) };
  const verify = spyOn(globalThis, 'fetch').mockResolvedValue(Response.json(expected));
  try {
    const solved = await app.request('/v1/human/verify', post({ ticket: challenge.ticket, token: 'test-token' }), env);
    expect(solved.status).toBe(200);
    const receipt = await app.request('/v1/human/result', post({ ticket: challenge.ticket }), env);
    const { clearance } = await receipt.json() as any;
    expect((await app.request('/v1/human/result', post({ ticket: challenge.ticket }), env)).status).toBe(204);
    expect((await request('alice@example.com', clearance)).status).toBe(200);
    expect((await request('bob@example.com', clearance)).status).toBe(403);
    expect((await request('alice@example.com', `${clearance}tampered`)).status).toBe(403);
    expect((await request('alice@example.com', challenge.ticket)).status).toBe(403);
    const date = spyOn(Date, 'now').mockReturnValue(Date.now() + 31 * 86_400_000);
    try { expect((await request('alice@example.com', clearance)).status).toBe(403); } finally { date.mockRestore(); }
    for (const result of [{ ...expected, success: false }, { ...expected, hostname: 'evil.test' }, { ...expected, action: 'first_publish' }, { ...expected, cdata: 'wrong' }]) {
      verify.mockResolvedValueOnce(Response.json(result));
      expect((await app.request('/v1/human/verify', post({ ticket: challenge.ticket, token: 'test-token' }), env)).status).toBe(400);
    }
  } finally { verify.mockRestore(); }
}));

test('missing configuration fails closed when enabled; disabled development stays usable', () => withData(async data => {
  const env = makeEnv(data), app = createApp({ data: () => data });
  env.TURNSTILE_ENABLED = '1';
  expect((await app.request('/v1/auth/email/send', post({ email: 'test@example.com' }), env)).status).toBe(500);
  delete env.TURNSTILE_ENABLED;
  expect((await app.request('/v1/auth/email/send', post({ email: 'test@example.com' }), env)).status).toBe(200);
  env.TURNSTILE_SITE_KEY = '1x00000000000000000000AA'; env.TURNSTILE_SECRET_KEY = '1x0000000000000000000000000000000AA';
  expect((await app.request('/v1/auth/email/send', post({ email: 'test@example.com' }), env)).status).toBe(500);
}));

test('first publisher is gated; established publisher releases stay frictionless', () => withData(async data => {
  const env = makeEnv(data), actor = await publisher(data, env, 'alice');
  const built = await uploadTree(data, env, actor, files());
  configure(env);
  const gated = await publishRequest(data, env, actor, 'demo', built.commitSha);
  expect(gated.status).toBe(403);
  expect((await gated.json() as any).scope).toStartWith('first_publish:');
  expect((await env.TARS.list()).objects).toHaveLength(0);
  delete env.TURNSTILE_SITE_KEY; delete env.TURNSTILE_SECRET_KEY;
  expect((await publishRequest(data, env, actor, 'demo', built.commitSha)).status).toBe(201);
  configure(env);
  const next = await uploadTree(data, env, actor, files('demo', '1.0.1'), { parents: [built.commitSha] });
  expect((await publishRequest(data, env, actor, 'demo', next.commitSha, { version: '1.0.1' })).status).toBe(201);
}));
