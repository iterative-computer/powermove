import { Hono, type Context } from 'hono';
import { zValidator } from '@hono/zod-validator';
import { ApiError, Auth } from '@powermove/registry/wire';
import { and, eq, isNull } from 'drizzle-orm';
import { desktopAuth } from '../db/schema';
import type { Env } from '../env';
import { createAuth } from '../auth';
import { rateAuth } from './session';
import { constantTimeEqual } from '../constant-time';
import { clientIp, enforce } from '../abuse';
import { icons, page } from '../page';
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
async function start(c: Context<Env>, done: string, state: string, challenge: string): Promise<Response> {
  await rateAuth(c);
  await enforce(c, 'auth_start_ip', clientIp(c));
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
      // The same link loaded again (a reload, or a browser that opens it
      // twice) carries on while nothing has used it; any other reuse of a
      // state is refused.
      const [row] = await c.var.data.db.select().from(desktopAuth).where(eq(desktopAuth.state, state)).limit(1);
      if (!row || row.challenge !== challenge || row.consumedAt || row.tokenHash || row.expiresAt <= new Date()) {
        throw new ApiError({ error: 'bad_request' });
      }
    } else {
      throw error;
    }
  }
  try {
    const result = await createAuth(c.var.data, c.env).api.signInSocial({
      body: {
        provider: 'google',
        callbackURL: `${c.env.APP_ORIGIN}${done}?state=${encodeURIComponent(state)}`,
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
  } catch (error) {
    console.error('google sign-in start failed', (error as Error)?.name, (error as Error)?.message);
    await c.var.data.db.update(desktopAuth).set({ consumedAt: new Date() }).where(eq(desktopAuth.state, state));
    throw new ApiError({ error: 'bad_request' });
  }
}

/* The browser routes answer a person, not a program: a failure is a page
   that says what to do, never raw JSON. */
async function inBrowser(c: Context<Env>, run: () => Promise<Response>): Promise<Response> {
  try {
    return await run();
  } catch (error) {
    if (!(error instanceof ApiError)) throw error;
    const limited = error.status === 429;
    const nonce = crypto.randomUUID();
    c.header('Content-Security-Policy', `default-src 'none'; style-src 'nonce-${nonce}'`);
    c.header('Cache-Control', 'no-store');
    c.header('Referrer-Policy', 'no-referrer');
    return c.html(page({
      title: 'Sign-in didn’t finish',
      nonce,
      body: `<div class="badge" data-tone="danger">${icons.alert}</div><h1>Sign-in didn’t finish</h1><p>${
        limited
          ? 'Too many sign-in attempts. Wait a minute, then try again in Powermove.'
          : 'This sign-in link has expired or was already used. Go back to Powermove and choose Continue with Google again.'
      }</p>`,
      foot: 'You can close this tab.',
    }), error.status as 400);
  }
}

/* The browser that just signed in gets a one-time token for the flow's state; the PKCE exchange turns it into a session. */
async function mint(c: Context<Env>, state: string): Promise<string> {
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
  return token;
}

async function exchange(c: Context<Env>, state: string, token: string, verifier: string): Promise<Response> {
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
}

export const desktop = new Hono<Env>()
  .get(
    '/',
    zValidator('query', Auth.DesktopStart.Req.shape.query, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    (c) => {
      const { state, challenge } = c.req.valid('query');
      return inBrowser(c, () => start(c, '/v1/auth/desktop/done', state, challenge));
    },
  )
  .get(
    '/done',
    zValidator('query', Auth.DesktopDone.Req.shape.query, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    (c) => inBrowser(c, async () => {
      const { state } = c.req.valid('query');
      const token = await mint(c, state);
      const link = `powermove://auth?state=${encodeURIComponent(state)}&token=${encodeURIComponent(token)}`;
      const nonce = crypto.randomUUID();
      c.header('Content-Security-Policy', `default-src 'none'; style-src 'nonce-${nonce}'`);
      c.header('Content-Type', 'text/html; charset=utf-8');
      c.header('Cache-Control', 'no-store');
      c.header('Referrer-Policy', 'no-referrer');
      return c.html(page({
        title: 'Signed in',
        nonce,
        head: `<meta http-equiv="refresh" content="0;url=${escapeHtml(link)}">`,
        body: `<div class="badge" data-tone="success">${icons.check}</div><h1>You’re signed in</h1><p>Powermove should open on its own. If it doesn’t, open it below.</p><a class="button" href="${escapeHtml(link)}">Open Powermove</a>`,
        foot: 'You can close this tab once Powermove opens.',
      }));
    }),
  )
  .post(
    '/exchange',
    zValidator('json', Auth.DesktopExchange.Req.shape.body, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    (c) => {
      const { state, token, verifier } = c.req.valid('json');
      return exchange(c, state, token, verifier);
    },
  );

/* The same PKCE hand-off for the admin panel (apps/admin): the one-time token
   goes back to ADMIN_ORIGIN, whose server holds the verifier and exchanges it.
   No other origin can receive a token; without ADMIN_ORIGIN the routes 404. */
function adminOrigin(c: Context<Env>): string {
  const origin = c.env.ADMIN_ORIGIN;
  if (!origin) throw new ApiError({ error: 'not_found' });
  return new URL(origin).origin;
}
export const web = new Hono<Env>()
  .get(
    '/',
    zValidator('query', Auth.WebStart.Req.shape.query, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    (c) => {
      adminOrigin(c);
      const { state, challenge } = c.req.valid('query');
      return start(c, '/v1/auth/web/done', state, challenge);
    },
  )
  .get(
    '/done',
    zValidator('query', Auth.WebDone.Req.shape.query, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    async (c) => {
      const origin = adminOrigin(c);
      const { state } = c.req.valid('query');
      const token = await mint(c, state);
      const target = new URL('/auth/callback', origin);
      target.searchParams.set('state', state);
      target.searchParams.set('token', token);
      c.header('Cache-Control', 'no-store');
      c.header('Referrer-Policy', 'no-referrer');
      return c.redirect(target.toString(), 302);
    },
  )
  .post(
    '/exchange',
    zValidator('json', Auth.WebExchange.Req.shape.body, (result) => {
      if (!result.success) {
        throw result.error;
      }
    }),
    (c) => {
      adminOrigin(c);
      const { state, token, verifier } = c.req.valid('json');
      return exchange(c, state, token, verifier);
    },
  );
