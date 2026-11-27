import { Hono } from 'hono';
import type { HonoBase } from 'hono/hono-base';
import { HTTPException } from 'hono/http-exception';
import { ApiError } from '@powermove/registry/wire';
import { ZodError } from 'zod';
import type { Env } from './env';
import type { Data } from './db/client';
import { createAuth } from './auth';
import { human } from './turnstile';
import { health } from './routes/health';
import { desktop, web } from './routes/auth-desktop';
import { email } from './routes/auth-email';
import { signout } from './routes/auth-signout';
import { me } from './routes/me';
import { objectRoutes } from './routes/objects';
import { type PublishDeps, publishRoutes } from './routes/publish';
import { repoManagement, storeUtility } from './routes/repo-management';
import { adminRoutes } from './routes/admin';
import { storeRoutes } from './routes/store';
import { installRoutes } from './routes/installs';
import { billing } from './routes/billing';
import { stripeWebhook } from './routes/stripe-webhook';
import { aero } from './routes/aero';
import { V2Error } from './v2-error';
import { productionProblems } from './production';
export const MIN_DESKTOP_VERSION = '0.0.0';
export function createApp(deps: {
  data: (env: CloudflareBindings) => Data;
  beforeCommit?: PublishDeps['beforeCommit'];
}) {
  const app = new Hono<Env>();
  app.use('*', async (c, next) => {
    // A production Worker with development settings (or without its live keys) serves nothing at all.
    const problems = c.env ? productionProblems(c.env) : [];
    if (problems.length) {
      console.error('refusing to serve: production misconfigured', problems.join(','));
      return c.json({ error: { code: 'misconfigured' } }, 503);
    }
    await next();
  });
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
    c.set('openData', () => deps.data(c.env));
    try {
      await next();
    } finally {
      await data.end();
    }
  });
  app.use('*', async (c, next) => {
    c.set('session', null);
    const authorization = c.req.header('Authorization');
    if (c.var.data && authorization?.startsWith('Bearer ')) {
      try {
        // The bearer alone: no cookie (or any other header) can stand in for it, even when the bearer is invalid.
        const result = await createAuth(c.var.data, c.env).api.getSession({ headers: new Headers({ authorization }) });
        c.set('session', result?.session ?? null);
      } catch {
        /* invalid bearer remains anonymous */
      }
    }
    await next();
  });
  app.onError((err, c) => {
    // v2 answers with a code only: `{"error": {"code": "..."}}`, never the backend's own text.
    if (err instanceof V2Error) {
      if (err.retryAfter !== undefined) c.header('Retry-After', String(err.retryAfter));
      return c.json({ error: { code: err.code } }, err.status);
    }
    if (c.req.path.startsWith('/v2/')) {
      if (err instanceof ApiError) {
        if (err.status === 429 && !c.res.headers.has('Retry-After')) c.header('Retry-After', '60');
        return c.json({ error: { code: err.body.error } }, err.status as 400);
      }
      if (err instanceof HTTPException || err instanceof ZodError) return c.json({ error: { code: 'bad_request' } }, 400);
      console.error('v2', c.req.path, (err as Error)?.name);
      return c.json({ error: { code: 'internal' } }, 500);
    }
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
  app.notFound((c) => c.req.path.startsWith('/v2/') ? c.json({ error: { code: 'not_found' } }, 404) : c.json({ error: 'not_found' }, 404));
  const routes = app.route('/v1/human', human).route('/health', health).route('/v1/auth/desktop', desktop).route('/v1/auth/web', web).route('/v1/auth/email', email).route(
    '/v1/auth/sign-out',
    signout,
  ).route('/v1/me', me).route('/v1/objects', objectRoutes).route(
    '/v1/repos',
    publishRoutes({ beforeCommit: deps.beforeCommit }),
  ).route('/v1/repos', repoManagement).route('/v1/admin', adminRoutes).route('/v1/store', storeRoutes).route(
    '/v1/store',
    storeUtility,
  ).route('/v1/installs', installRoutes).route('/v2/billing', billing).route('/v2/stripe/webhook', stripeWebhook).route('/v2/aero', aero);
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
