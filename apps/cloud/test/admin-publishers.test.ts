import { expect, test } from 'bun:test';
import { eq } from 'drizzle-orm';
import { createApp } from '../src/app';
import { publishers } from '../src/db/schema';
import { withData } from './db';
import { makeEnv } from './env';
import { seedSession } from './helpers';

const json = async (res: Response): Promise<any> => res.json();

test('an operator seeds a reserved handle for an existing user; nobody else can', () =>
  withData(async (data) => {
    const env = makeEnv(data);
    env.ADMIN_TOKEN = 'test-admin-token';
    const app = createApp({ data: () => data });
    const s = await seedSession(data, env);
    const post = (body: unknown, token?: string) =>
      app.request(
        '/v1/admin/publishers',
        {
          method: 'POST',
          headers: { 'content-type': 'application/json', ...(token ? { 'X-Admin-Token': token } : {}) },
          body: JSON.stringify(body)
        },
        env
      );

    // The reserved list still applies to the user's own claim.
    const own = await app.request(
      '/v1/me/handle',
      { method: 'POST', headers: { ...s.headers, 'content-type': 'application/json' }, body: JSON.stringify({ handle: 'powermove' }) },
      env
    );
    expect(own.status).toBe(400);
    expect((await json(own)).error).toBe('handle_reserved');

    // No token, wrong token → 401 and nothing written.
    expect((await post({ handle: 'powermove', userId: s.id })).status).toBe(401);
    expect((await post({ handle: 'powermove', userId: s.id }, 'nope')).status).toBe(401);
    expect(await data.db.select().from(publishers)).toHaveLength(0);

    // Unknown user → 404.
    expect((await post({ handle: 'powermove', userId: 'missing-user' }, 'test-admin-token')).status).toBe(404);

    // Seeded.
    const ok = await post({ handle: 'powermove', userId: s.id }, 'test-admin-token');
    expect(ok.status).toBe(201);
    expect((await json(ok)).publisher.handle).toBe('powermove');
    const [row] = await data.db.select().from(publishers).where(eq(publishers.userId, s.id));
    expect(row?.handle).toBe('powermove');

    // GET /v1/me sees it; a second seed for the same user is refused; the handle is taken for others.
    const me = await app.request('/v1/me', { headers: s.headers }, env);
    expect((await json(me)).publisher.handle).toBe('powermove');
    expect((await json(await post({ handle: 'official', userId: s.id }, 'test-admin-token'))).error).toBe('handle_already_set');
    const other = await seedSession(data, env, 'other@example.com');
    expect((await json(await post({ handle: 'powermove', userId: other.id }, 'test-admin-token'))).error).toBe('handle_taken');
  }));
