import { Hono, type Context } from 'hono';
import { ApiError } from '@powermove/registry/wire';
import type { Env } from './env';
import { eq, and, gt } from 'drizzle-orm';
import { verification } from './db/auth-schema';
import { clientIp, enforce } from './abuse';

type Action = 'email_send' | 'first_publish';
type Proof = { kind: 'challenge' | 'clearance'; action: Action; subject: string; expires: number; nonce: string };
const encoder = new TextEncoder();
const encode = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '');
const decode = (text: string) => Uint8Array.from(atob(text.replaceAll('-', '+').replaceAll('_', '/')), c => c.charCodeAt(0));
async function key(env: CloudflareBindings) {
  return crypto.subtle.importKey('raw', encoder.encode(`turnstile:${env.BETTER_AUTH_SECRET}`), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign', 'verify']);
}
async function sign(env: CloudflareBindings, proof: Proof): Promise<string> {
  const body = encode(encoder.encode(JSON.stringify(proof)));
  return `${body}.${encode(new Uint8Array(await crypto.subtle.sign('HMAC', await key(env), encoder.encode(body))))}`;
}
async function read(env: CloudflareBindings, token: string | undefined, kind: Proof['kind']): Promise<Proof | null> {
  if (!token || token.length > 2048) return null;
  try {
    const [body, signature, extra] = token.split('.');
    if (!body || !signature || extra || !await crypto.subtle.verify('HMAC', await key(env), decode(signature), encoder.encode(body))) return null;
    const proof = JSON.parse(new TextDecoder().decode(decode(body))) as Proof;
    return proof.kind === kind && ['email_send', 'first_publish'].includes(proof.action) && typeof proof.subject === 'string'
      && typeof proof.expires === 'number' && proof.expires > Date.now() && typeof proof.nonce === 'string' ? proof : null;
  } catch { return null; }
}
async function hash(value: string): Promise<string> {
  return encode(new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(value))));
}
function localTestKeys(env: CloudflareBindings): boolean {
  return ['localhost', '127.0.0.1'].includes(new URL(env.APP_ORIGIN).hostname)
    && env.TURNSTILE_SITE_KEY === '1x00000000000000000000AA'
    && env.TURNSTILE_SECRET_KEY === '1x0000000000000000000000000000000AA';
}
function enabled(env: CloudflareBindings): boolean {
  if (env.TURNSTILE_ENABLED !== '1' && !env.TURNSTILE_SITE_KEY && !env.TURNSTILE_SECRET_KEY) return false;
  if (!env.TURNSTILE_SITE_KEY || !env.TURNSTILE_SECRET_KEY) throw new ApiError({ error: 'internal', detail: 'Verification is temporarily unavailable. Please try again later.' });
  if ((env.TURNSTILE_SITE_KEY.startsWith('1x000000') || env.TURNSTILE_SECRET_KEY.startsWith('1x000000')) && !localTestKeys(env)) throw new ApiError({ error: 'internal', detail: 'Verification is temporarily unavailable. Please try again later.' });
  return true;
}

/** Remembered verification is bound to the action and email/user, not a global bypass. */
export async function requireHuman(c: Context<Env>, action: Action, identity: string): Promise<void> {
  if (!enabled(c.env)) return;
  const subject = await hash(identity);
  const proof = await read(c.env, c.req.header('X-Powermove-Human'), 'clearance');
  if (proof?.action === action && proof.subject === subject) return;
  const ticket = await sign(c.env, { kind: 'challenge', action, subject, expires: Date.now() + 5 * 60_000, nonce: crypto.randomUUID() });
  throw new ApiError({ error: 'human_verification_required', ticket, scope: `${action}:${subject}`, detail: 'Please complete the quick verification to continue.' });
}

function clearanceDays(env: CloudflareBindings): number {
  const days = Number(env.TURNSTILE_CLEARANCE_DAYS ?? '30');
  return Number.isFinite(days) ? Math.max(1, Math.min(90, days)) : 30;
}
const unavailable = () => new ApiError({ error: 'bad_request', detail: 'Verification expired or failed. Please try again.' });
export const human = new Hono<Env>()
  .get('/', async c => {
    if (!enabled(c.env)) throw unavailable();
    const ticket = c.req.query('ticket');
    const proof = await read(c.env, ticket, 'challenge');
    if (!proof || !ticket) throw unavailable();
    const nonce = crypto.randomUUID();
    const config = JSON.stringify({ ticket, sitekey: c.env.TURNSTILE_SITE_KEY, action: proof.action, cdata: await hash(ticket) }).replaceAll('<', '\\u003c');
    c.header('Cache-Control', 'no-store');
    c.header('Referrer-Policy', 'no-referrer');
    c.header('Content-Security-Policy', `default-src 'none'; script-src 'nonce-${nonce}' https://challenges.cloudflare.com; frame-src https://challenges.cloudflare.com; connect-src 'self' https://challenges.cloudflare.com; style-src 'nonce-${nonce}'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`);
    return c.html(`<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Verify · Powermove</title><style nonce="${nonce}">body{font:15px system-ui;margin:32px;color:#222;background:#fff}h1{font-size:20px}p{line-height:1.5;color:#555}</style></head><body><h1>A quick check</h1><p id="status">Verifying your request…</p><div id="widget"></div><script nonce="${nonce}">
const config=${config};
function failed(){document.getElementById('status').textContent='Verification could not finish. Close this window and try again.';location.hash='error';}
window.startVerification=()=>turnstile.render('#widget',{sitekey:config.sitekey,action:config.action,cData:config.cdata,appearance:'interaction-only',
'before-interactive-callback':()=>{location.hash='interaction';},'error-callback':()=>{failed();return true;},'expired-callback':failed,'timeout-callback':failed,
callback:async token=>{try{const response=await fetch('/v1/human/verify',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({ticket:config.ticket,token})});if(!response.ok)throw Error();document.getElementById('status').textContent='You’re verified. Return to Powermove to continue.';document.getElementById('widget').hidden=true;}catch{failed();}}});
</script><script nonce="${nonce}" src="https://challenges.cloudflare.com/turnstile/v0/api.js?onload=startVerification&amp;render=explicit" async defer></script></body></html>`);
  })
  .post('/verify', async c => {
    if (!enabled(c.env)) throw unavailable();
    await enforce(c, 'human_verify_ip', clientIp(c));
    const body = await c.req.json().catch(() => null) as { ticket?: unknown; token?: unknown } | null;
    if (typeof body?.ticket !== 'string' || typeof body.token !== 'string' || body.token.length > 2048) throw unavailable();
    const proof = await read(c.env, body.ticket, 'challenge');
    if (!proof) throw unavailable();
    let result: { success?: boolean; hostname?: string; action?: string; cdata?: string };
    try {
      const response = await fetch('https://challenges.cloudflare.com/turnstile/v0/siteverify', {
        method: 'POST', headers: { 'Content-Type': 'application/json' }, signal: AbortSignal.timeout(10_000),
        body: JSON.stringify({ secret: c.env.TURNSTILE_SECRET_KEY, response: body.token, ...(clientIp(c) !== 'unknown' ? { remoteip: clientIp(c) } : {}) })
      });
      if (!response.ok) throw unavailable();
      result = await response.json();
    } catch { throw unavailable(); }
    if (!result.success || (!localTestKeys(c.env) && (result.hostname !== new URL(c.env.APP_ORIGIN).hostname || result.action !== proof.action || result.cdata !== await hash(body.ticket)))) throw unavailable();
    c.header('Cache-Control', 'no-store');
    const clearance = await sign(c.env, { ...proof, kind: 'clearance', expires: Date.now() + clearanceDays(c.env) * 86_400_000 });
    const id = `human:${await hash(body.ticket)}`;
    const expiresAt = new Date(Date.now() + 5 * 60_000);
    await c.var.data.db.insert(verification).values({ id, identifier: id, value: clearance, expiresAt })
      .onConflictDoUpdate({ target: verification.id, set: { value: clearance, expiresAt } });
    return c.json({ ok: true });
  })
  .post('/result', async c => {
    if (!enabled(c.env)) throw unavailable();
    const body = await c.req.json().catch(() => null) as { ticket?: unknown } | null;
    if (typeof body?.ticket !== 'string' || !await read(c.env, body.ticket, 'challenge')) throw unavailable();
    const [receipt] = await c.var.data.db.delete(verification).where(and(
      eq(verification.id, `human:${await hash(body.ticket)}`), gt(verification.expiresAt, new Date())
    )).returning({ clearance: verification.value });
    c.header('Cache-Control', 'no-store');
    return receipt ? c.json(receipt) : c.body(null, 204);
  });
