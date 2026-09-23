import { Hono } from 'hono';
import type { HonoBase } from 'hono/hono-base';
import { HTTPException } from 'hono/http-exception';
import { ApiError } from '@powermove/registry/wire';
import { ZodError } from 'zod';
import type { Env } from './env';
import type { Data } from './db/client';
import { createAuth } from './auth';
import { health } from './routes/health';
import { desktop } from './routes/auth-desktop';
import { email } from './routes/auth-email';
import { signout } from './routes/auth-signout';
import { me } from './routes/me';
import { objectRoutes } from './routes/objects';
import { type PublishDeps, publishRoutes } from './routes/publish';
import { adminRoutes, repoManagement, storeUtility } from './routes/repo-management';
import { storeRoutes } from './routes/store';
import { installRoutes } from './routes/installs';
export const MIN_DESKTOP_VERSION = '0.0.0';
export function createApp(deps: {
  data: (env: CloudflareBindings) => Data;
  beforeCommit?: PublishDeps['beforeCommit'];
}) {
  const app = new Hono<Env>();
  app.use('*', async (c, next) => {
    c.set('requestId', crypto.randomUUID());
    c.header('X-Request-Id', c.var.requestId);
    await next();
  });
  app.use('*', async (c, next) => {
    const client = c.req.header('X-Powermove-Client');
    const version = client?.match(/^desktop\/(\d+)\.(\d+)\.(\d+)(?:-[0-9A-Za-z.-]+)?$/);
    if (version) {
      const current = version.slice(1, 4).map(Number);
      const minimum = MIN_DESKTOP_VERSION.split('.').map(Number);
      console.info('desktop client', current.join('.'));
      if (
        current.some((part, i) => part < minimum[i]! && current.slice(0, i).every((prior, j) => prior === minimum[j]))
      ) {
        throw new ApiError({ error: 'client_too_old', minimum: MIN_DESKTOP_VERSION });
      }
    }
    await next();
  });
  app.use('*', async (c, next) => {
    if (!c.env || c.req.path === '/health' || c.req.path === '/health/') {
      await next();
      return;
    }
    const data = deps.data(c.env);
    c.set('data', data);
    try {
      await next();
    } finally {
      await data.end();
    }
  });
  app.use('*', async (c, next) => {
    c.set('session', null);
    if (c.var.data && c.req.header('Authorization')?.startsWith('Bearer ')) {
      try {
        const result = await createAuth(c.var.data, c.env).api.getSession({ headers: c.req.raw.headers });
        c.set('session', result?.session ?? null);
      } catch {
        /* invalid bearer remains anonymous */
      }
    }
    await next();
  });
  app.onError((err, c) => {
    if (err instanceof ApiError) {
      if (err.status === 429 && !c.res.headers.has('Retry-After')) c.header('Retry-After', '60');
      return c.json(err.body, err.status as 400);
    }
    if (err instanceof HTTPException) {
      return c.json({ error: 'bad_request', detail: err.message }, 400);
    }
    if (err instanceof ZodError) {
      return c.json({ error: 'bad_request', detail: err.message }, 400);
    }
    console.error(err);
    return c.json({ error: 'internal' }, 500);
  });
  app.notFound((c) => c.json({ error: 'not_found' }, 404));
  const routes = app.route('/health', health).route('/v1/auth/desktop', desktop).route('/v1/auth/email', email).route(
    '/v1/auth/sign-out',
    signout,
  ).route('/v1/me', me).route('/v1/objects', objectRoutes).route(
    '/v1/repos',
    publishRoutes({ beforeCommit: deps.beforeCommit }),
  ).route('/v1/repos', repoManagement).route('/v1/admin', adminRoutes).route('/v1/store', storeRoutes).route(
    '/v1/store',
    storeUtility,
  ).route('/v1/installs', installRoutes);
  app.post('/v1/auth/sign-in/oauth2', async (c) => {
    const body = await c.req.json().catch(() => null) as { providerId?: unknown; callbackURL?: unknown } | null;
    if (body?.providerId !== 'google') return c.json({ error: 'not_found' }, 404);
    const url = new URL(c.req.url);
    url.pathname = '/v1/auth/sign-in/social';
    return createAuth(c.var.data, c.env).handler(new Request(url, {
      method: 'POST', headers: c.req.raw.headers,
      body: JSON.stringify({ provider: 'google', callbackURL: body.callbackURL }),
    }));
  });
  app.get('/v1/auth/oauth2/callback/:providerId', (c) => {
    if (c.req.param('providerId') !== 'google') return c.json({ error: 'not_found' }, 404);
    const url = new URL(c.req.url);
    url.pathname = '/v1/auth/callback/google';
    return createAuth(c.var.data, c.env).handler(new Request(url, c.req.raw));
  });
  app.get('/v1/auth/error', (c) => createAuth(c.var.data, c.env).handler(c.req.raw));
  app.all('/v1/auth/*', (c) => c.json({ error: 'not_found' }, 404));
  return routes;
}
type Routes = ReturnType<typeof createApp>;
export type AppType = Routes extends HonoBase<any, infer S, infer P> ? HonoBase<{}, S, P> : never;
