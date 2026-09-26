import { Hono } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { ApiError, Auth } from '@powermove/registry/wire';
import { and, eq, isNull } from 'drizzle-orm';
import { desktopAuth } from '../db/schema';
import type { Env } from '../env';
import { createAuth } from '../auth';
import { rateAuth } from './session';
import { constantTimeEqual } from '../constant-time';
import { clientIp, enforce } from '../abuse';
function decode(value: string): Uint8Array | null {
  try {
    if (!/^[A-Za-z0-9_-]+$/.test(value)) {
      return null;
    }
    return Uint8Array.from(atob(value.replace(/-/g, '+').replace(/_/g, '/')), (c) => c.charCodeAt(0));
  } catch {
    return null;
  }
}
function b64url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes)).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}
async function sha(value: string): Promise<string> {
  return Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(value)))).map((v) =>
    v.toString(16).padStart(2, '0')
  ).join('');
}
async function challengeOf(verifier: string): Promise<string | null> {
  const bytes = decode(verifier);
  if (!bytes || bytes.length !== 32) {
    return null;
  }
  return b64url(new Uint8Array(await crypto.subtle.digest('SHA-256', new Uint8Array(bytes))));
}
function escapeHtml(value: string): string {
  return value.replace(
    /[&<>"']/g,
    (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!,
  );
}
export const desktop = new Hono<Env>()
  .get(
    '/',
    zValidator('query', Auth.DesktopStart.Req.shape.query, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    async (c) => {
      await rateAuth(c);
      await enforce(c, 'auth_start_ip', clientIp(c));
      const { state, challenge } = c.req.valid('query');
      if (decode(challenge)?.length !== 32) {
        throw new ApiError({ error: 'bad_request' });
      }
      try {
        await c.var.data.db.insert(desktopAuth).values({ state, challenge, expiresAt: new Date(Date.now() + 600000) });
      } catch (error) {
        if (
          (error as {
              code?: string;
              cause?: {
                code?: string;
              };
            }).code === '23505' || (error as {
                cause?: {
                  code?: string;
                };
              }).cause?.code === '23505'
        ) {
          throw new ApiError({ error: 'bad_request' });
        }
        throw error;
      }
      try {
        const result = await createAuth(c.var.data, c.env).api.signInSocial({
          body: {
            provider: 'google',
            callbackURL: `${c.env.APP_ORIGIN}/v1/auth/desktop/done?state=${encodeURIComponent(state)}`,
          },
          headers: c.req.raw.headers,
          returnHeaders: true,
        });
        if (!result.response.url) {
          throw new Error('no redirect');
        }
        const redirect = c.redirect(result.response.url);
        for (const cookie of result.headers?.getSetCookie() ?? []) {
          redirect.headers.append('set-cookie', cookie);
        }
        return redirect;
      } catch {
        await c.var.data.db.update(desktopAuth).set({ consumedAt: new Date() }).where(eq(desktopAuth.state, state));
        throw new ApiError({ error: 'bad_request' });
      }
    },
  )
  .get(
    '/done',
    zValidator('query', Auth.DesktopDone.Req.shape.query, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    async (c) => {
      const { state } = c.req.valid('query');
      const auth = createAuth(c.var.data, c.env);
      const browser = await auth.api.getSession({ headers: c.req.raw.headers }).catch(() => null);
      if (!browser) {
        throw new ApiError({ error: 'unauthorized' });
      }
      const [row] = await c.var.data.db.select().from(desktopAuth).where(eq(desktopAuth.state, state)).limit(1);
      if (!row || row.consumedAt || row.expiresAt <= new Date()) {
        throw new ApiError({ error: 'unauthorized' });
      }
      let token: string;
      try {
        token = (await auth.api.generateOneTimeToken({ headers: c.req.raw.headers })).token;
      } catch {
        throw new ApiError({ error: 'unauthorized' });
      }
      const [updated] = await c.var.data.db.update(desktopAuth).set({ tokenHash: await sha(token) }).where(
        and(eq(desktopAuth.state, state), isNull(desktopAuth.tokenHash), isNull(desktopAuth.consumedAt)),
      ).returning();
      if (!updated) {
        throw new ApiError({ error: 'unauthorized' });
      }
      const link = `powermove://auth?state=${encodeURIComponent(state)}&token=${encodeURIComponent(token)}`;
      c.header('Content-Security-Policy', "default-src 'none'; style-src 'unsafe-inline'");
      c.header('Content-Type', 'text/html; charset=utf-8');
      c.header('Cache-Control', 'no-store');
      c.header('Referrer-Policy', 'no-referrer');
      return c.html(
        `<!doctype html><html><head><meta charset="utf-8"><title>Return to Powermove</title><meta http-equiv="refresh" content="0;url=${
          escapeHtml(link)
        }"><style>body{font:16px system-ui;padding:3rem}</style></head><body><a href="${
          escapeHtml(link)
        }">Return to Powermove</a></body></html>`,
      );
    },
  )
  .post(
    '/exchange',
    zValidator('json', Auth.DesktopExchange.Req.shape.body, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    async (c) => {
      const { state, token, verifier } = c.req.valid('json');
      await rateAuth(c);
      await enforce(c, 'exchange_ip', clientIp(c));
      try {
        const session = await c.var.data.tx(async (tx) => {
          const [row] = await tx.select().from(desktopAuth).where(eq(desktopAuth.state, state)).for('update').limit(1);
          const challenge = await challengeOf(verifier);
          if (
            !row || row.consumedAt || row.expiresAt <= new Date() || !row.tokenHash || !challenge ||
            !constantTimeEqual(challenge, row.challenge) || !constantTimeEqual(await sha(token), row.tokenHash)
          ) {
            throw new Error('invalid desktop exchange');
          }
          const auth = createAuth({ ...c.var.data, db: tx, authDb: () => tx }, c.env);
          const verified = await auth.api.verifyOneTimeToken({ body: { token }, headers: c.req.raw.headers });
          await tx.update(desktopAuth).set({ consumedAt: new Date() }).where(eq(desktopAuth.state, state));
          const created = await (await auth.$context).internalAdapter.createSession(verified.user.id);
          if (!created) {
            throw new Error('session creation failed');
          }
          return created;
        });
        return c.json({ token: session.token, expiresAt: session.expiresAt.toISOString() });
      } catch {
        await c.var.data.db.update(desktopAuth).set({ consumedAt: new Date() }).where(eq(desktopAuth.state, state));
        throw new ApiError({ error: 'unauthorized' });
      }
    },
  );
