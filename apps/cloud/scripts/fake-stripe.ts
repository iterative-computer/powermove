/* A small local Stripe for development and tests. The `emulate` package's Stripe has no subscriptions, invoices,
   Customer Portal or webhooks, so this covers the part of Stripe the Worker uses:
   - the REST calls (customers, prices and products by lookup key, Checkout Sessions, Billing Portal Sessions,
     subscription reads), form-encoded the way the Stripe SDK sends them;
   - a hosted Checkout page with a Pay button, and a Portal page (cancel at period end, resume, switch plan);
   - signed webhooks (`Stripe-Signature: t=…,v1=…`) posted to the Worker, in the current API's object shapes;
   - development controls: POST /_fake/subscriptions/:id/renew (a new period starts now, its invoice is paid),
     /fail-renewal (the renewal payment fails), /end (a subscription set to cancel at period end ends);
   - payments: every paid invoice has a payment intent and a charge (invoice payments list them), refunds
     (charge.refunded), and controls for a dispute (charge.dispute.created), a voided or uncollectible invoice.

   bun scripts/fake-stripe.ts [--port 12111] [--webhook http://localhost:8787/v2/stripe/webhook]
   Worker env: STRIPE_API_BASE=http://localhost:12111 STRIPE_SECRET_KEY=sk_test_fake STRIPE_WEBHOOK_SECRET=whsec_fake */

export const FAKE_SECRET_KEY = 'sk_test_powermove_local_fake';
export const FAKE_WEBHOOK_SECRET = 'whsec_powermove_local_fake';

type Obj = Record<string, any>;
export interface FakeStripe {
  url: string;
  /** Every webhook delivered, with the Worker's status. */
  deliveries: Array<{ type: string; id: string; status: number }>;
  state: { customers: Map<string, Obj>; prices: Map<string, Obj>; products: Map<string, Obj>; sessions: Map<string, Obj>; subscriptions: Map<string, Obj>; invoices: Map<string, Obj>; charges: Map<string, Obj>; events: Obj[] };
  /** Completes a Checkout Session as if the Pay button was pressed. Returns the success URL. */
  pay(sessionId: string, backdateDays?: number): Promise<string>;
  renew(subscriptionId: string): Promise<void>;
  failRenewal(subscriptionId: string): Promise<void>;
  /** The portal's actions, without the page. */
  setCancelAtPeriodEnd(subscriptionId: string, value: boolean): Promise<void>;
  switchPlan(subscriptionId: string, lookupKey: string): Promise<void>;
  end(subscriptionId: string): Promise<void>;
  /** Refunds an invoice's payment (as the Dashboard or the API would). */
  refund(invoiceId: string): Promise<void>;
  /** The customer's bank disputes an invoice's payment. */
  dispute(invoiceId: string): Promise<void>;
  /** An open invoice voided, or marked uncollectible. */
  voidInvoice(invoiceId: string): Promise<void>;
  markUncollectible(invoiceId: string): Promise<void>;
  /** Sends an event of any shape, signed (a test's own event). */
  emit(type: string, object: Obj): Promise<void>;
  /** Sends an already-delivered event again (a Stripe retry). */
  redeliver(eventId: string): Promise<number>;
  stop(): Promise<void>;
}

export interface FakeStripeOptions {
  port?: number;
  webhookUrl?: string;
  webhookSecret?: string;
  /** Delivers a signed webhook somewhere other than a URL (tests hand it to the app directly). */
  deliver?: (payload: string, signature: string) => Promise<number>;
  /** Creates the two Powermove prices at start (otherwise `bun run stripe:setup` does). */
  seedPrices?: boolean;
}

const API_VERSION = '2026-11-18.endive';
const id = (prefix: string) => `${prefix}_${crypto.randomUUID().replaceAll('-', '').slice(0, 24)}`;
const now = () => Math.floor(Date.now() / 1000);
const addMonth = (seconds: number) => { const date = new Date(seconds * 1000); date.setUTCMonth(date.getUTCMonth() + 1); return Math.floor(date.getTime() / 1000); };
const escape = (text: string) => text.replace(/[&<>"']/g, ch => `&#${ch.charCodeAt(0)};`);

/** Stripe's form encoding (`a[b][0][c]=d`) back into objects and arrays. */
export function parseForm(text: string): Obj {
  const root: Obj = {};
  for (const [rawKey, value] of new URLSearchParams(text)) {
    const keys = rawKey.split(/\[|\]\[|\]/).filter(Boolean);
    let node: any = root;
    keys.forEach((key, index) => {
      const last = index === keys.length - 1;
      if (last) { if (Array.isArray(node)) node.push(value); else node[key] = value; return; }
      const nextIsIndex = /^\d+$/.test(keys[index + 1]!);
      node[key] ??= nextIsIndex ? [] : {};
      node = node[key];
    });
  }
  // Arrays came in as objects keyed by index; normalise.
  const normalise = (value: any): any => {
    if (Array.isArray(value)) return value.map(normalise);
    if (value && typeof value === 'object') {
      const keys = Object.keys(value);
      if (keys.length && keys.every(key => /^\d+$/.test(key))) return keys.sort((a, b) => Number(a) - Number(b)).map(key => normalise(value[key]));
      return Object.fromEntries(keys.map(key => [key, normalise(value[key])]));
    }
    return value;
  };
  return normalise(root);
}

async function sign(secret: string, payload: string, timestamp = now()): Promise<string> {
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['sign']);
  const mac = await crypto.subtle.sign('HMAC', key, new TextEncoder().encode(`${timestamp}.${payload}`));
  return `t=${timestamp},v1=${Array.from(new Uint8Array(mac), byte => byte.toString(16).padStart(2, '0')).join('')}`;
}

export function startFakeStripe(options: FakeStripeOptions = {}): FakeStripe {
  const secret = options.webhookSecret ?? FAKE_WEBHOOK_SECRET;
  const state: FakeStripe['state'] = { customers: new Map(), prices: new Map(), products: new Map(), sessions: new Map(), subscriptions: new Map(), invoices: new Map(), charges: new Map(), events: [] };
  const deliveries: FakeStripe['deliveries'] = [];
  let base = '';

  function createProduct(name: string, metadata: Obj = {}) {
    const product = { id: id('prod'), object: 'product', name, active: true, metadata, created: now(), livemode: false };
    state.products.set(product.id, product);
    return product;
  }
  function createPrice(params: Obj) {
    const price = {
      id: id('price'), object: 'price', active: true, currency: params.currency ?? 'usd', unit_amount: Number(params.unit_amount ?? 0),
      product: params.product, lookup_key: params.lookup_key ?? null, metadata: params.metadata ?? {}, type: 'recurring',
      recurring: { interval: params.recurring?.interval ?? 'month', interval_count: 1 }, tax_behavior: params.tax_behavior ?? 'unspecified', livemode: false, created: now(),
    };
    state.prices.set(price.id, price);
    return price;
  }
  if (options.seedPrices) {
    for (const [name, lookup_key, plan, amount] of [['Powermove Aerolite', 'powermove_aerolite_monthly', 'aerolite', 999], ['Powermove Aero', 'powermove_aero_monthly', 'aero', 1999]] as const) {
      const product = createProduct(name, { plan });
      createPrice({ product: product.id, unit_amount: amount, lookup_key, metadata: { plan }, recurring: { interval: 'month' }, tax_behavior: 'exclusive' });
    }
  }
  const priceByLookup = (key: string) => [...state.prices.values()].find(price => price.lookup_key === key && price.active);

  async function emit(type: string, object: Obj) {
    const event = { id: id('evt'), object: 'event', api_version: API_VERSION, created: now(), type, livemode: false, pending_webhooks: 1, request: { id: null, idempotency_key: null }, data: { object: structuredClone(object) } };
    state.events.push(event);
    await send(event);
  }
  async function send(event: Obj): Promise<number> {
    const payload = JSON.stringify(event);
    const signature = await sign(secret, payload);
    let status = 0;
    try {
      if (options.deliver) status = await options.deliver(payload, signature);
      else if (options.webhookUrl) status = (await fetch(options.webhookUrl, { method: 'POST', headers: { 'content-type': 'application/json', 'stripe-signature': signature }, body: payload })).status;
    } catch { status = 0; }
    deliveries.push({ type: event.type, id: event.id, status });
    return status;
  }

  function invoiceFor(sub: Obj, reason: string, period: { start: number; end: number }, amount: number, status: 'paid' | 'open') {
    const item = sub.items.data[0];
    const invoice = {
      id: id('in'), object: 'invoice', customer: sub.customer, billing_reason: reason, status, currency: 'usd',
      amount_due: amount, amount_paid: status === 'paid' ? amount : 0, created: now(), livemode: false,
      parent: { type: 'subscription_details', subscription_details: { subscription: sub.id, metadata: sub.metadata } },
      lines: { object: 'list', has_more: false, data: [{ id: id('il'), object: 'line_item', amount, currency: 'usd', period, pricing: { type: 'price_details', price_details: { price: item.price.id, product: item.price.product } }, parent: { type: 'subscription_item_details', subscription_item_details: { subscription: sub.id, subscription_item: item.id, proration: false } } }] },
    } as Obj;
    if (status === 'paid') {
      const charge = { id: id('ch'), object: 'charge', amount, amount_refunded: 0, currency: 'usd', customer: sub.customer, payment_intent: id('pi'), paid: true, refunded: false, disputed: false, livemode: false, created: now() };
      state.charges.set(charge.id, charge);
      invoice.payment_intent = charge.payment_intent;
      invoice.charge = charge.id;
    }
    state.invoices.set(invoice.id, invoice);
    return invoice;
  }

  /** `backdateDays` starts the period that many days ago (local seed data with a few days of history). */
  async function pay(sessionId: string, backdateDays = 0): Promise<string> {
    const session = state.sessions.get(sessionId);
    if (!session) throw new Error('no such session');
    if (session.status === 'complete') return session.success_url;
    const price = state.prices.get(session.line_items[0].price);
    if (!price) throw new Error('no such price');
    const start = now() - Math.round(backdateDays * 86_400), end = addMonth(start);
    const sub = {
      id: id('sub'), object: 'subscription', customer: session.customer, status: 'active', cancel_at_period_end: false, cancel_at: null, canceled_at: null,
      created: start, livemode: false, metadata: session.subscription_data?.metadata ?? {}, automatic_tax: { enabled: true },
      items: { object: 'list', has_more: false, data: [{ id: id('si'), object: 'subscription_item', price, quantity: 1, current_period_start: start, current_period_end: end }] },
    };
    state.subscriptions.set(sub.id, sub);
    Object.assign(session, { status: 'complete', payment_status: 'paid', subscription: sub.id });
    const invoice = invoiceFor(sub, 'subscription_create', { start, end }, price.unit_amount, 'paid');
    // Stripe doesn't order these; the Worker must not depend on it.
    await emit('customer.subscription.created', sub);
    await emit('invoice.paid', invoice);
    await emit('checkout.session.completed', session);
    return session.success_url;
  }

  async function renew(subscriptionId: string) {
    const sub = state.subscriptions.get(subscriptionId);
    if (!sub) throw new Error('no such subscription');
    const item = sub.items.data[0];
    const start = Math.max(now(), item.current_period_start + 1), end = addMonth(start);
    item.current_period_start = start; item.current_period_end = end;
    sub.status = 'active';
    const invoice = invoiceFor(sub, 'subscription_cycle', { start, end }, item.price.unit_amount, 'paid');
    await emit('invoice.paid', invoice);
    await emit('customer.subscription.updated', sub);
  }

  async function failRenewal(subscriptionId: string) {
    const sub = state.subscriptions.get(subscriptionId);
    if (!sub) throw new Error('no such subscription');
    const item = sub.items.data[0];
    const start = Math.max(now(), item.current_period_start + 1), end = addMonth(start);
    item.current_period_start = start; item.current_period_end = end;
    const invoice = invoiceFor(sub, 'subscription_cycle', { start, end }, item.price.unit_amount, 'open');
    await emit('invoice.payment_failed', invoice);
    sub.status = 'past_due';
    await emit('customer.subscription.updated', sub);
  }

  async function setCancelAtPeriodEnd(subscriptionId: string, value: boolean) {
    const sub = state.subscriptions.get(subscriptionId);
    if (!sub) throw new Error('no such subscription');
    sub.cancel_at_period_end = value;
    sub.cancel_at = value ? sub.items.data[0].current_period_end : null;
    await emit('customer.subscription.updated', sub);
  }

  async function switchPlan(subscriptionId: string, lookupKey: string) {
    const sub = state.subscriptions.get(subscriptionId);
    const price = priceByLookup(lookupKey);
    if (!sub || !price) throw new Error('no such subscription or price');
    const item = sub.items.data[0];
    const upgrade = price.unit_amount > item.price.unit_amount;
    const prorated = Math.round((price.unit_amount - item.price.unit_amount) * (item.current_period_end - now()) / (item.current_period_end - item.current_period_start));
    item.price = price;
    await emit('customer.subscription.updated', sub);
    // An upgrade is invoiced now (Stripe's default proration); a downgrade leaves a credit for the next invoice.
    if (upgrade) await emit('invoice.paid', invoiceFor(sub, 'subscription_update', { start: item.current_period_start, end: item.current_period_end }, prorated, 'paid'));
  }

  const chargeOf = (invoiceId: string) => {
    const invoice = state.invoices.get(invoiceId);
    const charge = invoice?.charge && state.charges.get(invoice.charge);
    if (!charge) throw new Error('no payment for that invoice');
    return charge;
  };
  async function refundCharge(charge: Obj) {
    Object.assign(charge, { refunded: true, amount_refunded: charge.amount });
    await emit('charge.refunded', charge);
  }
  async function refund(invoiceId: string) { await refundCharge(chargeOf(invoiceId)); }
  async function dispute(invoiceId: string) {
    const charge = chargeOf(invoiceId);
    charge.disputed = true;
    await emit('charge.dispute.created', { id: id('dp'), object: 'dispute', amount: charge.amount, charge: charge.id, payment_intent: charge.payment_intent, status: 'needs_response', reason: 'fraudulent', livemode: false, created: now() });
  }
  async function closeInvoice(invoiceId: string, status: 'void' | 'uncollectible') {
    const invoice = state.invoices.get(invoiceId);
    if (!invoice) throw new Error('no such invoice');
    invoice.status = status;
    await emit(status === 'void' ? 'invoice.voided' : 'invoice.marked_uncollectible', invoice);
  }

  async function end(subscriptionId: string) {
    const sub = state.subscriptions.get(subscriptionId);
    if (!sub) throw new Error('no such subscription');
    Object.assign(sub, { status: 'canceled', canceled_at: now(), ended_at: now() });
    await emit('customer.subscription.deleted', sub);
  }

  const list = (data: Obj[], url: string) => ({ object: 'list', data, has_more: false, url });
  const error = (status: number, message: string) => Response.json({ error: { type: 'invalid_request_error', message } }, { status });
  const html = (title: string, body: string) => new Response(`<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>${escape(title)}</title><style>body{font:15px/1.5 -apple-system,system-ui,sans-serif;max-width:420px;margin:64px auto;padding:0 16px;color:#1b1d23}h1{font-size:20px;font-weight:600}.muted{color:#6b6f78}button,a.button{display:inline-block;margin:6px 6px 0 0;padding:10px 16px;border-radius:6px;border:0;background:#635bff;color:#fff;font:inherit;text-decoration:none}button.secondary,a.secondary{background:#e8e8ee;color:#1b1d23}form{display:inline}.card{border:1px solid #e3e3e8;border-radius:10px;padding:20px;margin:16px 0}</style></head><body><p class="muted">Local fake Stripe · no real payment</p>${body}</body></html>`, { headers: { 'content-type': 'text/html; charset=utf-8' } });

  const server = Bun.serve({
    port: options.port ?? 0,
    async fetch(request) {
      const url = new URL(request.url);
      const path = url.pathname;
      const method = request.method;
      const form = method === 'POST' ? parseForm(await request.text()) : {};
      let match: RegExpMatchArray | null;

      // Hosted pages (what the browser sees).
      if ((match = path.match(/^\/checkout\/(cs_\w+)$/)) && method === 'GET') {
        const session = state.sessions.get(match[1]!);
        if (!session) return error(404, 'no such session');
        const price = state.prices.get(session.line_items[0].price);
        const product = price && state.products.get(price.product);
        return html('Checkout', `<h1>${escape(product?.name ?? 'Subscription')}</h1><div class="card"><p>$${((price?.unit_amount ?? 0) / 100).toFixed(2)} a month, plus tax</p><p class="muted">Billing address and tax ID would be collected here.</p><form method="post" action="/checkout/${session.id}/pay"><button type="submit">Pay</button></form><a class="button secondary" href="${escape(session.cancel_url)}">Cancel</a></div>`);
      }
      if ((match = path.match(/^\/checkout\/(cs_\w+)\/pay$/)) && method === 'POST') {
        try { return Response.redirect(await pay(match[1]!), 303); } catch (e) { return error(400, (e as Error).message); }
      }
      if ((match = path.match(/^\/portal\/(bps_\w+)$/)) && method === 'GET') {
        const portal = state.sessions.get(match[1]!);
        if (!portal) return error(404, 'no such portal session');
        const subs = [...state.subscriptions.values()].filter(sub => sub.customer === portal.customer && sub.status !== 'canceled');
        const rows = subs.map(sub => {
          const price = sub.items.data[0].price;
          const other = price.lookup_key === 'powermove_aero_monthly' ? 'powermove_aerolite_monthly' : 'powermove_aero_monthly';
          return `<div class="card"><p><strong>${escape(state.products.get(price.product)?.name ?? price.id)}</strong> · ${escape(sub.status)}${sub.cancel_at_period_end ? ' · cancels at period end' : ''}</p>
            <form method="post" action="/portal/${portal.id}/action?sub=${sub.id}&do=${sub.cancel_at_period_end ? 'resume' : 'cancel'}"><button class="secondary">${sub.cancel_at_period_end ? 'Resume' : 'Cancel plan'}</button></form>
            <form method="post" action="/portal/${portal.id}/action?sub=${sub.id}&do=switch&to=${other}"><button class="secondary">Switch plan</button></form></div>`;
        }).join('');
        return html('Billing', `<h1>Billing</h1>${rows || '<p>No active plan.</p>'}<a class="button" href="${escape(portal.return_url)}">Return to Powermove</a>`);
      }
      if ((match = path.match(/^\/portal\/(bps_\w+)\/action$/)) && method === 'POST') {
        const portal = state.sessions.get(match[1]!);
        const sub = state.subscriptions.get(url.searchParams.get('sub') ?? '');
        if (!portal || !sub || sub.customer !== portal.customer) return error(404, 'no such subscription');
        const action = url.searchParams.get('do');
        if (action === 'cancel' || action === 'resume') await setCancelAtPeriodEnd(sub.id, action === 'cancel');
        else if (action === 'switch') await switchPlan(sub.id, url.searchParams.get('to') ?? '');
        return Response.redirect(`${base}/portal/${portal.id}`, 303);
      }
      // Development controls.
      if ((match = path.match(/^\/_fake\/subscriptions\/(sub_\w+)\/(renew|fail-renewal|end|cancel-at-period-end)$/)) && method === 'POST') {
        const [, subId, action] = match;
        try {
          if (action === 'renew') await renew(subId!);
          else if (action === 'fail-renewal') await failRenewal(subId!);
          else if (action === 'end') await end(subId!);
          else await setCancelAtPeriodEnd(subId!, true);
        } catch (e) { return error(404, (e as Error).message); }
        return Response.json(state.subscriptions.get(subId!));
      }
      if (path === '/_fake/state' && method === 'GET') {
        return Response.json({ customers: [...state.customers.values()], subscriptions: [...state.subscriptions.values()], deliveries });
      }

      // The REST API.
      if (!path.startsWith('/v1/')) return error(404, 'not found');
      if (!request.headers.get('authorization')?.startsWith('Bearer sk_test_')) return error(401, 'invalid api key');
      if (path === '/v1/customers' && method === 'POST') {
        const customer = { id: id('cus'), object: 'customer', email: form.email ?? null, name: form.name ?? null, metadata: form.metadata ?? {}, created: now(), livemode: false };
        state.customers.set(customer.id, customer);
        return Response.json(customer);
      }
      if (path === '/v1/products' && method === 'POST') return Response.json(createProduct(form.name, form.metadata));
      if (path === '/v1/products' && method === 'GET') return Response.json(list([...state.products.values()], path));
      if (path === '/v1/prices' && method === 'POST') {
        if (form.lookup_key && priceByLookup(form.lookup_key)) return error(400, 'lookup_key in use');
        return Response.json(createPrice(form));
      }
      if (path === '/v1/prices' && method === 'GET') {
        const keys = url.searchParams.getAll('lookup_keys[]').concat(url.searchParams.getAll('lookup_keys[0]'), url.searchParams.getAll('lookup_keys[1]'));
        const data = [...state.prices.values()].filter(price => price.active && (keys.length === 0 || keys.includes(price.lookup_key)));
        return Response.json(list(data, path));
      }
      if ((match = path.match(/^\/v1\/subscriptions\/(sub_\w+)$/)) && method === 'DELETE') {
        const sub = state.subscriptions.get(match[1]!);
        if (!sub) return error(404, 'no such subscription');
        Object.assign(sub, { status: 'canceled', canceled_at: now() });
        await emit('customer.subscription.deleted', sub);
        return Response.json(sub);
      }
      if ((match = path.match(/^\/v1\/charges\/(ch_\w+)$/)) && method === 'GET') {
        const charge = state.charges.get(match[1]!);
        return charge ? Response.json(charge) : error(404, 'no such charge');
      }
      if ((match = path.match(/^\/v1\/invoices\/(in_\w+)$/)) && method === 'GET') {
        const invoice = state.invoices.get(match[1]!) ?? [...state.events].reverse().map(event => (event as Obj).data?.object).find(object => object?.id === match![1]);
        return invoice ? Response.json(invoice) : error(404, 'no such invoice');
      }
      if (path === '/v1/invoices' && method === 'GET') {
        const subscription = url.searchParams.get('subscription'), status = url.searchParams.get('status');
        const data = [...state.invoices.values()].filter(invoice => (!subscription || invoice.parent?.subscription_details?.subscription === subscription) && (!status || invoice.status === status));
        return Response.json(list(data, path));
      }
      if (path === '/v1/invoice_payments' && method === 'GET') {
        const intent = url.searchParams.get('payment[payment_intent]'), invoiceFilter = url.searchParams.get('invoice');
        const data = [...state.invoices.values()].filter(invoice => invoice.payment_intent && (!intent || invoice.payment_intent === intent) && (!invoiceFilter || invoice.id === invoiceFilter))
          .map(invoice => ({ id: id('inpay'), object: 'invoice_payment', invoice: invoice.id, status: 'paid', is_default: true, payment: { type: 'payment_intent', payment_intent: invoice.payment_intent } }));
        return Response.json(list(data, path));
      }
      if (path === '/v1/refunds' && method === 'POST') {
        const charge = [...state.charges.values()].find(entry => entry.payment_intent === form.payment_intent || entry.id === form.charge);
        if (!charge) return error(400, 'no such payment');
        await refundCharge(charge);
        return Response.json({ id: id('re'), object: 'refund', amount: charge.amount, charge: charge.id, payment_intent: charge.payment_intent, status: 'succeeded' });
      }
      if ((match = path.match(/^\/v1\/checkout\/sessions\/(cs_\w+)$/)) && method === 'GET') {
        const session = state.sessions.get(match[1]!);
        return session ? Response.json(session) : error(404, 'no such session');
      }
      if ((match = path.match(/^\/v1\/subscriptions\/(sub_\w+)$/)) && method === 'GET') {
        const sub = state.subscriptions.get(match[1]!);
        return sub ? Response.json(sub) : error(404, 'no such subscription');
      }
      if (path === '/v1/checkout/sessions' && method === 'POST') {
        if (!state.customers.has(form.customer)) return error(400, 'no such customer');
        if (!form.line_items?.[0]?.price || !state.prices.has(form.line_items[0].price)) return error(400, 'no such price');
        const sessionId = id('cs');
        const session = {
          id: sessionId, object: 'checkout.session', mode: form.mode, customer: form.customer, client_reference_id: form.client_reference_id ?? null,
          metadata: form.metadata ?? {}, subscription_data: form.subscription_data, line_items: form.line_items, success_url: form.success_url, cancel_url: form.cancel_url,
          automatic_tax: { enabled: form.automatic_tax?.enabled === 'true' }, billing_address_collection: form.billing_address_collection ?? null,
          tax_id_collection: { enabled: form.tax_id_collection?.enabled === 'true' }, status: 'open', payment_status: 'unpaid', subscription: null,
          url: `${base}/checkout/${sessionId}`, created: now(), expires_at: now() + 86_400, livemode: false,
        };
        state.sessions.set(sessionId, session);
        return Response.json(session);
      }
      if (path === '/v1/billing_portal/sessions' && method === 'POST') {
        if (!state.customers.has(form.customer)) return error(400, 'no such customer');
        const portal = { id: id('bps'), object: 'billing_portal.session', customer: form.customer, return_url: form.return_url, url: '', created: now(), livemode: false };
        portal.url = `${base}/portal/${portal.id}`;
        state.sessions.set(portal.id, portal);
        return Response.json(portal);
      }
      return error(404, `the fake has no ${method} ${path}`);
    },
  });
  base = `http://localhost:${server.port}`;

  return {
    url: base, deliveries, state, pay, renew, failRenewal, setCancelAtPeriodEnd, switchPlan, end, refund, dispute,
    voidInvoice: (invoiceId) => closeInvoice(invoiceId, 'void'), markUncollectible: (invoiceId) => closeInvoice(invoiceId, 'uncollectible'), emit,
    redeliver: async (eventId) => { const event = state.events.find(entry => entry.id === eventId); if (!event) throw new Error('no such event'); return send(event); },
    stop: async () => { await server.stop(true); },
  };
}

if (import.meta.main) {
  const arg = (name: string) => { const i = process.argv.indexOf(name); return i > 0 ? process.argv[i + 1] : undefined; };
  const fake = startFakeStripe({ port: Number(arg('--port') ?? 12111), webhookUrl: arg('--webhook') ?? 'http://localhost:8787/v2/stripe/webhook', seedPrices: process.argv.includes('--seed-prices') });
  console.log(`Fake Stripe on ${fake.url} (webhooks to ${arg('--webhook') ?? 'http://localhost:8787/v2/stripe/webhook'})`);
  console.log(`STRIPE_API_BASE=${fake.url}`);
}
