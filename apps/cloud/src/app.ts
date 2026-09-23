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
export const MIN_DESKTOP_VERSION = '0.0.0';
export function createApp(deps: { data: (env: CloudflareBindings) => Data }) {
  const app = new Hono<Env>();
  app.use('*', async (c, next) => { c.set('requestId', crypto.randomUUID()); c.header('X-Request-Id', c.var.requestId); await next(); });
  app.use('*', async (c, next) => {
    const client = c.req.header('X-Powermove-Client');
    if (client) console.info('X-Powermove-Client', client);
    if (client?.startsWith('desktop/')) { const v = client.slice(8).split('.').map(Number), min = MIN_DESKTOP_VERSION.split('.').map(Number); if (v.length !== 3 || v.some(n => !Number.isInteger(n) || n < 0) || min.some((n,i) => (v[i] ?? 0) < n && min.slice(0,i).every((m,j) => v[j] === m))) throw new ApiError({ error: 'client_too_old', minimum: MIN_DESKTOP_VERSION }); }
    await next();
  });
  app.use('*', async (c, next) => { if (!c.env || c.req.path === '/health' || c.req.path === '/health/') { await next(); return; } const data = deps.data(c.env); c.set('data', data); try { await next(); } finally { await data.end(); } });
  app.use('*', async (c, next) => { c.set('session', null); if (c.var.data && c.req.header('Authorization')?.startsWith('Bearer ')) { try { const result = await createAuth(c.var.data, c.env).api.getSession({ headers: c.req.raw.headers }); c.set('session', result?.session ?? null); } catch { /* invalid bearer remains anonymous */ } } await next(); });
  app.onError((err,c) => { if (err instanceof ApiError) return c.json(err.body, err.status as 400); if (err instanceof HTTPException) return err.getResponse(); if (err instanceof ZodError) return c.json({ error: 'bad_request', detail: err.message },400); console.error(err); return c.json({ error: 'internal' },500); });
  app.notFound(c => c.json({ error: 'not_found' },404));
  const routes = app.route('/health', health).route('/v1/auth/desktop', desktop).route('/v1/auth/email', email).route('/v1/auth/sign-out', signout).route('/v1/me', me);
  app.on(['GET','POST'], '/v1/auth/*', c => createAuth(c.var.data,c.env).handler(c.req.raw));
  return routes;
}
type Routes = ReturnType<typeof createApp>;
export type AppType = Routes extends HonoBase<any, infer S, infer P> ? HonoBase<{}, S, P> : never;
