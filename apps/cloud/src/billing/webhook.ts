import type Stripe from 'stripe';
import { and, asc, eq, inArray, lte, sql } from 'drizzle-orm';
import type { Data, Db } from '../db/client';
import { checkoutSessions, stripeCustomers, stripeEvents, subscriptions } from '../db/schema';
import { grantedFor, lockBalance, recordGrant, reverseInvoice } from './ledger';
import { planById, planForPrice } from './plans';
import { idOf, invoiceSubscriptionId, stripeClient, subscriptionPeriod } from './stripe';

/* Trust: an account owns a Stripe customer only through the `stripe_customers` row the Worker wrote when it created
   that customer. client_reference_id, metadata and lookup keys are never trusted for ownership or plans. A plan is
   only ever the exact configured price id. Records that disagree (a subscription of another customer, an invoice
   of another subscription) are ignored. */
export const HANDLED = new Set([
  'checkout.session.completed', 'checkout.session.expired',
  'customer.subscription.created', 'customer.subscription.updated', 'customer.subscription.deleted',
  'invoice.paid', 'invoice.payment_failed', 'invoice.voided', 'invoice.marked_uncollectible',
  'charge.refunded', 'charge.dispute.created',
]);
const LIVE = ['active', 'trialing', 'past_due', 'unpaid', 'paused'];

/** The DB changes one event makes, worked out before the transaction (Stripe reads happen here, never inside it).
    It returns the account it touched, for the reconciliation that runs after. */
type Apply = (tx: Db) => Promise<string | null | void>;

/** Processes a verified event once. Returns false for a replay of an event already processed. */
export async function processEvent(data: Data, env: CloudflareBindings, event: Stripe.Event): Promise<boolean> {
  const apply = HANDLED.has(event.type) ? await prepare(env, data.db, event) : null;
  const result = await data.tx(async (tx) => {
    const fresh = await tx.insert(stripeEvents).values({ id: event.id, type: event.type }).onConflictDoNothing().returning({ id: stripeEvents.id });
    if (fresh.length === 0) return { processed: false, userId: null };
    const userId = apply ? await apply(tx) : null;
    return { processed: true, userId: userId ?? null };
  });
  if (result.userId) await reconcile(data, env, result.userId);
  return result.processed;
}

/** The account that owns a Stripe customer, by the Worker's own mapping only. */
async function ownerOf(db: Db, customerId: string | null): Promise<string | null> {
  if (!customerId) return null;
  const [row] = await db.select({ userId: stripeCustomers.userId }).from(stripeCustomers).where(eq(stripeCustomers.stripeCustomerId, customerId)).limit(1);
  return row?.userId ?? null;
}

/** The configured Aerolite line of an invoice for this subscription: its exact price id, nothing else. */
function aeroliteLine(env: CloudflareBindings, invoice: Stripe.Invoice, subscriptionId: string): Stripe.InvoiceLineItem | null {
  const price = env.STRIPE_PRICE_AEROLITE;
  if (!price) return null;
  return (invoice.lines?.data ?? []).find((line) => {
    const linePrice = (line as any).pricing?.price_details?.price ?? (line as any).price?.id;
    const lineSub = (line as any).parent?.subscription_item_details?.subscription ?? (line as any).subscription;
    return typeof linePrice === 'string' && linePrice === price && lineSub === subscriptionId;
  }) ?? null;
}

/** The invoice a charge paid, through Stripe's invoice payments. */
async function invoiceOfCharge(env: CloudflareBindings, charge: Stripe.Charge): Promise<{ invoiceId: string; customerId: string | null } | null> {
  const intent = idOf(charge.payment_intent as string | { id: string } | null);
  if (!intent) return null;
  const payments = await stripeClient(env).invoicePayments.list({ payment: { type: 'payment_intent', payment_intent: intent }, limit: 1 });
  const invoiceId = idOf(payments.data[0]?.invoice as string | { id: string } | undefined);
  return invoiceId ? { invoiceId, customerId: idOf(charge.customer as string | { id: string } | null) } : null;
}

async function prepare(env: CloudflareBindings, db: Db, event: Stripe.Event): Promise<Apply | null> {
  const at = new Date(event.created * 1000);
  switch (event.type) {
    case 'checkout.session.completed':
    case 'checkout.session.expired': {
      const session = event.data.object;
      const customerId = idOf(session.customer), subscriptionId = idOf(session.subscription);
      const clear = async (tx: Db) => { await tx.delete(checkoutSessions).where(eq(checkoutSessions.sessionId, session.id)); };
      if (event.type === 'checkout.session.expired' || session.mode !== 'subscription' || !customerId || !subscriptionId) return clear;
      // Read the subscription now: Checkout's events can arrive in any order, and this one is enough to go active.
      const sub = await stripeClient(env).subscriptions.retrieve(subscriptionId);
      if (idOf(sub.customer) !== customerId) return clear;
      return async (tx) => { await clear(tx); return upsertSubscription(tx, env, sub, at); };
    }
    case 'customer.subscription.created':
    case 'customer.subscription.updated':
    case 'customer.subscription.deleted': {
      const sub = event.data.object;
      return async (tx) => upsertSubscription(tx, env, sub, at);
    }
    case 'invoice.paid': {
      const invoice = event.data.object;
      const subscriptionId = invoiceSubscriptionId(invoice);
      const reason = invoice.billing_reason;
      if (!subscriptionId || !invoice.id || (reason !== 'subscription_create' && reason !== 'subscription_cycle' && reason !== 'subscription_update')) return null;
      const sub = await stripeClient(env).subscriptions.retrieve(subscriptionId);
      if (idOf(sub.customer) !== idOf(invoice.customer)) return null;
      return async (tx) => {
        const userId = await upsertSubscription(tx, env, sub, at);
        if (!userId) return null;
        // A grant only for a paid invoice that took money, from its line for the exact Aerolite price on this
        // subscription; the credits and the period come from that line, never from the subscription's plan now.
        const line = aeroliteLine(env, invoice, sub.id);
        if (invoice.status !== 'paid' || !(invoice.amount_paid > 0) || !line || (sub.status !== 'active' && sub.status !== 'trialing')) return userId;
        if (!await isSurviving(tx, userId, sub.id)) return userId;
        const plan = planById('aerolite');
        const period = { start: new Date(line.period.start * 1000), end: new Date(line.period.end * 1000) };
        // Nothing rolls over: a period is granted the plan's credits once (an upgrade line tops it up to them).
        const credits = Math.max(0, plan.credits - await grantedFor(tx, userId, period.start));
        if (credits > 0) await recordGrant(tx, { userId, key: `invoice:${invoice.id}`, invoiceId: invoice.id, credits, subscriptionId: sub.id, period });
        return userId;
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
    case 'invoice.voided':
    case 'invoice.marked_uncollectible': {
      const invoice = event.data.object;
      if (!invoice.id) return null;
      return async (tx) => reverse(tx, invoice.id!, idOf(invoice.customer), event);
    }
    case 'charge.refunded': {
      const found = await invoiceOfCharge(env, event.data.object);
      if (!found) return null;
      return async (tx) => reverse(tx, found.invoiceId, found.customerId, event);
    }
    case 'charge.dispute.created': {
      const chargeId = idOf(event.data.object.charge as string | { id: string });
      if (!chargeId) return null;
      const found = await invoiceOfCharge(env, await stripeClient(env).charges.retrieve(chargeId));
      if (!found) return null;
      return async (tx) => reverse(tx, found.invoiceId, found.customerId, event);
    }
    default:
      return null;
  }
}

async function reverse(tx: Db, invoiceId: string, customerId: string | null, event: Stripe.Event) {
  const userId = await ownerOf(tx, customerId);
  if (!userId) return null;
  await lockBalance(tx, userId);
  await reverseInvoice(tx, { invoiceId, userId, reason: event.type, eventId: event.id });
  return null;
}

/** Whether a subscription is the account's one surviving subscription: the oldest live one. */
async function isSurviving(db: Db, userId: string, subscriptionId: string): Promise<boolean> {
  const [first] = await db.select({ id: subscriptions.id }).from(subscriptions)
    .where(and(eq(subscriptions.userId, userId), inArray(subscriptions.status, LIVE)))
    .orderBy(asc(subscriptions.createdAt), asc(subscriptions.currentPeriodStart), asc(subscriptions.id)).limit(1);
  return !first || first.id === subscriptionId;
}

/** One account, one subscription. A second live one (two Checkouts completed) is canceled and every invoice it
    was paid with is refunded (the refunds' own events then reverse anything it was granted). Idempotent: Stripe
    calls carry idempotency keys and a canceled subscription is skipped. */
export async function reconcile(data: Data, env: CloudflareBindings, userId: string): Promise<void> {
  const live = await data.db.select({ id: subscriptions.id }).from(subscriptions)
    .where(and(eq(subscriptions.userId, userId), inArray(subscriptions.status, LIVE)))
    .orderBy(asc(subscriptions.createdAt), asc(subscriptions.currentPeriodStart), asc(subscriptions.id));
  if (live.length < 2) return;
  const stripe = stripeClient(env);
  for (const extra of live.slice(1)) {
    console.error('billing: duplicate subscription, canceling and refunding', extra.id);
    await stripe.subscriptions.cancel(extra.id, {}, { idempotencyKey: `reconcile-cancel:${extra.id}` }).catch((error) => console.error('reconcile cancel failed', (error as Error).name));
    await data.db.update(subscriptions).set({ status: 'canceled', updatedAt: new Date() }).where(eq(subscriptions.id, extra.id));
    const invoices = await stripe.invoices.list({ subscription: extra.id, status: 'paid', limit: 100 }).catch(() => null);
    for (const invoice of invoices?.data ?? []) {
      if (!invoice.id || invoiceSubscriptionId(invoice) !== extra.id) continue;
      const payments = await stripe.invoicePayments.list({ invoice: invoice.id, limit: 10 }).catch(() => null);
      const intent = idOf(payments?.data[0]?.payment?.payment_intent as string | { id: string } | undefined);
      if (!intent) continue;
      const invoiceId = invoice.id;
      await stripe.refunds.create({ payment_intent: intent }, { idempotencyKey: `reconcile-refund:${invoiceId}` }).catch((error) => console.error('reconcile refund failed', (error as Error).name));
    }
  }
}

/** Writes Stripe's view of a subscription unless a newer one is already stored. Only a customer the Worker created
    for an account, and only the exact configured prices; anything else is skipped. Returns the account. */
async function upsertSubscription(tx: Db, env: CloudflareBindings, sub: Stripe.Subscription, at: Date): Promise<string | null> {
  const customerId = idOf(sub.customer);
  const item = sub.items?.data?.find(entry => planForPrice(env, entry.price));
  const plan = item ? planForPrice(env, item.price) : null;
  const userId = await ownerOf(tx, customerId);
  if (!customerId || !item || !plan || !userId) return null;
  // A stored subscription never changes hands.
  const [existing] = await tx.select({ userId: subscriptions.userId, customer: subscriptions.stripeCustomerId }).from(subscriptions).where(eq(subscriptions.id, sub.id)).limit(1);
  if (existing && (existing.userId !== userId || existing.customer !== customerId)) return null;
  // A period change waits for any request being admitted against the old one (and the other way round).
  await lockBalance(tx, userId);
  const period = subscriptionPeriod(sub);
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
  return userId;
}
