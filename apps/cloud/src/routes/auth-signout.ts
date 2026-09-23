import { Hono } from 'hono';
import type { Env } from '../env';
import { createAuth } from '../auth';
import { requireSession } from './session';
export const signout = new Hono<Env>().post('/', async c => { requireSession(c); await createAuth(c.var.data,c.env).api.signOut({ headers: c.req.raw.headers }); return c.json({ ok: true as const }); });
