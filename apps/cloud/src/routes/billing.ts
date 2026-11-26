import { Hono, type Context } from 'hono';
import { eq } from 'drizzle-orm';
import type Stripe from 'stripe';
import type { Env } from '../env';
import { user } from '../db/auth-schema';
import { stripeCustomers } from '../db/schema';
import { billingState, currentSubscription, publicStatus } from '../billing/ledger';
import { type Plan, isPlanId, planById } from '../billing/plans';
import { stripeClient } from '../billing/stripe';
import { consume } from '../abuse';
import { icons, page } from '../page';
import { V2Error } from '../v2-error';

function signedIn(c: Context<Env>): string {
  const session = c.var.session;
  if (!session) throw new V2Error(401, 'unauthorized');
  return session.userId;
}

async function limited(c: Context<Env>, userId: string) {
  const result = await consume(c.var.data.db, 'billing_user', userId);
  if (!result.ok) throw new V2Error(429, 'rate_limited', result.retryAfterSeconds);
}

/** The return page Checkout and the Portal send the browser back to, on the Worker's own origin (APP_ORIGIN:
    under wrangler dev the request's host is the production route, so it can't come from the request). */
const returnUrl = (c: Context<Env>, status: 'success' | 'cancel' | 'portal') => `${c.env.APP_ORIGIN ?? new URL(c.req.url).origin}/v2/billing/return?status=${status}`;

/** The account's Stripe customer, created once and keyed to the user. */
async function customerFor(c: Context<Env>, stripe: Stripe, userId: string): Promise<string> {
  const db = c.var.data.db;
  const [existing] = await db.select().from(stripeCustomers).where(eq(stripeCustomers.userId, userId)).limit(1);
  if (existing) return existing.stripeCustomerId;
  const [account] = await db.select({ email: user.email, name: user.name }).from(user).where(eq(user.id, userId)).limit(1);
  if (!account) throw new V2Error(401, 'unauthorized');
  const customer = await stripe.customers.create({ email: account.email, name: account.name, metadata: { userId } }, { idempotencyKey: `customer:${userId}` });
  await db.insert(stripeCustomers).values({ userId, stripeCustomerId: customer.id }).onConflictDoNothing();
  const [winner] = await db.select().from(stripeCustomers).where(eq(stripeCustomers.userId, userId)).limit(1);
  return winner!.stripeCustomerId;
}

async function priceFor(c: Context<Env>, stripe: Stripe, plan: Plan): Promise<string> {
  const configured = c.env[plan.envPrice];
  if (configured) return configured;
  const prices = await stripe.prices.list({ lookup_keys: [plan.lookupKey], active: true, limit: 1 });
  const price = prices.data[0];
  if (!price) throw new V2Error(503, 'billing_unavailable');
  return price.id;
}

export const billing = new Hono<Env>()
  .get('/', async (c) => {
    const userId = signedIn(c);
    c.header('Cache-Control', 'no-store');
    return c.json(await billingState(c.var.data.db, userId));
  })
  .post('/checkout', async (c) => {
    const userId = signedIn(c);
    const body = await c.req.json().catch(() => null) as { plan?: unknown } | null;
    if (!isPlanId(body?.plan)) throw new V2Error(400, 'invalid_plan');
    await limited(c, userId);
    const sub = await currentSubscription(c.var.data.db, userId);
    const status = sub ? publicStatus(sub.status) : null;
    if (status === 'active' || status === 'past_due') throw new V2Error(409, 'has_plan');
    const plan = planById(body.plan);
    const stripe = stripeClient(c.env);
    const customer = await customerFor(c, stripe, userId);
    const session = await stripe.checkout.sessions.create({
      mode: 'subscription',
      customer,
      client_reference_id: userId,
      line_items: [{ price: await priceFor(c, stripe, plan), quantity: 1 }],
      automatic_tax: { enabled: true },
      billing_address_collection: 'required',
      tax_id_collection: { enabled: true },
      customer_update: { address: 'auto', name: 'auto' },
      metadata: { userId, plan: plan.id },
      subscription_data: { metadata: { userId, plan: plan.id } },
      success_url: returnUrl(c, 'success'),
      cancel_url: returnUrl(c, 'cancel'),
    });
    if (!session.url) throw new V2Error(502, 'billing_unavailable');
    return c.json({ url: session.url });
  })
  .post('/portal', async (c) => {
    const userId = signedIn(c);
    await limited(c, userId);
    const [customer] = await c.var.data.db.select().from(stripeCustomers).where(eq(stripeCustomers.userId, userId)).limit(1);
    if (!customer) throw new V2Error(409, 'no_billing_account');
    const session = await stripeClient(c.env).billingPortal.sessions.create({ customer: customer.stripeCustomerId, return_url: returnUrl(c, 'portal') });
    return c.json({ url: session.url });
  })
  .get('/return', (c) => {
    const raw = c.req.query('status');
    const status = raw === 'success' || raw === 'cancel' ? raw : 'portal';
    const link = `powermove://billing/return?status=${status}`;
    const nonce = crypto.randomUUID();
    c.header('Content-Security-Policy', `default-src 'none'; style-src 'nonce-${nonce}'`);
    c.header('Cache-Control', 'no-store');
    c.header('Referrer-Policy', 'no-referrer');
    const [badge, title, text] = status === 'success'
      ? [`<div class="badge" data-tone="success">${icons.check}</div>`, 'You’re subscribed', 'Your plan is ready. You can return to Powermove.']
      : status === 'cancel'
        ? [`<div class="badge">${icons.alert}</div>`, 'Checkout canceled', 'Nothing was charged. You can return to Powermove.']
        : [`<div class="badge" data-tone="success">${icons.check}</div>`, 'Billing updated', 'You can return to Powermove.'];
    return c.html(page({
      title,
      nonce,
      head: `<meta http-equiv="refresh" content="0;url=${link}">`,
      body: `${badge}<h1>${title}</h1><p>${text}</p><a class="button" href="${link}">Open Powermove</a>`,
      foot: 'You can close this tab once Powermove opens.',
    }));
  });
