import { Hono } from 'hono';
import { eq } from 'drizzle-orm';
import { ApiError } from '@powermove/registry/wire';
import type { Env } from '../env';
import { user } from '../db/auth-schema';
import { stripeEvents, subscriptions } from '../db/schema';
import { requireSession } from './session';
import { loadAccount, providerSpend, shareHistory, storedBytes } from '../billing/account';
import { isTier, MODEL_PROVIDERS, MODELS, PLANS, PURCHASABLE } from '../billing/plans';
import { checkShareQuota } from '../billing/quotas';
import { priceFor, stripe, subscriptionRow, verifyStripeSignature, type StripeSubscription } from '../billing/stripe';
import { icons, page } from '../page';

const planSummary = (tier: keyof typeof PLANS) => {
  const p = PLANS[tier];
  return { tier: p.tier, label: p.label, priceCents: p.priceCents, storageBytes: p.storageBytes, shares: p.shares, interactiveShares: p.interactiveShares,
    models: p.models ? MODEL_PROVIDERS.flatMap(provider => p.models![provider].models.map(id => ({ id, provider, label: MODELS[id]!.label }))) : [] };
};

export const billingRoutes = new Hono<Env>()
  /** Everything the Plan & Usage screen shows. Usage is reported as fractions of the allowance, never dollars. */
  .get('/plan', async (c) => {
    const s = requireSession(c);
    const db = c.var.data.db, now = new Date();
    const account = await loadAccount(db, s.userId, now);
    const [history, bytes] = await Promise.all([shareHistory(db, s.userId, now), storedBytes(db, s.userId)]);
    const shareDecision = checkShareQuota(account.ent.shares, history, now);
    const usage = account.ent.models ? Object.fromEntries(await Promise.all(MODEL_PROVIDERS.map(async provider => {
      const allowance = account.ent.models![provider];
      const spend = await providerSpend(db, s.userId, provider, account.windows);
      return [provider, {
        period: Math.min(1, spend.periodMicros / allowance.periodMicros), day: Math.min(1, spend.dayMicros / allowance.dayMicros),
        periodResetsAt: account.windows.period.end.toISOString(), dayResetsAt: account.windows.day.end.toISOString(),
      }];
    }))) : null;
    return c.json({
      plan: planSummary(account.tier),
      subscription: account.sub && account.tier !== 'free' ? {
        status: account.sub.status, renewsAt: account.sub.currentPeriodEnd?.toISOString() ?? null, cancelAtPeriodEnd: account.sub.cancelAtPeriodEnd,
      } : null,
      usage,
      shares: { used: history.total, usedThisWeek: history.inLastWeek, canCreate: shareDecision.ok,
        nextAvailableAt: shareDecision.ok ? null : shareDecision.nextAvailableAt?.toISOString() ?? null },
      storage: { usedBytes: bytes, limitBytes: account.ent.storageBytes },
      upgrades: PURCHASABLE.filter(t => PLANS[t].priceCents > account.ent.priceCents).map(planSummary),
      billingConfigured: Boolean(c.env.STRIPE_SECRET_KEY && c.env.STRIPE_PRICE_AEROLITE),
    });
  })
  /** Starts Stripe Checkout and returns its URL for the desktop to open in the browser. */
  .post('/checkout', async (c) => {
    const s = requireSession(c);
    const body = await c.req.json().catch(() => ({})) as { tier?: unknown };
    const tier = isTier(body.tier) ? body.tier : 'aerolite';
    if (!PURCHASABLE.includes(tier)) throw new ApiError({ error: 'bad_request', detail: `${PLANS[tier].label} is not available yet` });
    const price = priceFor(c.env, tier);
    if (!price) throw new ApiError({ error: 'internal', detail: 'billing is not configured' });
    const account = await loadAccount(c.var.data.db, s.userId);
    if (account.tier === tier) throw new ApiError({ error: 'bad_request', detail: `You're already on ${PLANS[tier].label}.` });
    const [u] = await c.var.data.db.select({ email: user.email }).from(user).where(eq(user.id, s.userId)).limit(1);
    const origin = c.env.APP_ORIGIN;
    const session = await stripe<{ url: string; id: string }>(c.env, 'POST', '/checkout/sessions', {
      mode: 'subscription',
      'line_items[0][price]': price, 'line_items[0][quantity]': 1,
      client_reference_id: s.userId,
      ...(account.sub?.stripeCustomerId ? { customer: account.sub.stripeCustomerId } : { customer_email: u?.email }),
      'subscription_data[metadata][user_id]': s.userId, 'subscription_data[metadata][tier]': tier,
      'metadata[user_id]': s.userId, 'metadata[tier]': tier,
      allow_promotion_codes: true,
      success_url: `${origin}/v1/billing/return?status=success`,
      cancel_url: `${origin}/v1/billing/return?status=cancelled`,
    });
    return c.json({ url: session.url });
  })
  /** Stripe's customer portal: change card, cancel, invoices. */
  .post('/portal', async (c) => {
    const s = requireSession(c);
    const account = await loadAccount(c.var.data.db, s.userId);
    if (!account.sub?.stripeCustomerId) throw new ApiError({ error: 'not_found', detail: 'no subscription' });
    const portal = await stripe<{ url: string }>(c.env, 'POST', '/billing_portal/sessions', {
      customer: account.sub.stripeCustomerId, return_url: `${c.env.APP_ORIGIN}/v1/billing/return?status=portal`,
    });
    return c.json({ url: portal.url });
  })
  /** Where the browser lands after Checkout or the portal. */
  .get('/return', (c) => {
    const status = c.req.query('status');
    const nonce = crypto.randomUUID();
    c.header('Content-Security-Policy', `default-src 'none'; style-src 'nonce-${nonce}'`);
    const ok = status === 'success';
    const title = ok ? 'Welcome to Aerolite' : status === 'portal' ? 'Billing updated' : 'Checkout cancelled';
    const copy = ok ? 'Your plan is active. Built-in Claude and OpenAI models are ready in Powermove.'
      : status === 'portal' ? 'Changes to your plan show up in Powermove within a few seconds.' : 'Nothing was charged. You can upgrade any time from Settings › Plan & Usage.';
    return c.html(page({ title, nonce, body: `<div class="badge" data-tone="${ok ? 'success' : 'accent'}">${ok ? icons.check : icons.alert}</div><h1>${title}</h1><p>${copy}</p><a class="button" href="powermove://plan">Return to Powermove</a>` }));
  })
  /** Stripe webhook. Verifies the signature, ignores replays, and upserts the subscription row. */
  .post('/webhook', async (c) => {
    const secret = c.env.STRIPE_WEBHOOK_SECRET;
    if (!secret) throw new ApiError({ error: 'internal', detail: 'billing is not configured' });
    const payload = await c.req.text();
    if (!(await verifyStripeSignature(payload, c.req.header('stripe-signature'), secret))) throw new ApiError({ error: 'unauthorized' });
    const event = JSON.parse(payload) as { id: string; type: string; livemode?: boolean; data: { object: Record<string, unknown> } };
    if (event.livemode) {
      console.error('live-mode Stripe event refused', event.type);
      return c.json({ received: true, ignored: 'livemode' });
    }
    const [seen] = await c.var.data.db.select({ id: stripeEvents.id }).from(stripeEvents).where(eq(stripeEvents.id, event.id)).limit(1);
    if (seen) return c.json({ received: true, duplicate: true });

    let subscriptionId: string | undefined;
    let userId: string | undefined;
    if (event.type === 'checkout.session.completed') {
      const session = event.data.object as { subscription?: string; client_reference_id?: string; metadata?: Record<string, string> };
      userId = session.client_reference_id ?? session.metadata?.user_id;
      subscriptionId = session.subscription;
    } else if (event.type.startsWith('customer.subscription.')) {
      const object = event.data.object as unknown as StripeSubscription;
      subscriptionId = object.id;
      userId = object.metadata?.user_id;
    }
    if (subscriptionId) {
      // Events can arrive out of order; always store Stripe's current state, not the event's snapshot.
      const sub = await stripe<StripeSubscription>(c.env, 'GET', `/subscriptions/${subscriptionId}`);
      const row = subscriptionRow(c.env, sub);
      userId ??= sub.metadata?.user_id;
      if (!userId) {
        const [existing] = await c.var.data.db.select({ userId: subscriptions.userId }).from(subscriptions).where(eq(subscriptions.stripeCustomerId, sub.customer)).limit(1);
        userId = existing?.userId;
      }
      const [exists] = userId ? await c.var.data.db.select({ id: user.id }).from(user).where(eq(user.id, userId)).limit(1) : [];
      if (exists) await c.var.data.db.insert(subscriptions).values({ userId: exists.id, ...row }).onConflictDoUpdate({ target: subscriptions.userId, set: row });
      else console.error('stripe subscription without a user', sub.id);
    }
    // Recorded only after success, so a failed attempt is retried by Stripe.
    await c.var.data.db.insert(stripeEvents).values({ id: event.id, type: event.type }).onConflictDoNothing();
    return c.json({ received: true });
  });
