import { expect, test } from 'bun:test';
import { and, eq } from 'drizzle-orm';
import { withData } from './db';
import { billing, stack, subscribe } from './billing-helpers';
import { creditLedger, stripeEvents, subscriptions } from '../src/db/schema';

test('signed out: every v2 billing and Aero route answers 401 with a code only', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      for (const [method, path] of [['GET', '/v2/billing'], ['POST', '/v2/billing/checkout'], ['POST', '/v2/billing/portal'], ['GET', '/v2/aero/models'], ['POST', '/v2/aero/chat/completions']] as const) {
        const r = await s.request(path, { method, ...(method === 'POST' && { body: '{}' }) }, {});
        expect(r.status).toBe(401);
        expect(await r.json()).toEqual({ error: { code: 'unauthorized' } });
      }
      const bad = await s.request('/v2/billing', {}, { Authorization: 'Bearer not-a-session' });
      expect(bad.status).toBe(401);
    } finally { await s.stop(); }
  }));

test('a new account is Free: no plan, no credits, and the plans it can buy', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      expect(await billing(s)).toEqual({
        plan: null, status: null, cancelAtPeriodEnd: false, credits: { included: 0, used: 0, remaining: 0 }, period: null, daily: [],
        plans: [{ id: 'aerolite', name: 'Aerolite', priceCents: 999, credits: 1000 }, { id: 'aero', name: 'Aero', priceCents: 1999, credits: 2500 }],
      });
    } finally { await s.stop(); }
  }));

test('checkout to active: the webhooks grant the period’s 1,000 credits; a second checkout is refused', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      expect((await s.request('/v2/billing/checkout', { json: { plan: 'gold' } })).status).toBe(400);
      const { url, sessionId, success } = await subscribe(s, 'aerolite');
      expect(url).toBe(`${s.stripe.url}/checkout/${sessionId}`);
      const session = s.stripe.state.sessions.get(sessionId)!;
      // Stripe Tax on, billing address required, tax IDs collected, the user's customer reused, return pages set.
      expect(session.automatic_tax).toEqual({ enabled: true });
      expect(session.billing_address_collection).toBe('required');
      expect(session.tax_id_collection).toEqual({ enabled: true });
      expect(session.client_reference_id).toBe(s.user.id);
      expect(session.success_url).toBe('http://localhost/v2/billing/return?status=success');
      expect(session.cancel_url).toBe('http://localhost/v2/billing/return?status=cancel');
      expect(success).toBe(session.success_url);
      expect(s.stripe.deliveries.map(d => [d.type, d.status])).toEqual([['customer.subscription.created', 200], ['invoice.paid', 200], ['checkout.session.completed', 200]]);
      const state = await billing(s);
      expect(state.plan).toEqual({ id: 'aerolite', name: 'Aerolite', priceCents: 999, interval: 'month' });
      expect(state.status).toBe('active');
      expect(state.cancelAtPeriodEnd).toBe(false);
      expect(state.credits).toEqual({ included: 1000, used: 0, remaining: 1000 });
      expect(new Date(state.period.end).getTime()).toBeGreaterThan(new Date(state.period.start).getTime());
      expect(state.daily).toEqual([{ date: new Date().toISOString().slice(0, 10), credits: 0 }]);
      const again = await s.request('/v2/billing/checkout', { json: { plan: 'aero' } });
      expect(again.status).toBe(409);
      expect(await again.json()).toEqual({ error: { code: 'has_plan' } });
      expect(s.stripe.state.customers.size).toBe(1);
      const portal = await s.request('/v2/billing/portal', { method: 'POST' });
      expect(portal.status).toBe(200);
      const portalUrl = (await portal.json() as any).url as string;
      expect(portalUrl.startsWith(`${s.stripe.url}/portal/`)).toBe(true);
      expect(s.stripe.state.sessions.get(portalUrl.split('/').at(-1)!)!.return_url).toBe('http://localhost/v2/billing/return?status=portal');
    } finally { await s.stop(); }
  }));

test('the return page hands back to the app', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      for (const status of ['success', 'cancel']) {
        const r = await s.request(`/v2/billing/return?status=${status}`, {}, {});
        expect(r.status).toBe(200);
        const html = await r.text();
        expect(html).toContain('You can return to Powermove');
        expect(html).toContain(`powermove://billing/return?status=${status}`);
        expect(r.headers.get('content-security-policy')).toContain("default-src 'none'");
      }
      expect(await (await s.request('/v2/billing/return?status=<script>', {}, {})).text()).toContain('powermove://billing/return?status=portal');
    } finally { await s.stop(); }
  }));

test('the webhook is verified and idempotent: replayed events grant nothing twice', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const forged = await s.app.request('/v2/stripe/webhook', { method: 'POST', headers: { 'stripe-signature': 't=1,v1=00' }, body: '{"id":"evt_x","type":"invoice.paid"}' }, s.env);
      expect(forged.status).toBe(400);
      expect(await forged.json()).toEqual({ error: { code: 'invalid_signature' } });
      await subscribe(s);
      const before = (await data.db.select().from(creditLedger)).length;
      for (const event of [...s.stripe.state.events]) expect(await s.stripe.redeliver(event.id)).toBe(200);
      expect((await data.db.select().from(creditLedger)).length).toBe(before);
      expect((await data.db.select().from(stripeEvents)).length).toBe(3);
      expect((await billing(s)).credits).toEqual({ included: 1000, used: 0, remaining: 1000 });
    } finally { await s.stop(); }
  }));

test('a renewal resets credits to the plan’s grant; nothing rolls over', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const { subscriptionId } = await subscribe(s);
      await data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'spend', idempotencyKey: 'request:test:one', credits: 188, model: 'openai/gpt-6.2-sol' });
      expect((await billing(s)).credits).toEqual({ included: 1000, used: 188, remaining: 812 });
      await new Promise(resolve => setTimeout(resolve, 1100)); // the new period starts on a later second
      await s.stripe.renew(subscriptionId);
      const state = await billing(s);
      expect(state.credits).toEqual({ included: 1000, used: 0, remaining: 1000 });
      expect(state.status).toBe('active');
      const grants = await data.db.select().from(creditLedger).where(eq(creditLedger.kind, 'grant'));
      expect(grants.map(g => g.credits)).toEqual([1000, 1000]);
    } finally { await s.stop(); }
  }));

test('an upgrade tops the period up to the new plan; a failed renewal is past_due with no new credits', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const { subscriptionId } = await subscribe(s);
      await s.stripe.switchPlan(subscriptionId, 'powermove_aero_monthly');
      let state = await billing(s);
      expect(state.plan.id).toBe('aero');
      expect(state.credits.included).toBe(2500);
      await new Promise(resolve => setTimeout(resolve, 1100));
      await s.stripe.failRenewal(subscriptionId);
      state = await billing(s);
      expect(state.status).toBe('past_due');
      expect(state.credits).toEqual({ included: 0, used: 0, remaining: 0 });
      const r = await s.request('/v2/aero/chat/completions', { json: { model: 'openai/gpt-6.2-sol', stream: true, messages: [{ role: 'user', content: 'hi' }] } });
      expect(r.status).toBe(402);
    } finally { await s.stop(); }
  }));

test('cancel at period end keeps the plan to the end, then it ends', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const { subscriptionId } = await subscribe(s);
      await s.stripe.setCancelAtPeriodEnd(subscriptionId, true);
      let state = await billing(s);
      expect(state.status).toBe('active');
      expect(state.cancelAtPeriodEnd).toBe(true);
      expect(state.credits.remaining).toBe(1000);
      await s.stripe.setCancelAtPeriodEnd(subscriptionId, false);
      expect((await billing(s)).cancelAtPeriodEnd).toBe(false);
      await s.stripe.setCancelAtPeriodEnd(subscriptionId, true);
      await s.stripe.end(subscriptionId);
      state = await billing(s);
      expect(state.plan).toBeNull();
      expect(state.status).toBe('canceled');
      expect(state.credits).toEqual({ included: 0, used: 0, remaining: 0 });
      const [row] = await data.db.select().from(subscriptions).where(and(eq(subscriptions.id, subscriptionId)));
      expect(row!.status).toBe('canceled');
      // Ended: a new checkout is allowed again.
      expect((await s.request('/v2/billing/checkout', { json: { plan: 'aero' } })).status).toBe(200);
    } finally { await s.stop(); }
  }));
