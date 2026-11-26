import Stripe from 'stripe';
import { V2Error } from '../v2-error';

const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '[::1]']);

/** The Stripe client. `STRIPE_API_BASE` points it at the local fake and is honoured only on a loopback host,
    so a misconfigured deployment can't send the secret key anywhere but Stripe. */
export function stripeClient(env: CloudflareBindings): Stripe {
  if (!env.STRIPE_SECRET_KEY) throw new V2Error(503, 'billing_unavailable');
  const base = env.STRIPE_API_BASE ? new URL(env.STRIPE_API_BASE) : null;
  if (base && !LOCAL_HOSTS.has(base.hostname)) throw new V2Error(503, 'billing_unavailable');
  return new Stripe(env.STRIPE_SECRET_KEY, {
    httpClient: Stripe.createFetchHttpClient(),
    maxNetworkRetries: 2,
    ...(base && { host: base.hostname, port: Number(base.port || 80), protocol: base.protocol === 'https:' ? 'https' : 'http' }),
  });
}

export async function verifiedEvent(env: CloudflareBindings, payload: string, signature: string | undefined): Promise<Stripe.Event> {
  if (!env.STRIPE_WEBHOOK_SECRET || !env.STRIPE_SECRET_KEY) throw new V2Error(503, 'billing_unavailable');
  if (!signature) throw new V2Error(400, 'invalid_signature');
  try {
    return await stripeClient(env).webhooks.constructEventAsync(payload, signature, env.STRIPE_WEBHOOK_SECRET, 300, Stripe.createSubtleCryptoProvider());
  } catch {
    throw new V2Error(400, 'invalid_signature');
  }
}

/** A subscription's period: its items carry it in current API versions, the subscription itself in older ones. */
export function subscriptionPeriod(sub: Stripe.Subscription): { start: Date; end: Date } {
  const legacy = sub as unknown as { current_period_start?: number; current_period_end?: number };
  const items = sub.items?.data ?? [];
  const start = Math.max(...items.map(item => item.current_period_start ?? 0), legacy.current_period_start ?? 0);
  const end = Math.max(...items.map(item => item.current_period_end ?? 0), legacy.current_period_end ?? 0);
  return { start: new Date(start * 1000), end: new Date(end * 1000) };
}

export function invoiceSubscriptionId(invoice: Stripe.Invoice): string | null {
  const fromParent = invoice.parent?.subscription_details?.subscription;
  const legacy = (invoice as unknown as { subscription?: string | { id: string } | null }).subscription;
  const value = fromParent ?? legacy ?? null;
  return typeof value === 'string' ? value : value?.id ?? null;
}

export const idOf = (value: string | { id: string } | null | undefined): string | null => typeof value === 'string' ? value : value?.id ?? null;
