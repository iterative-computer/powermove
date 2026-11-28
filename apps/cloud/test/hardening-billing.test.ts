/* Billing hardening: reversals, paid-only spending, strict grants, Stripe ownership, one Checkout and one
   subscription per account. Every event here is a real signed webhook through the fake Stripe. */
import { expect, test } from 'bun:test';
import { eq } from 'drizzle-orm';
import { withData } from './db';
import { billing, priceId, stack, subscribe, type Stack } from './billing-helpers';
import { billingReversals, creditLedger, subscriptions } from '../src/db/schema';
import { seedSession } from './helpers';
import type { Data } from '../src/db/client';

const chat = (s: Stack) => s.request('/v2/aero/chat/completions', { json: { model: 'openai/gpt-6-luna', stream: true, messages: [{ role: 'user', content: 'hi' }] } });
const grants = (data: Data, userId: string) => data.db.select().from(creditLedger).where(eq(creditLedger.userId, userId)).then(rows => rows.filter(row => row.kind === 'grant'));
const firstInvoice = (s: Stack, subscriptionId: string) => [...s.stripe.state.invoices.values()].find(invoice => invoice.parent.subscription_details.subscription === subscriptionId)!;

for (const [name, act] of [
  ['a refund', (s: Stack, invoiceId: string) => s.stripe.refund(invoiceId)],
  ['a dispute', (s: Stack, invoiceId: string) => s.stripe.dispute(invoiceId)],
  ['an invoice marked uncollectible', (s: Stack, invoiceId: string) => s.stripe.markUncollectible(invoiceId)],
  ['a voided invoice', (s: Stack, invoiceId: string) => s.stripe.voidInvoice(invoiceId)],
] as const) {
  test(`1. ${name} zeroes the period's remaining credits, once, and Aero refuses to spend`, async () =>
    withData(async (data) => {
      const s = await stack(data);
      try {
        const { subscriptionId } = await subscribe(s);
        expect((await billing(s)).credits.remaining).toBe(1000);
        const invoice = firstInvoice(s, subscriptionId);
        await act(s, invoice.id);
        expect((await billing(s)).credits).toEqual({ included: 0, used: 0, remaining: 0 });
        expect((await chat(s)).status).toBe(402);
        // A redelivery, and the invoice's paid event arriving again, change nothing.
        const last = s.stripe.deliveries.at(-1)!;
        await s.stripe.redeliver(last.id);
        await s.stripe.emit('invoice.paid', invoice);
        expect(await data.db.select().from(billingReversals)).toHaveLength(1);
        expect((await billing(s)).credits.included).toBe(0);
        expect((await grants(data, s.user.id)).every(grant => grant.revokedAt !== null)).toBe(true);
      } finally { await s.stop(); }
    }));
}

test('1. only a currently paid entitlement spends: active or trialing; past_due, unpaid, paused, incomplete, canceled get 402', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const { subscriptionId } = await subscribe(s);
      for (const [status, code] of [['trialing', 200], ['past_due', 402], ['unpaid', 402], ['paused', 402], ['incomplete', 402], ['canceled', 402], ['active', 200]] as const) {
        await data.db.update(subscriptions).set({ status }).where(eq(subscriptions.id, subscriptionId));
        const r = await chat(s);
        await r.text();
        expect([status, r.status]).toEqual([status, code]);
      }
    } finally { await s.stop(); }
  }));

/** A paid invoice for `sub`, shaped like Stripe's, with the given lines. */
function invoiceOf(s: Stack, sub: Record<string, any>, lines: Array<{ price: string; subscription?: string; period?: { start: number; end: number }; amount?: number }>, extra: Record<string, unknown> = {}) {
  const item = sub.items.data[0];
  return {
    id: `in_test${crypto.randomUUID().replaceAll('-', '').slice(0, 16)}`, object: 'invoice', customer: sub.customer, billing_reason: 'subscription_cycle', status: 'paid', currency: 'usd',
    amount_due: 999, amount_paid: 999, created: Math.floor(Date.now() / 1000), livemode: false,
    parent: { type: 'subscription_details', subscription_details: { subscription: sub.id, metadata: {} } },
    lines: { object: 'list', has_more: false, data: lines.map(line => ({ id: `il_${crypto.randomUUID().slice(0, 8)}`, object: 'line_item', amount: line.amount ?? 999, currency: 'usd', period: line.period ?? { start: item.current_period_start, end: item.current_period_end }, pricing: { type: 'price_details', price_details: { price: line.price } }, parent: { type: 'subscription_item_details', subscription_item_details: { subscription: line.subscription ?? sub.id, subscription_item: item.id } } })) },
    ...extra,
  };
}

test('2. invoice.paid grants only for a paid invoice that took money, from its exact Aerolite line on this subscription, with that line’s period', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const { subscriptionId } = await subscribe(s);
      const sub = s.stripe.state.subscriptions.get(subscriptionId)!;
      // A new period starts; then invoices that must not grant it, and one that must.
      const start = sub.items.data[0].current_period_end, end = start + 30 * 86_400;
      Object.assign(sub.items.data[0], { current_period_start: start, current_period_end: end });
      const period = { start, end };
      const aerolite = priceId(s.stripe, 'powermove_aerolite_monthly');
      await s.stripe.emit('invoice.paid', invoiceOf(s, sub, [{ price: aerolite, period }], { amount_paid: 0 }));
      await s.stripe.emit('invoice.paid', invoiceOf(s, sub, [{ price: aerolite, period }], { status: 'open' }));
      await s.stripe.emit('invoice.paid', invoiceOf(s, sub, [{ price: 'price_somethingelse', period }]));
      await s.stripe.emit('invoice.paid', invoiceOf(s, sub, [{ price: aerolite, period, subscription: 'sub_another' }]));
      expect(await grants(data, s.user.id)).toHaveLength(1);
      // The right one: credits for the line's own period, unrelated lines ignored.
      const linePeriod = { start: start + 3600, end: end - 3600 };
      await s.stripe.emit('invoice.paid', invoiceOf(s, sub, [{ price: 'price_unrelated', amount: 5000 }, { price: aerolite, period: linePeriod }]));
      const rows = await grants(data, s.user.id);
      expect(rows).toHaveLength(2);
      expect(rows[1]).toMatchObject({ credits: 1000 });
      expect(rows[1]!.periodStart.getTime()).toBe(linePeriod.start * 1000);
      expect(rows[1]!.periodEnd.getTime()).toBe(linePeriod.end * 1000);
    } finally { await s.stop(); }
  }));

test('7. ownership comes only from the Worker’s customer mapping; client_reference_id, metadata and lookup keys decide nothing', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const victim = await seedSession(data, s.env, 'victim@example.com');
      // An attacker's own Stripe customer (not created by the Worker for anyone) claims the victim in every field.
      const aerolite = s.stripe.state.prices.get(priceId(s.stripe, 'powermove_aerolite_monthly'))!;
      const now = Math.floor(Date.now() / 1000);
      const sub = { id: 'sub_attacker0001', object: 'subscription', customer: 'cus_attacker0001', status: 'active', cancel_at_period_end: false, cancel_at: null, metadata: { userId: victim.id, plan: 'aerolite' },
        items: { object: 'list', data: [{ id: 'si_attacker', object: 'subscription_item', price: aerolite, quantity: 1, current_period_start: now, current_period_end: now + 2_592_000 }] } };
      s.stripe.state.subscriptions.set(sub.id, sub);
      await s.stripe.emit('customer.subscription.created', sub);
      await s.stripe.emit('checkout.session.completed', { id: 'cs_attacker0001', object: 'checkout.session', mode: 'subscription', customer: sub.customer, subscription: sub.id, client_reference_id: victim.id, metadata: { userId: victim.id } });
      await s.stripe.emit('invoice.paid', invoiceOf(s, sub, [{ price: aerolite.id }], { billing_reason: 'subscription_create' }));
      expect(await data.db.select().from(subscriptions)).toHaveLength(0);
      expect(await grants(data, victim.id)).toHaveLength(0);
      // A price that merely carries Aerolite's lookup key and metadata is not Aerolite.
      const { subscriptionId } = await subscribe(s);
      const real = s.stripe.state.subscriptions.get(subscriptionId)!;
      const fake = { ...aerolite, id: 'price_lookalike', lookup_key: 'powermove_aerolite_monthly', metadata: { plan: 'aerolite' } };
      await s.stripe.emit('customer.subscription.updated', { ...real, items: { object: 'list', data: [{ ...real.items.data[0], price: fake }] } });
      expect((await data.db.select().from(subscriptions).where(eq(subscriptions.id, subscriptionId)))[0]!.stripePriceId).toBe(aerolite.id);
      // An invoice whose customer isn't its subscription's: ignored.
      const before = (await grants(data, s.user.id)).length;
      await s.stripe.emit('invoice.paid', invoiceOf(s, real, [{ price: aerolite.id, period: { start: now + 9_000_000, end: now + 9_100_000 } }], { customer: 'cus_someoneelse' }));
      expect(await grants(data, s.user.id)).toHaveLength(before);
    } finally { await s.stop(); }
  }));

test('10. one pending Checkout per account, reused until it expires; a second subscription is canceled and refunded', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const urls = await Promise.all([1, 2, 3].map(async () => (await (await s.request('/v2/billing/checkout', { json: { plan: 'aerolite' } })).json() as { url: string }).url));
      expect(new Set(urls).size).toBe(1);
      expect([...s.stripe.state.sessions.values()].filter(session => session.object === 'checkout.session')).toHaveLength(1);
      // Two Checkouts completed anyway (a session opened elsewhere for the same customer): the newer subscription
      // is canceled and its payment refunded; the account keeps one plan and one period of credits.
      const first = urls[0]!.split('/').at(-1)!;
      const customer = s.stripe.state.sessions.get(first)!.customer;
      const second = await (await fetch(`${s.stripe.url}/v1/checkout/sessions`, { method: 'POST', headers: { authorization: `Bearer ${s.env.STRIPE_SECRET_KEY}`, 'content-type': 'application/x-www-form-urlencoded' },
        body: new URLSearchParams({ mode: 'subscription', customer, 'line_items[0][price]': priceId(s.stripe, 'powermove_aerolite_monthly'), 'line_items[0][quantity]': '1', success_url: 'https://x/s', cancel_url: 'https://x/c' }) })).json() as { id: string };
      await s.stripe.pay(first);
      await new Promise(resolve => setTimeout(resolve, 1100));
      await s.stripe.pay(second.id);
      const subs = [...s.stripe.state.subscriptions.values()];
      expect(subs).toHaveLength(2);
      const [older, newer] = subs;
      expect(older!.status).toBe('active');
      expect(newer!.status).toBe('canceled');
      expect(s.stripe.state.charges.get(firstInvoice(s, newer!.id).charge)!.refunded).toBe(true);
      expect(s.stripe.state.charges.get(firstInvoice(s, older!.id).charge)!.refunded).toBe(false);
      const state = await billing(s);
      expect(state.status).toBe('active');
      expect(state.credits).toEqual({ included: 1000, used: 0, remaining: 1000 });
    } finally { await s.stop(); }
  }));

test('2b. a $0 invoice grants only when our own comp coupon zeroed it', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      s.env.COMP_COUPONS = 'comp-forever';
      const { subscriptionId } = await subscribe(s);
      const sub = s.stripe.state.subscriptions.get(subscriptionId)!;
      const start = sub.items.data[0].current_period_end, end = start + 30 * 86_400;
      Object.assign(sub.items.data[0], { current_period_start: start, current_period_end: end });
      const aerolite = priceId(s.stripe, 'powermove_aerolite_monthly');
      const zero = (coupon: string) => invoiceOf(s, sub, [{ price: aerolite, period: { start, end }, amount: 0 }], { amount_paid: 0, amount_due: 0, discounts: [{ id: 'di_x', object: 'discount', source: { type: 'coupon', coupon } }] });
      await s.stripe.emit('invoice.paid', zero('someone-elses-coupon'));
      expect(await grants(data, s.user.id)).toHaveLength(1);
      await s.stripe.emit('invoice.paid', zero('comp-forever'));
      const rows = await grants(data, s.user.id);
      expect(rows).toHaveLength(2);
      expect(rows[1]).toMatchObject({ credits: 1000 });
    } finally { await s.stop(); }
  }));
