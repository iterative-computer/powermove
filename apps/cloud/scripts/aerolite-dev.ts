/*
 * Aerolite without any accounts: the real cloud app on an in-memory PGlite
 * database, with simulated Stripe and simulated model providers. For trying
 * the desktop flow end to end (`bun scripts/aerolite-dev.ts`, then run the
 * desktop with POWERMOVE_REGISTRY_URL=http://localhost:8787).
 *
 * Nothing here talks to Stripe, Anthropic or OpenAI. Sign-in codes are kept in
 * memory and readable at /__dev/otp?email=… . POST /__dev/complete-checkout
 * finishes the last checkout by sending a correctly signed webhook to the app.
 * Set AEROLITE_ANTHROPIC_API_KEY / AEROLITE_OPENAI_API_KEY to reach the real
 * providers instead of the simulated ones.
 */
import { createApp } from '../src/app';
import { signStripePayload } from '../src/billing/stripe';
import { pgliteData } from '../test/db';
import { FakeEmail } from '../test/env';
import { FakeR2 } from '../test/r2';
import { desc } from 'drizzle-orm';
import { shares } from '../src/db/schema';

const port = Number(process.env.PORT ?? 8787);
const origin = `http://localhost:${port}`;
const data = await pgliteData();
const otps = new Map<string, string>();
const checkout: { userId?: string; customer?: string } = {};
const WEBHOOK_SECRET = 'whsec_local_dev_only';
const PRICE = 'price_local_aerolite';
const allow = { limit: async () => ({ success: true }) } as unknown as RateLimit;
const now = () => Math.floor(Date.now() / 1000);

const simulatedModel = (async (url: RequestInfo | URL, init?: RequestInit) => {
  const body = JSON.parse(String(init?.body ?? '{}'));
  const provider = String(url).includes('anthropic') ? 'Anthropic' : 'OpenAI';
  const text = `Hi! This reply came through Powermove Aerolite's metered proxy as ${body.model} (simulated ${provider} upstream on this Mac). Your agent is connected; ask me to animate something.`;
  const promptTokens = Math.ceil(JSON.stringify(body.messages ?? []).length / 4);
  const chunks = text.match(/.{1,24}/g) ?? [text];
  const events = chunks.map(part => `data: ${JSON.stringify({ choices: [{ index: 0, delta: { content: part } }] })}\n\n`);
  events.push(`data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }] })}\n\n`);
  events.push(`data: ${JSON.stringify({ choices: [], usage: { prompt_tokens: promptTokens, completion_tokens: Math.ceil(text.length / 4) } })}\n\ndata: [DONE]\n\n`);
  if (!body.stream) return Response.json({ choices: [{ message: { role: 'assistant', content: text }, finish_reason: 'stop' }], usage: { prompt_tokens: promptTokens, completion_tokens: 40 } });
  return new Response(new ReadableStream({ async start(c) { for (const e of events) { c.enqueue(new TextEncoder().encode(e)); await Bun.sleep(25); } c.close(); } }), { headers: { 'Content-Type': 'text/event-stream' } });
}) as typeof fetch;

const simulatedStripe = (async (url: RequestInfo | URL, init?: RequestInit) => {
  const u = new URL(String(url));
  const form = new URLSearchParams(String(init?.body ?? ''));
  if (u.pathname === '/v1/checkout/sessions') {
    checkout.userId = form.get('client_reference_id') ?? undefined;
    checkout.customer = form.get('customer') ?? `cus_dev_${checkout.userId?.slice(0, 8)}`;
    return Response.json({ id: 'cs_test_dev', url: 'https://checkout.stripe.com/c/pay/cs_test_dev' });
  }
  if (u.pathname === '/v1/billing_portal/sessions') return Response.json({ url: 'https://billing.stripe.com/p/session/test_dev' });
  if (u.pathname.startsWith('/v1/subscriptions/')) {
    return Response.json({ id: 'sub_dev', customer: checkout.customer, status: 'active', cancel_at_period_end: false, metadata: { user_id: checkout.userId, tier: 'aerolite' },
      items: { data: [{ price: { id: PRICE }, current_period_start: now() - 60, current_period_end: now() + 30 * 86400 }] } });
  }
  return Response.json({ error: { message: `unhandled ${u.pathname}` } }, { status: 400 });
}) as typeof fetch;

const env = {
  DATABASE_URL: 'pglite://memory', BETTER_AUTH_SECRET: 'local-development-secret-32-characters-long',
  APP_ORIGIN: origin, ADMIN_ORIGIN: 'http://localhost:5174', HANDLE_MAIL_DOMAIN: 'users.trypowermove.com', EMAIL_FROM: 'Powermove <sign-in@trypowermove.com>',
  EMAIL: new FakeEmail(), OBJECTS: new FakeR2(), TARS: new FakeR2(), ICONS: new FakeR2(), SHARES: new FakeR2(),
  RL_UPLOAD: allow, RL_AUTH: allow, RL_ADMIN: allow,
  OTP_SENDER: async (email: string, otp: string) => { otps.set(email, otp); console.log(`OTP for ${email}: ${otp}`); },
  AEROLITE_ANTHROPIC_API_KEY: process.env.AEROLITE_ANTHROPIC_API_KEY ?? 'simulated', AEROLITE_OPENAI_API_KEY: process.env.AEROLITE_OPENAI_API_KEY ?? 'simulated',
  ...(process.env.AEROLITE_ANTHROPIC_API_KEY || process.env.AEROLITE_OPENAI_API_KEY ? {} : { AEROLITE_UPSTREAM_FETCH: simulatedModel }),
  STRIPE_SECRET_KEY: 'sk_test_simulated', STRIPE_WEBHOOK_SECRET: WEBHOOK_SECRET, STRIPE_PRICE_AEROLITE: PRICE, STRIPE_FETCH: simulatedStripe,
} as unknown as CloudflareBindings;

const app = createApp({ data: () => data });
Bun.serve({
  port,
  idleTimeout: 120,
  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === '/__dev/otp') return Response.json({ otp: otps.get(url.searchParams.get('email') ?? '') ?? null });
    if (url.pathname === '/__dev/last-share') {
      const [row] = await data.db.select({ id: shares.id }).from(shares).orderBy(desc(shares.createdAt)).limit(1);
      return Response.json({ url: row ? `${origin}/s/${row.id}` : null });
    }
    if (url.pathname === '/__dev/complete-checkout' && request.method === 'POST') {
      const payload = JSON.stringify({ id: `evt_${crypto.randomUUID()}`, type: 'checkout.session.completed', livemode: false, data: { object: { subscription: 'sub_dev', client_reference_id: checkout.userId } } });
      return app.request('/v1/billing/webhook', { method: 'POST', body: payload, headers: { 'stripe-signature': await signStripePayload(payload, WEBHOOK_SECRET) } }, env);
    }
    return app.fetch(request, env, { waitUntil() {}, passThroughOnException() {} } as unknown as ExecutionContext);
  },
});
console.log(`Aerolite dev cloud on ${origin} (simulated Stripe${env.AEROLITE_UPSTREAM_FETCH ? ' and models' : ''})`);
