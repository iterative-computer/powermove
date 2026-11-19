import { expect, test } from 'bun:test';
import { eq } from 'drizzle-orm';
import { createApp } from '../src/app';
import { withData } from './db';
import { makeEnv } from './env';
import { seedSession } from './helpers';
import { shares, subscriptions, usageEvents } from '../src/db/schema';
import { CONFIG } from '../src/billing/plans';
import { signStripePayload } from '../src/billing/stripe';
import type { Data } from '../src/db/client';

const json = (body: unknown, extra: Record<string, string> = {}) => ({ method: 'POST', headers: { 'Content-Type': 'application/json', ...extra }, body: JSON.stringify(body) });
const period = { start: new Date(Date.now() - 86400e3), end: new Date(Date.now() + 29 * 86400e3) };
async function subscribe(data: Data, userId: string, tier = 'aerolite') {
  await data.db.insert(subscriptions).values({ userId, tier, status: 'active', stripeCustomerId: `cus_${userId.slice(0, 6)}`, stripeSubscriptionId: `sub_${userId.slice(0, 6)}`, currentPeriodStart: period.start, currentPeriodEnd: period.end });
}
function setup(data: Data) {
  const env = makeEnv(data);
  const upstream: Array<{ url: string; body: any; headers: Headers }> = [];
  env.AEROLITE_ANTHROPIC_API_KEY = 'sk-ant-test';
  env.AEROLITE_OPENAI_API_KEY = 'sk-openai-test';
  env.AEROLITE_UPSTREAM_FETCH = (async (url: string, init: RequestInit) => {
    const body = JSON.parse(String(init.body));
    upstream.push({ url, body, headers: new Headers(init.headers) });
    if (body.stream) {
      const sse = `data: {"choices":[{"delta":{"content":"ok"}}]}\n\ndata: {"choices":[],"usage":{"prompt_tokens":1000,"completion_tokens":200}}\n\ndata: [DONE]\n\n`;
      return new Response(sse, { headers: { 'Content-Type': 'text/event-stream' } });
    }
    return Response.json({ choices: [{ message: { role: 'assistant', content: 'ok' } }], usage: { prompt_tokens: 1000, completion_tokens: 200 } });
  }) as typeof fetch;
  return { env, app: createApp({ data: () => data }), upstream };
}

test('free users get no hosted models; aerolite users are proxied and metered per provider', () => withData(async (data) => {
  const { env, app, upstream } = setup(data);
  const s = await seedSession(data, env);
  const call = (body: unknown) => app.request('/v1/ai/chat/completions', json(body, s.headers), env);

  let r = await call({ model: 'claude-sonnet-5-5', messages: [{ role: 'user', content: 'hi' }] });
  expect(r.status).toBe(403);
  expect(((await r.json()) as any).detail).toContain('Aerolite');
  r = await app.request('/v1/ai/models', { headers: s.headers }, env);
  expect(((await r.json()) as any).data).toEqual([]);

  await subscribe(data, s.id);
  r = await app.request('/v1/ai/models', { headers: s.headers }, env);
  expect(((await r.json()) as any).data.map((m: any) => m.id)).toEqual(['claude-sonnet-5-5', 'claude-haiku-5-5', 'gpt-6.2-sol', 'gpt-6-luna']);

  r = await call({ model: 'claude-opus-5-5', messages: [] });
  expect(r.status).toBe(403);

  r = await call({ model: 'claude-sonnet-5-5', max_tokens: 999_999, stream: true, user: 'spoof', messages: [{ role: 'user', content: 'hi' }] });
  expect(r.status).toBe(200);
  expect(await r.text()).toContain('[DONE]');
  await Bun.sleep(20);
  expect(upstream[0]!.url).toBe('https://api.anthropic.com/v1/chat/completions');
  expect(upstream[0]!.headers.get('x-api-key')).toBe('sk-ant-test');
  expect(upstream[0]!.body.max_tokens).toBe(16000);
  expect(upstream[0]!.body.stream_options).toEqual({ include_usage: true });
  expect(upstream[0]!.body.user).toBeUndefined();

  r = await call({ model: 'gpt-6.2-sol', messages: [{ role: 'user', content: 'hi' }] });
  expect(r.status).toBe(200);
  // A client that stops reading early is still metered (from the proxy's own copy of the stream).
  r = await call({ model: 'gpt-6-luna', stream: true, messages: [{ role: 'user', content: 'hi' }] });
  await r.body!.cancel();
  await Bun.sleep(50);
  expect(upstream[1]!.url).toBe('https://api.openai.com/v1/chat/completions');
  expect(upstream[1]!.body.user).toBe(s.id);

  const events = await data.db.select().from(usageEvents).where(eq(usageEvents.userId, s.id));
  expect(events.map(e => [e.provider, e.inputTokens, e.outputTokens, e.costMicros])).toEqual([
    ['anthropic', 1000, 200, 4000], ['openai', 1000, 200, 4000], ['openai', 1000, 200, 200],
  ]);

  // Exhaust Claude's monthly allowance: Claude is refused, OpenAI still works.
  await data.db.insert(usageEvents).values({ userId: s.id, provider: 'anthropic', model: 'claude-sonnet-5-5', costMicros: CONFIG.aerolite.anthropic.period, createdAt: new Date(period.start.getTime() + 1000) });
  r = await call({ model: 'claude-haiku-5-5', messages: [] });
  expect(r.status).toBe(429);
  expect(((await r.json()) as any).error).toBe('quota_exceeded');
  expect(r.headers.get('X-Powermove-Limit-Resets')).toBe(period.end.toISOString());
  r = await call({ model: 'gpt-6-luna', messages: [] });
  expect(r.status).toBe(200);

  r = await app.request('/v1/billing/plan', { headers: s.headers }, env);
  const plan = await r.json() as any;
  expect(plan.plan.tier).toBe('aerolite');
  expect(plan.usage.anthropic.period).toBe(1);
  expect(plan.usage.openai.period).toBeGreaterThan(0);
  expect(plan.usage.openai.period).toBeLessThan(0.01);
  expect(plan.storage.limitBytes).toBe(1024 ** 3);
}));

test('daily cap refuses after a heavy day', () => withData(async (data) => {
  const { env, app } = setup(data);
  const s = await seedSession(data, env);
  await subscribe(data, s.id);
  await data.db.insert(usageEvents).values({ userId: s.id, provider: 'openai', model: 'gpt-6.2-sol', costMicros: CONFIG.aerolite.openai.day });
  const r = await app.request('/v1/ai/chat/completions', json({ model: 'gpt-6.2-sol', messages: [] }, s.headers), env);
  expect(r.status).toBe(429);
  expect(((await r.json()) as any).detail).toContain('today');
}));

const snapshot = { kind: 'project', title: 'Launch <film>', width: 1920, height: 1080, fps: 30, durationSeconds: 15, layers: [{ name: 'Logo', type: 'shape', inSeconds: 0, outSeconds: 15 }] };
const PNG = btoa(String.fromCharCode(137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0));

test('share links: free gets one ever, revoking does not refund, page is static and escaped', () => withData(async (data) => {
  const { env, app } = setup(data);
  const s = await seedSession(data, env);
  let r = await app.request('/v1/shares', json({ snapshot, poster: PNG }, s.headers), env);
  expect(r.status).toBe(201);
  const { id, url, interactive } = await r.json() as any;
  expect(url).toBe(`https://cloud.trypowermove.com/s/${id}`);
  expect(interactive).toBe(false);

  r = await app.request(`/s/${id}`, {}, env);
  expect(r.status).toBe(200);
  const html = await r.text();
  expect(html).toContain('Launch &lt;film&gt;');
  expect(html).not.toContain('<film>');
  expect(html).toContain('Read-only view');
  expect(html).not.toContain('<script');
  expect(r.headers.get('Content-Security-Policy')).toContain("default-src 'none'");
  r = await app.request(`/s/${id}/poster`, {}, env);
  expect(r.headers.get('Content-Type')).toBe('image/png');

  r = await app.request('/v1/shares', json({ snapshot }, s.headers), env);
  expect(r.status).toBe(429);
  expect(((await r.json()) as any).detail).toContain('one share link');

  r = await app.request(`/v1/shares/${id}`, { method: 'DELETE', headers: s.headers }, env);
  expect(r.status).toBe(200);
  expect((await app.request(`/s/${id}`, {}, env)).status).toBe(404);
  r = await app.request('/v1/shares', json({ snapshot }, s.headers), env);
  expect(r.status).toBe(429);
}));

test('share links: aerolite gets one per rolling week, aero is unlimited', () => withData(async (data) => {
  const { env, app } = setup(data);
  const a = await seedSession(data, env, 'a@example.com');
  await subscribe(data, a.id);
  expect((await app.request('/v1/shares', json({ snapshot }, a.headers), env)).status).toBe(201);
  let r = await app.request('/v1/shares', json({ snapshot }, a.headers), env);
  expect(r.status).toBe(429);
  let plan = await (await app.request('/v1/billing/plan', { headers: a.headers }, env)).json() as any;
  expect(plan.shares.canCreate).toBe(false);
  expect(plan.shares.nextAvailableAt).not.toBeNull();
  // Eight days later the week has rolled over.
  await data.db.update(shares).set({ createdAt: new Date(Date.now() - 8 * 86400e3) }).where(eq(shares.userId, a.id));
  expect((await app.request('/v1/shares', json({ snapshot }, a.headers), env)).status).toBe(201);

  const b = await seedSession(data, env, 'b@example.com');
  await subscribe(data, b.id, 'aero');
  for (let i = 0; i < 4; i++) {
    r = await app.request('/v1/shares', json({ snapshot }, b.headers), env);
    expect(r.status).toBe(201);
    expect(((await r.json()) as any).interactive).toBe(true);
  }
  plan = await (await app.request('/v1/billing/plan', { headers: b.headers }, env)).json() as any;
  expect(plan.plan.tier).toBe('aero');
  expect(plan.shares.canCreate).toBe(true);
}));

test('share creation respects the storage hook', () => withData(async (data) => {
  const { env, app } = setup(data);
  const s = await seedSession(data, env);
  await subscribe(data, s.id);
  await data.db.insert(shares).values({ id: 'big', userId: s.id, kind: 'project', title: 'x', snapshot: {}, sizeBytes: 2 ** 31 - 1, createdAt: new Date(Date.now() - 30 * 86400e3) });
  const r = await app.request('/v1/shares', json({ snapshot }, s.headers), env);
  expect(r.status).toBe(429);
  expect(((await r.json()) as any).detail).toContain('storage');
}));

test('checkout refuses live keys; webhook verifies, upserts and is idempotent', () => withData(async (data) => {
  const { env, app } = setup(data);
  const s = await seedSession(data, env);
  env.STRIPE_PRICE_AEROLITE = 'price_aerolite';
  env.STRIPE_WEBHOOK_SECRET = 'whsec_test_secret';
  env.STRIPE_SECRET_KEY = 'sk_live_nope';
  const stripeCalls: Array<{ url: string; body: string }> = [];
  env.STRIPE_FETCH = (async (url: string, init: RequestInit) => {
    stripeCalls.push({ url, body: String(init.body ?? '') });
    if (url.includes('/checkout/sessions')) return Response.json({ id: 'cs_1', url: 'https://checkout.stripe.com/c/pay/cs_test_1' });
    if (url.includes('/subscriptions/sub_1')) return Response.json({ id: 'sub_1', customer: 'cus_1', status: 'active', cancel_at_period_end: false, metadata: { user_id: s.id, tier: 'aerolite' }, items: { data: [{ price: { id: 'price_aerolite' }, current_period_start: Math.floor(period.start.getTime() / 1000), current_period_end: Math.floor(period.end.getTime() / 1000) }] } });
    return Response.json({ error: { message: 'unexpected' } }, { status: 400 });
  }) as typeof fetch;

  let r = await app.request('/v1/billing/checkout', json({ tier: 'aerolite' }, s.headers), env);
  expect(r.status).toBe(500);
  expect(stripeCalls).toHaveLength(0);
  env.STRIPE_SECRET_KEY = 'sk_test_ok';
  r = await app.request('/v1/billing/checkout', json({ tier: 'aero' }, s.headers), env);
  expect(r.status).toBe(400);
  r = await app.request('/v1/billing/checkout', json({ tier: 'aerolite' }, s.headers), env);
  expect(r.status).toBe(200);
  expect(((await r.json()) as any).url).toContain('checkout.stripe.com');
  const form = new URLSearchParams(stripeCalls[0]!.body);
  expect(form.get('mode')).toBe('subscription');
  expect(form.get('line_items[0][price]')).toBe('price_aerolite');
  expect(form.get('client_reference_id')).toBe(s.id);

  const event = JSON.stringify({ id: 'evt_1', type: 'checkout.session.completed', livemode: false, data: { object: { subscription: 'sub_1', client_reference_id: s.id } } });
  r = await app.request('/v1/billing/webhook', { method: 'POST', body: event, headers: { 'stripe-signature': 't=1,v1=bad' } }, env);
  expect(r.status).toBe(401);
  const sig = await signStripePayload(event, 'whsec_test_secret');
  r = await app.request('/v1/billing/webhook', { method: 'POST', body: event, headers: { 'stripe-signature': sig } }, env);
  expect(r.status).toBe(200);
  const [row] = await data.db.select().from(subscriptions).where(eq(subscriptions.userId, s.id));
  expect(row).toMatchObject({ tier: 'aerolite', status: 'active', stripeCustomerId: 'cus_1', stripeSubscriptionId: 'sub_1' });
  r = await app.request('/v1/billing/webhook', { method: 'POST', body: event, headers: { 'stripe-signature': sig } }, env);
  expect(((await r.json()) as any).duplicate).toBe(true);

  const plan = await (await app.request('/v1/billing/plan', { headers: s.headers }, env)).json() as any;
  expect(plan.plan.tier).toBe('aerolite');
  expect(plan.subscription.renewsAt).toBe(new Date(Math.floor(period.end.getTime() / 1000) * 1000).toISOString());
  expect(plan.upgrades).toEqual([]);

  const live = JSON.stringify({ id: 'evt_live', type: 'checkout.session.completed', livemode: true, data: { object: {} } });
  r = await app.request('/v1/billing/webhook', { method: 'POST', body: live, headers: { 'stripe-signature': await signStripePayload(live, 'whsec_test_secret') } }, env);
  expect(((await r.json()) as any).ignored).toBe('livemode');
}));
