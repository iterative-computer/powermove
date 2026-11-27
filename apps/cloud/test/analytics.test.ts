/* Aero & Billing analytics: the admin endpoints (auth, aggregates on seeded ledger rows, unblock) and the PostHog
   server events (off unless configured, never blocking, never content). */
import { expect, test } from 'bun:test';
import { eq } from 'drizzle-orm';
import { withData } from './db';
import { makeEnv } from './env';
import { seedAdmin, seedSession } from './helpers';
import { frames, fxReduce, stack, subscribe } from './billing-helpers';
import { createApp } from '../src/app';
import { aeroBlocks, billingReversals, creditLedger, moderationLog, subscriptions } from '../src/db/schema';
import { emitter, telemetryOn } from '../src/telemetry';
import type { Data } from '../src/db/client';

const DAY = 86_400_000;

test('the Aero & Billing endpoints are admin only (RL_ADMIN and the admins row)', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    const someone = await seedSession(data, env, 'someone@example.com');
    const admin = await seedAdmin(data, env);
    expect((await app.request('/v1/admin/aero/overview', {}, env)).status).toBe(401);
    expect((await app.request('/v1/admin/aero/overview', { headers: someone.headers }, env)).status).toBe(403);
    expect((await app.request(`/v1/admin/aero/blocks/${someone.id}`, { method: 'DELETE', headers: someone.headers }, env)).status).toBe(403);
    expect((await app.request('/v1/admin/aero/overview', { headers: admin.headers }, env)).status).toBe(200);
    const limited = { ...env, RL_ADMIN: { limit: async () => ({ success: false }) } as RateLimit };
    expect((await app.request('/v1/admin/aero/overview', { headers: admin.headers }, limited)).status).toBe(429);
  }));

/** The current billing period every seeded row shares. */
const PERIOD = { periodStart: new Date(Date.now() - 20 * DAY), periodEnd: new Date(Date.now() + 10 * DAY) };
/** One request row, `daysAgo` days back from now. */
const row = (userId: string, key: string, daysAgo: number, fields: Partial<typeof creditLedger.$inferInsert>) => ({
  userId, kind: 'spend' as const, idempotencyKey: `request:${userId}:${key}`, credits: 0, model: 'openai/gpt-6.2-sol',
  ...PERIOD, createdAt: new Date(Date.now() - daysAgo * DAY), ...fields,
});

test('the overview adds up: usage today and over 30 days, per model and per day, subscriptions, top accounts, estimated, blocks, ceiling, reversals', async () =>
  withData(async (data) => {
    const env = { ...makeEnv(data), AERO_GLOBAL_DAILY_USD: '50' }, app = createApp({ data: () => data });
    const admin = await seedAdmin(data, env);
    const a = await seedSession(data, env, 'a@example.com');
    const b = await seedSession(data, env, 'b@example.com');
    const period = PERIOD;
    // a: Aerolite, active; b: Aerolite, cancelling; plus one past due and one canceled.
    const sub = (id: string, userId: string, status: string, cancelAtPeriodEnd = false, createdDaysAgo = 3) => ({ id, userId, stripeCustomerId: `cus_${id}`, stripePriceId: 'price_a', plan: 'aerolite', status, currentPeriodStart: period.periodStart, currentPeriodEnd: period.periodEnd, cancelAtPeriodEnd, stripeUpdatedAt: new Date(), createdAt: new Date(Date.now() - createdDaysAgo * DAY) });
    await data.db.insert(subscriptions).values([sub('sub_a', a.id, 'active'), sub('sub_b', b.id, 'active', true, 45), sub('sub_c', admin.id, 'past_due'), sub('sub_d', admin.id, 'canceled')]);
    await data.db.insert(creditLedger).values({ userId: a.id, kind: 'grant', idempotencyKey: 'invoice:in_a', credits: 1000, ...period });
    await data.db.insert(creditLedger).values([
      // Today: a reported Sol call ($0.30) and an estimated Opus call billed at its hold ($1.20).
      row(a.id, 'today-1', 0, { costNanoUsd: 300_000_000, inputTokens: 1000, cachedInputTokens: 600, cacheWriteTokens: 100, outputTokens: 50, usageSource: 'reported' }),
      row(a.id, 'today-2', 0, { model: 'anthropic/claude-opus-5-5', costNanoUsd: 1_200_000_000, holdNanoUsd: 1_200_000_000, estimated: true, usageSource: 'hold' }),
      // Five days ago: b, Sol, $2.
      row(b.id, 'old-1', 5, { costNanoUsd: 2_000_000_000, inputTokens: 4000, cachedInputTokens: 0, outputTokens: 400, usageSource: 'reported' }),
      // An expired, never-settled hold (counts at its $0.50 hold) and a live one (not spend yet).
      row(b.id, 'expired', 2, { kind: 'hold', holdNanoUsd: 500_000_000, holdExpiresAt: new Date(Date.now() - DAY) }),
      row(a.id, 'live', 0, { kind: 'hold', holdNanoUsd: 700_000_000, holdExpiresAt: new Date(Date.now() + DAY) }),
      // Forty days ago: outside the window.
      row(a.id, 'ancient', 40, { costNanoUsd: 9_000_000_000, inputTokens: 99_999, periodStart: new Date(Date.now() - 50 * DAY), periodEnd: new Date(Date.now() - 20 * DAY) }),
    ]);
    await data.db.insert(aeroBlocks).values({ userId: b.id, reason: 'spend over hold', key: 'request:b:x' });
    await data.db.insert(billingReversals).values({ invoiceId: 'in_refunded', userId: a.id, periodStart: period.periodStart, reason: 'charge.refunded', eventId: 'evt_1' });

    const r = await app.request('/v1/admin/aero/overview', { headers: admin.headers }, env);
    expect(r.status).toBe(200);
    const o = await r.json() as any;
    // Today: $0.30 + $1.20 = $1.50 = 300 credits; 2 calls, 1 estimated; live hold excluded.
    expect(o.today).toMatchObject({ calls: 3, estimatedCalls: 1, inputTokens: 1000, cachedTokens: 600, cacheWriteTokens: 100, outputTokens: 50, cacheHitRatio: 0.6 });
    expect(o.today.costUsd).toBeCloseTo(1.5, 9);
    // $1.50 at the fixture's fake unit ($0.001 a credit).
    expect(o.today.credits).toBeCloseTo(1500, 9);
    expect(o.today.estimatedUsd).toBeCloseTo(1.2, 9);
    // 30 days: + $2 + the expired $0.50 = $4.00; the 40-day-old row is out.
    expect(o.last30.costUsd).toBeCloseTo(4, 9);
    expect(o.last30.calls).toBe(5);
    expect(o.last30.inputTokens).toBe(5000);
    expect(o.last30.cacheHitRatio).toBeCloseTo(600 / 5000, 9);
    const sol = o.models.find((m: any) => m.model === 'openai/gpt-6.2-sol');
    expect(sol.costUsd).toBeCloseTo(2.8, 9);
    expect(o.models.find((m: any) => m.model === 'anthropic/claude-opus-5-5').costUsd).toBeCloseTo(1.2, 9);
    expect(o.daily).toHaveLength(30);
    expect(o.daily.at(-1).date).toBe(new Date().toISOString().slice(0, 10));
    expect(o.daily.at(-1).costUsd).toBeCloseTo(1.5, 9);
    expect(o.daily.at(-6).costUsd).toBeCloseTo(2, 9);
    expect(o.daily.reduce((sum: number, d: any) => sum + d.costUsd, 0)).toBeCloseTo(4, 9);
    expect(o.subscriptions).toEqual({ active: 2, mrrCents: 1998, new30: 3, cancelling: 1, pastDue: 1 });
    // Top accounts: b ($2.50) then a ($1.50 plus a $0.70 live hold against a $1 grant at the fixture's unit: none left).
    expect(o.topAccounts.map((t: any) => t.user.email)).toEqual(['b@example.com', 'a@example.com']);
    expect(o.topAccounts[1].creditsLeft).toBe(0);
    expect(o.estimated).toHaveLength(1);
    expect(o.estimated[0]).toMatchObject({ model: 'anthropic/claude-opus-5-5', source: 'hold' });
    expect(o.blocked).toEqual([expect.objectContaining({ user: { id: b.id, email: 'b@example.com' }, reason: 'spend over hold' })]);
    // Today vs the ceiling: spends and holds created today, live holds included ($0.30 + $1.20 + $0.70).
    expect(o.ceiling.todayUsd).toBeCloseTo(2.2, 9);
    expect(o.ceiling.ceilingUsd).toBe(50);
    expect(o.reversals).toEqual([expect.objectContaining({ invoiceId: 'in_refunded', reason: 'charge.refunded', user: { id: a.id, email: 'a@example.com' } })]);
  }));

test('unblocking an account is an admin write, audit-logged', async () =>
  withData(async (data) => {
    const env = makeEnv(data), app = createApp({ data: () => data });
    const admin = await seedAdmin(data, env);
    const blocked = await seedSession(data, env, 'blocked@example.com');
    await data.db.insert(aeroBlocks).values({ userId: blocked.id, reason: 'spend over hold' });
    expect((await app.request(`/v1/admin/aero/blocks/${blocked.id}`, { method: 'DELETE', headers: admin.headers }, env)).status).toBe(204);
    expect(await data.db.select().from(aeroBlocks)).toHaveLength(0);
    const [entry] = await data.db.select().from(moderationLog).where(eq(moderationLog.userId, blocked.id));
    expect(entry).toMatchObject({ action: 'unblock_aero', actor: admin.id });
    expect((await app.request(`/v1/admin/aero/blocks/${blocked.id}`, { method: 'DELETE', headers: admin.headers }, env)).status).toBe(404);
  }));

test('PostHog is off unless configured (and off outside production unless POSTHOG_LOCAL), and emitting never blocks or throws', async () => {
  const base = { ENVIRONMENT: 'production', POSTHOG_HOST: 'https://us.i.posthog.com', POSTHOG_KEY: 'phc_test' } as CloudflareBindings;
  expect(telemetryOn(base)).toBe(true);
  for (const env of [{ ...base, POSTHOG_KEY: undefined }, { ...base, POSTHOG_HOST: undefined }, { ...base, ENVIRONMENT: 'development' }]) expect(telemetryOn(env as CloudflareBindings)).toBe(false);
  expect(telemetryOn({ ...base, ENVIRONMENT: 'development', POSTHOG_LOCAL: '1' })).toBe(true);
  let calls = 0;
  emitter({ ...base, POSTHOG_KEY: undefined }, undefined, (async () => { calls++; return new Response(); }) as unknown as typeof fetch)('x', 'u');
  expect(calls).toBe(0);
  // A capture that never answers, and one that throws: emit returns at once, and nothing escapes.
  const hang = (() => new Promise<Response>(() => {})) as unknown as typeof fetch;
  const boom = (async () => { throw new Error('network down'); }) as unknown as typeof fetch;
  const started = performance.now();
  emitter(base, undefined, hang)('x', 'u');
  emitter(base, () => { throw new Error('no context'); }, boom)('x', 'u');
  expect(performance.now() - started).toBeLessThan(50);
  await new Promise(resolve => setTimeout(resolve, 20));
});

/** A local stand-in for PostHog's capture endpoint. `hang` makes it never answer. */
function capture(hang = false) {
  const events: any[] = [];
  const server = Bun.serve({ port: 0, async fetch(request) { events.push(await request.json()); if (hang) await new Promise(() => {}); return Response.json({ status: 1 }); } });
  return { url: `http://localhost:${server.port}`, events, stop: () => server.stop(true) };
}

for (const hang of [false, true]) {
  test(`server events: subscription_started, aero_call_settled (usage only, no content), credits_reversed${hang ? ', with PostHog not answering' : ''}`, async () =>
    withData(async (data: Data) => {
      const posthog = capture(hang);
      const s = await stack(data);
      try {
        Object.assign(s.env, { POSTHOG_HOST: posthog.url, POSTHOG_KEY: 'phc_test', POSTHOG_LOCAL: '1' });
        const { subscriptionId } = await subscribe(s);
        const secret = 'the secret prompt about my unreleased film';
        const started = performance.now();
        const r = await s.request('/v2/aero/chat/completions', { json: { model: 'openai/gpt-6-luna', stream: true, stream_options: { include_usage: true }, messages: [{ role: 'user', content: secret }] } });
        fxReduce(await frames(r));
        expect(performance.now() - started).toBeLessThan(3000);
        await s.stripe.refund([...s.stripe.state.invoices.values()].find(i => i.parent.subscription_details.subscription === subscriptionId)!.id);
        for (let i = 0; i < 100 && !posthog.events.some(e => e.event === 'credits_reversed'); i++) await new Promise(resolve => setTimeout(resolve, 20));
        const names = posthog.events.map(e => e.event);
        expect(names).toContain('subscription_started');
        expect(names).toContain('aero_call_settled');
        expect(names).toContain('credits_reversed');
        const settled = posthog.events.find(e => e.event === 'aero_call_settled');
        expect(settled.distinct_id).toBe(s.user.id);
        expect(settled.api_key).toBe('phc_test');
        expect(settled.properties).toMatchObject({ model: 'openai/gpt-6-luna', provider: 'openai', usage_source: 'reported', estimated: false });
        for (const key of ['input_tokens', 'cache_read_tokens', 'cache_write_tokens', 'output_tokens', 'cost_usd', 'credits', 'cache_hit_ratio', 'duration_ms']) expect(typeof settled.properties[key]).toBe('number');
        // Never any content.
        expect(JSON.stringify(posthog.events)).not.toContain('secret prompt');
        expect(JSON.stringify(posthog.events)).not.toContain('You said');
      } finally { await s.stop(); posthog.stop(); }
    }));
}
