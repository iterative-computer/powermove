import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { ApiError, Auth } from '@powermove/registry/wire';
import type { Env } from '../env';
import { createAuth } from '../auth';
import { rateAuth } from './session';
import { session as sessionTable } from '../db/auth-schema';
import { eq } from 'drizzle-orm';
import { clientIp, enforce } from '../abuse';
const verifyDetail = "Couldn't verify the code. Try again.";
export const email = new Hono<Env>()
  .post(
    '/send',
    zValidator('json', Auth.EmailSend.Req.shape.body, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    async (c) => {
      await rateAuth(c);
      const { email } = c.req.valid('json');
      await enforce(c, 'email_send_addr', email.trim().toLowerCase());
      await enforce(c, 'email_send_ip', clientIp(c));
      await enforce(c, 'email_send_global', '*');
      await new Promise(resolve => setTimeout(resolve, 30));
      if (!c.env.OTP_SENDER && !c.env.EMAIL && c.env.DEV_LOG_OTP !== '1') {
        throw new ApiError({ error: 'bad_request', detail: "Couldn't send the code. Try again." });
      }
      let delivery: Promise<boolean> | undefined;
      try {
        await createAuth(c.var.data, c.env, (pending) => {
          delivery = pending;
        }).api.sendVerificationOTP({
          body: { email, type: 'sign-in' },
          headers: c.req.raw.headers,
        });
        if (!delivery || !await delivery) throw new Error('delivery failed');
      } catch {
        throw new ApiError({ error: 'bad_request', detail: "Couldn't send the code. Try again." });
      }
      return c.json({ ok: true as const });
    },
  )
  .post(
    '/verify',
    zValidator('json', Auth.EmailVerify.Req.shape.body, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    async (c) => {
      const { email, otp } = c.req.valid('json');
      await rateAuth(c);
      await enforce(c, 'email_verify_addr', email.trim().toLowerCase());
      await enforce(c, 'email_verify_ip', clientIp(c));
      try {
        const result = await createAuth(c.var.data, c.env).api.signInEmailOTP({
          body: { email, otp },
          headers: c.req.raw.headers,
        });
        const [session] = await c.var.data.db.select().from(sessionTable).where(eq(sessionTable.token, result.token))
          .limit(1);
        if (!session) {
          throw new Error('no session');
        }
        return c.json({ token: session.token, expiresAt: session.expiresAt.toISOString() });
      } catch {
        throw new ApiError({ error: 'bad_request', detail: verifyDetail });
      }
    },
  );
