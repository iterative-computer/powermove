import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { ApiError, Auth } from '@powermove/registry/wire';
import type { Env } from '../env';
import { createAuth } from '../auth';
import { rateAuth } from './session';
import { session as sessionTable } from '../db/auth-schema';
import { eq } from 'drizzle-orm';
export const email = new Hono<Env>()
 .post('/send', zValidator('json', Auth.EmailSend.Req.shape.body, result => { if (!result.success) throw result.error; }), async c => { await rateAuth(c); const { email } = c.req.valid('json'); try { await createAuth(c.var.data,c.env).api.sendVerificationOTP({ body: { email, type: 'sign-in' }, headers: c.req.raw.headers }); } catch { throw new ApiError({ error: 'bad_request' }); } return c.json({ ok: true as const }); })
 .post('/verify', zValidator('json', Auth.EmailVerify.Req.shape.body, result => { if (!result.success) throw result.error; }), async c => { const { email, otp } = c.req.valid('json'); try { const result = await createAuth(c.var.data,c.env).api.signInEmailOTP({ body: { email, otp }, headers: c.req.raw.headers }); const [session] = await c.var.data.db.select().from(sessionTable).where(eq(sessionTable.token,result.token)).limit(1); if (!session) throw new Error('no session'); return c.json({ token: session.token, expiresAt: session.expiresAt.toISOString() }); } catch { throw new ApiError({ error: 'unauthorized' }); } });
