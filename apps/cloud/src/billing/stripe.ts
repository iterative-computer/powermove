/* A minimal Stripe client over fetch (no SDK: it runs on Workers and keeps the
   bundle small). Only test-mode keys are accepted until the team flips the
   guard in `stripeKey` deliberately. */
import { ApiError } from '@powermove/registry/wire';
import { isTier, type Tier } from './plans';
import { constantTimeEqual } from '../constant-time';

export function stripeKey(env: CloudflareBindings): string {
  const key = env.STRIPE_SECRET_KEY;
  if (!key) throw new ApiError({ error: 'internal', detail: 'billing is not configured' });
  // Aerolite MVP: refuse live keys outright so nothing can charge real money.
  if (!/^(sk|rk)_test_/.test(key)) throw new ApiError({ error: 'internal', detail: 'billing accepts Stripe test-mode keys only' });
  return key;
}

function form(params: Record<string, string | number | boolean | undefined>): string {
  const body = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) if (v !== undefined) body.append(k, String(v));
  return body.toString();
}

export async function stripe<T>(env: CloudflareBindings, method: 'GET' | 'POST', path: string, params: Record<string, string | number | boolean | undefined> = {}): Promise<T> {
  const fetcher = env.STRIPE_FETCH ?? fetch;
  const url = `https://api.stripe.com/v1${path}${method === 'GET' && Object.keys(params).length ? `?${form(params)}` : ''}`;
  const response = await fetcher(url, {
    method,
    headers: { Authorization: `Bearer ${stripeKey(env)}`, 'Content-Type': 'application/x-www-form-urlencoded', 'Stripe-Version': '2025-03-31.basil' },
    body: method === 'POST' ? form(params) : undefined,
  });
  const json = await response.json().catch(() => ({})) as T & { error?: { message?: string } };
  if (!response.ok) {
    console.error('stripe error', path, response.status, json.error?.message);
    throw new ApiError({ error: 'internal', detail: 'billing provider error' });
  }
  return json;
}

export function priceFor(env: CloudflareBindings, tier: Tier): string | undefined {
  return tier === 'aerolite' ? env.STRIPE_PRICE_AEROLITE : tier === 'aero' ? env.STRIPE_PRICE_AERO : undefined;
}
export function tierForPrice(env: CloudflareBindings, priceId: string | undefined): Tier | null {
  if (!priceId) return null;
  if (priceId === env.STRIPE_PRICE_AEROLITE) return 'aerolite';
  if (priceId === env.STRIPE_PRICE_AERO) return 'aero';
  return null;
}

export interface StripeSubscription {
  id: string; customer: string; status: string; cancel_at_period_end: boolean;
  current_period_start?: number; current_period_end?: number;
  metadata?: Record<string, string>;
  items?: { data: Array<{ price?: { id: string }; current_period_start?: number; current_period_end?: number }> };
}

/** The subscription fields we store. Newer Stripe API versions moved the period onto the item. */
export function subscriptionRow(env: CloudflareBindings, sub: StripeSubscription) {
  const item = sub.items?.data[0];
  const start = sub.current_period_start ?? item?.current_period_start;
  const end = sub.current_period_end ?? item?.current_period_end;
  const metaTier = sub.metadata?.tier;
  const tier = tierForPrice(env, item?.price?.id) ?? (isTier(metaTier) ? metaTier : 'free');
  return {
    tier, status: sub.status, stripeCustomerId: sub.customer, stripeSubscriptionId: sub.id,
    currentPeriodStart: start ? new Date(start * 1000) : null, currentPeriodEnd: end ? new Date(end * 1000) : null,
    cancelAtPeriodEnd: sub.cancel_at_period_end, updatedAt: new Date(),
  };
}

const hex = (bytes: ArrayBuffer) => Array.from(new Uint8Array(bytes), b => b.toString(16).padStart(2, '0')).join('');

/** Verifies a `Stripe-Signature` header (v1 scheme, HMAC-SHA256 over `${t}.${payload}`). */
export async function verifyStripeSignature(payload: string, header: string | undefined, secret: string, now = Date.now(), toleranceSeconds = 300): Promise<boolean> {
  if (!header) return false;
  const parts = header.split(',').map(p => p.split('=') as [string, string]);
  const t = Number(parts.find(([k]) => k === 't')?.[1]);
  const signatures = parts.filter(([k]) => k === 'v1').map(([, v]) => v);
  if (!Number.isFinite(t) || !signatures.length) return false;
  if (Math.abs(now / 1000 - t) > toleranceSeconds) return false;
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const expected = hex(await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(`${t}.${payload}`)));
  return signatures.some(sig => constantTimeEqual(sig, expected));
}

/** For tests and local webhook replay: a valid signature header. */
export async function signStripePayload(payload: string, secret: string, t = Math.floor(Date.now() / 1000)): Promise<string> {
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  return `t=${t},v1=${hex(await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(`${t}.${payload}`)))}`;
}
