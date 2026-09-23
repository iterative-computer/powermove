import { expect, test } from 'bun:test';
import { user } from '../src/db/auth-schema';
import { abuseCounters, reports } from '../src/db/schema';
import { consume } from '../src/abuse';
import { createApp } from '../src/app';
import { withData } from './db';
import { makeEnv } from './env';
import { files, publisher, publishRequest, uploadTree } from './publish-fixture';

const post = (body: unknown) => ({ method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });

test('counter is exact at the limit, hashes keys, and rolls over', () => withData(async data => {
  const start = new Date('2026-09-23T12:00:00.000Z');
  const results = await Promise.all(Array.from({ length: 4 }, () => consume(data.db, 'email_send_addr', 'secret@example.com', start)));
  expect(results.filter(r => r.ok)).toHaveLength(3);
  expect(results.filter(r => !r.ok)).toHaveLength(1);
  expect(results.find(r => !r.ok)).toEqual({ ok: false, retryAfterSeconds: 900 });
  const [row] = await data.db.select().from(abuseCounters);
  expect(row?.key).toMatch(/^[0-9a-f]{32}$/);
  expect(row?.key).not.toContain('secret');
  expect(await consume(data.db, 'email_send_addr', 'secret@example.com', new Date('2026-09-23T12:15:00.000Z')))
    .toEqual({ ok: true, remaining: 2 });
}));

test('fourth email send is 429 and known and unknown addresses get the same response', () => withData(async data => {
  const env = makeEnv(data), app = createApp({ data: () => data });
  await data.db.insert(user).values({ id: crypto.randomUUID(), name: 'Known', email: 'known@example.com', emailVerified: true });
  const known = await app.request('/v1/auth/email/send', post({ email: 'known@example.com' }), env);
  const unknown = await app.request('/v1/auth/email/send', post({ email: 'unknown@example.com' }), env);
  expect(await known.json() as any).toEqual({ ok: true });
  expect(await unknown.json() as any).toEqual({ ok: true });
  for (let i = 0; i < 2; i++) expect((await app.request('/v1/auth/email/send', post({ email: 'known@example.com' }), env)).status).toBe(200);
  const blocked = await app.request('/v1/auth/email/send', post({ email: 'known@example.com' }), env);
  expect(blocked.status).toBe(429);
  expect(blocked.headers.get('Retry-After')).toBeTruthy();
  expect(await blocked.json() as any).toEqual({ error: 'rate_limited' });
}));

test('verification is neutral for wrong codes and locked after five requests', () => withData(async data => {
  const env = makeEnv(data), app = createApp({ data: () => data });
  expect((await app.request('/v1/auth/email/send', post({ email: 'verify@example.com' }), env)).status).toBe(200);
  for (let i = 0; i < 5; i++) {
    const response = await app.request('/v1/auth/email/verify', post({ email: 'verify@example.com', otp: '000000' }), env);
    expect(response.status).toBe(400);
    expect(await response.json() as any).toEqual({ error: 'bad_request', detail: "Couldn't verify the code. Try again." });
  }
  const blocked = await app.request('/v1/auth/email/verify', post({ email: 'verify@example.com', otp: '000000' }), env);
  expect(blocked.status).toBe(429);
  expect(blocked.headers.get('Retry-After')).toBeTruthy();
}));

test('21st publish is blocked before tar write', () => withData(async data => {
  const env = makeEnv(data), actor = await publisher(data, env, 'alice');
  const built = await uploadTree(data, env, actor, files());
  for (let i = 0; i < 20; i++) expect((await consume(data.db, 'publish_user_hour', actor.id)).ok).toBe(true);
  const response = await publishRequest(data, env, actor, 'demo', built.commitSha);
  expect(response.status).toBe(429);
  expect(response.headers.get('Retry-After')).toBeTruthy();
  expect(await response.json() as any).toEqual({ error: 'rate_limited' });
  expect((await env.TARS.list()).objects).toHaveLength(0);
}));

test('one report per repository per IP each day', () => withData(async data => {
  const env = makeEnv(data), actor = await publisher(data, env, 'alice');
  const built = await uploadTree(data, env, actor, files());
  expect((await publishRequest(data, env, actor, 'demo', built.commitSha)).status).toBe(201);
  const app = createApp({ data: () => data });
  const first = await app.request('/v1/store/x/alice/demo/report', post({ reason: 'problem' }), env);
  expect(first.status).toBe(204);
  const second = await app.request('/v1/store/x/alice/demo/report', post({ reason: 'problem' }), env);
  expect(second.status).toBe(429);
  expect(second.headers.get('Retry-After')).toBeTruthy();
  expect(await second.json() as any).toEqual({ error: 'rate_limited' });
  expect(await data.db.select().from(reports)).toHaveLength(1);
}));
