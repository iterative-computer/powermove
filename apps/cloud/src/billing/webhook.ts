import type Stripe from 'stripe';
import { and, eq, inArray, lte, sql } from 'drizzle-orm';
import type { Data, Db } from '../db/client';
import { user } from '../db/auth-schema';
import { stripeCustomers, stripeEvents, subscriptions } from '../db/schema';
import { grantedFor, recordGrant } from './ledger';
import { type Plan, planForPrice } from './plans';
import { idOf, invoiceSubscriptionId, stripeClient, subscriptionPeriod } from './stripe';

export const HANDLED = new Set([
  'checkout.session.completed',
  'customer.subscription.created', 'customer.subscription.updated', 'customer.subscription.deleted',
  'invoice.paid', 'invoice.payment_failed',
]);

/** The DB changes one event makes, worked out before the transaction (Stripe reads happen here, never inside it). */
type Apply = (tx: Db) => Promise<void>;

/** Processes a verified event once. Returns false for a replay of an event already processed. */
export async function processEvent(data: Data, env: CloudflareBindings, event: Stripe.Event): Promise<boolean> {
  const apply = HANDLED.has(event.type) ? await prepare(env, data.db, event) : null;
  return data.tx(async (tx) => {
    const fresh = await tx.insert(stripeEvents).values({ id: event.id, type: event.type }).onConflictDoNothing().returning({ id: stripeEvents.id });
    if (fresh.length === 0) return false;
    if (apply) await apply(tx);
    return true;
  });
}

async function prepare(env: CloudflareBindings, db: Db, event: Stripe.Event): Promise<Apply | null> {
  const at = new Date(event.created * 1000);
  switch (event.type) {
    case 'checkout.session.completed': {
      const session = event.data.object;
      const customerId = idOf(session.customer), subscriptionId = idOf(session.subscription);
      if (session.mode !== 'subscription' || !customerId || !subscriptionId) return null;
      // Read the subscription now: Checkout's events can arrive in any order, and this one is enough to go active.
      const sub = await stripeClient(env).subscriptions.retrieve(subscriptionId);
      const userId = session.client_reference_id ?? session.metadata?.userId ?? null;
      return async (tx) => {
        if (userId && await userExists(tx, userId)) {
          await tx.insert(stripeCustomers).values({ userId, stripeCustomerId: customerId }).onConflictDoNothing();
        }
        await upsertSubscription(tx, env, sub, at);
      };
    }
    case 'customer.subscription.created':
    case 'customer.subscription.updated':
    case 'customer.subscription.deleted': {
      const sub = event.data.object;
      return async (tx) => { await upsertSubscription(tx, env, sub, at); };
    }
    case 'invoice.paid': {
      const invoice = event.data.object;
      const subscriptionId = invoiceSubscriptionId(invoice);
      const reason = invoice.billing_reason;
      if (!subscriptionId || !invoice.id || (reason !== 'subscription_create' && reason !== 'subscription_cycle' && reason !== 'subscription_update')) return null;
      const sub = await stripeClient(env).subscriptions.retrieve(subscriptionId);
      return async (tx) => {
        const row = await upsertSubscription(tx, env, sub, at);
        if (!row) return;
        const plan = planOf(env, sub);
        if (!plan) return;
        const period = invoicePeriod(invoice) ?? subscriptionPeriod(sub);
        // A new period's grant replaces the last one (nothing rolls over). A mid-period upgrade tops the period
        // up to the new plan's credits; a downgrade changes nothing until the next period.
        const already = await grantedFor(tx, row.userId, period.start);
        const credits = reason === 'subscription_update' ? Math.max(0, plan.credits - already) : (already === 0 ? plan.credits : 0);
        if (credits > 0) await recordGrant(tx, { userId: row.userId, key: `invoice:${invoice.id}`, credits, subscriptionId: sub.id, period });
      };
    }
    case 'invoice.payment_failed': {
      const subscriptionId = invoiceSubscriptionId(event.data.object);
      if (!subscriptionId) return null;
      return async (tx) => {
        await tx.update(subscriptions).set({ status: 'past_due', updatedAt: new Date() })
          .where(and(eq(subscriptions.id, subscriptionId), inArray(subscriptions.status, ['active', 'trialing'])));
      };
    }
    default:
      return null;
  }
}

/** The plan line's period: the subscription line that ends last (proration lines end with it too). */
function invoicePeriod(invoice: Stripe.Invoice): { start: Date; end: Date } | null {
  const lines = invoice.lines?.data ?? [];
  const best = lines.reduce<Stripe.InvoiceLineItem | null>((acc, line) => (!acc || line.period.end > acc.period.end ? line : acc), null);
  return best ? { start: new Date(best.period.start * 1000), end: new Date(best.period.end * 1000) } : null;
}

function planOf(env: CloudflareBindings, sub: Stripe.Subscription): Plan | null {
  for (const item of sub.items?.data ?? []) {
    const plan = planForPrice(env, item.price);
    if (plan) return plan;
  }
  return null;
}

async function userExists(tx: Db, userId: string): Promise<boolean> {
  const [row] = await tx.select({ id: user.id }).from(user).where(eq(user.id, userId)).limit(1);
  return !!row;
}

/** Writes Stripe's view of a subscription unless a newer one is already stored. Unknown customers and prices
    are skipped (they are not Powermove plans). */
async function upsertSubscription(tx: Db, env: CloudflareBindings, sub: Stripe.Subscription, at: Date) {
  const customerId = idOf(sub.customer);
  const plan = planOf(env, sub);
  if (!customerId || !plan) return null;
  const [customer] = await tx.select({ userId: stripeCustomers.userId }).from(stripeCustomers).where(eq(stripeCustomers.stripeCustomerId, customerId)).limit(1);
  let userId = customer?.userId ?? null;
  if (!userId && sub.metadata?.userId && await userExists(tx, sub.metadata.userId)) {
    userId = sub.metadata.userId;
    await tx.insert(stripeCustomers).values({ userId, stripeCustomerId: customerId }).onConflictDoNothing();
  }
  if (!userId) return null;
  const period = subscriptionPeriod(sub);
  const item = sub.items.data.find(entry => planForPrice(env, entry.price))!;
  const values = {
    id: sub.id, userId, stripeCustomerId: customerId, stripePriceId: item.price.id, plan: plan.id, status: sub.status,
    currentPeriodStart: period.start, currentPeriodEnd: period.end, cancelAtPeriodEnd: sub.cancel_at_period_end || sub.cancel_at != null,
    stripeUpdatedAt: at, updatedAt: new Date(),
  };
  await tx.insert(subscriptions).values(values).onConflictDoUpdate({
    target: subscriptions.id,
    set: { ...values, id: undefined, userId: undefined, createdAt: undefined },
    setWhere: lte(subscriptions.stripeUpdatedAt, sql`excluded.stripe_updated_at`),
  });
  return { userId };
}
