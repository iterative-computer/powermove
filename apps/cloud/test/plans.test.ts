import { describe, expect, test } from 'bun:test';
import { CONFIG, costMicros, effectiveTier, entitlementsFor, MODELS, PLANS, PURCHASABLE, resolveModel } from '../src/billing/plans';
import { checkModelBudget, checkShareQuota, checkStorage, usageDay, usagePeriod, WEEK } from '../src/billing/quotas';
import { estimateUsage, meteredStream, usageFromJson } from '../src/billing/meter';
import { signStripePayload, verifyStripeSignature } from '../src/billing/stripe';

const now = new Date('2026-11-18T15:00:00Z');
const windows = { period: usagePeriod(now), day: usageDay(now) };

describe('entitlements', () => {
  test('tiers carry the spec numbers', () => {
    expect(PLANS.aerolite.priceCents).toBe(900);
    expect(PLANS.aero.priceCents).toBe(1900);
    expect(PLANS.aerolite.storageBytes).toBe(CONFIG.aeroliteStorageBytes);
    expect(PLANS.aerolite.storageBytes).toBe(1024 ** 3);
    expect(PLANS.aero.storageBytes).toBe(50 * 1024 ** 3);
    expect(PLANS.free.models).toBeNull();
    expect(PLANS.aerolite.interactiveShares).toBe(false);
    expect(PLANS.aero.interactiveShares).toBe(true);
    expect(PURCHASABLE).toEqual(['aerolite']);
  });
  test('Aero is a superset of Aerolite', () => {
    for (const p of ['anthropic', 'openai'] as const) {
      expect(PLANS.aero.models![p].periodMicros).toBeGreaterThan(PLANS.aerolite.models![p].periodMicros);
      for (const id of PLANS.aerolite.models![p].models) expect(PLANS.aero.models![p].models).toContain(id);
    }
    expect(PLANS.aero.storageBytes).toBeGreaterThan(PLANS.aerolite.storageBytes);
  });
  test('every model in a plan is priced and belongs to its provider', () => {
    for (const plan of Object.values(PLANS)) for (const [provider, a] of Object.entries(plan.models ?? {}))
      for (const id of a.models) expect(MODELS[id]?.provider).toBe(provider as 'anthropic' | 'openai');
  });
  test('worst-case Aerolite inference stays inside the $9 price after Stripe fees', () => {
    const a = PLANS.aerolite.models!;
    const inference = (a.anthropic.periodMicros + a.openai.periodMicros) / 1e6;
    const stripeFee = 9 * 0.029 + 0.3;
    const storage = 0.015; // R2, 1 GB-month
    expect(9 - stripeFee - storage - inference).toBeGreaterThan(4);
  });
  test('effective tier follows subscription status and period', () => {
    expect(effectiveTier(null, now)).toBe('free');
    const end = new Date(now.getTime() + 86400e3);
    expect(effectiveTier({ tier: 'aerolite', status: 'active', currentPeriodEnd: end }, now)).toBe('aerolite');
    expect(effectiveTier({ tier: 'aerolite', status: 'past_due', currentPeriodEnd: end }, now)).toBe('aerolite');
    expect(effectiveTier({ tier: 'aerolite', status: 'canceled', currentPeriodEnd: end }, now)).toBe('free');
    expect(effectiveTier({ tier: 'aerolite', status: 'incomplete', currentPeriodEnd: end }, now)).toBe('free');
    expect(effectiveTier({ tier: 'platinum', status: 'active', currentPeriodEnd: end }, now)).toBe('free');
    expect(effectiveTier({ tier: 'aerolite', status: 'active', currentPeriodEnd: new Date(now.getTime() - 3 * 86400e3) }, now)).toBe('free');
  });
  test('model resolution', () => {
    expect(resolveModel(PLANS.free, 'anthropic', 'claude-sonnet-5-5')).toEqual({ ok: false, reason: 'no_hosted_models' });
    expect(resolveModel(PLANS.aerolite, 'anthropic', 'claude-sonnet-5-5').ok).toBe(true);
    expect(resolveModel(PLANS.aerolite, 'anthropic', 'claude-opus-5-5')).toEqual({ ok: false, reason: 'model_not_included' });
    expect(resolveModel(PLANS.aero, 'anthropic', 'claude-opus-5-5').ok).toBe(true);
    expect(resolveModel(PLANS.aerolite, 'openai', 'claude-sonnet-5-5')).toEqual({ ok: false, reason: 'model_not_included' });
    expect(resolveModel(PLANS.aerolite, 'openai', 42)).toEqual({ ok: false, reason: 'model_not_included' });
  });
});

describe('model usage', () => {
  test('cost is list price in micro-dollars, rounded up', () => {
    const sonnet = MODELS['claude-sonnet-5-5']!;
    expect(costMicros(sonnet, { input: 1_000_000, output: 0 })).toBe(2_000_000);
    expect(costMicros(sonnet, { input: 0, output: 1000 })).toBe(10_000);
    expect(costMicros(sonnet, { input: 0, output: 0, cacheRead: 1000 })).toBe(100);
    expect(costMicros(MODELS['gpt-6-luna']!, { input: 1, output: 0 })).toBe(1);
  });
  test('budget: separate per provider, monthly and daily', () => {
    const ent = entitlementsFor('aerolite');
    expect(checkModelBudget(ent, 'anthropic', { periodMicros: 0, dayMicros: 0 }, windows).ok).toBe(true);
    const month = checkModelBudget(ent, 'anthropic', { periodMicros: CONFIG.aerolite.anthropic.period, dayMicros: 0 }, windows);
    expect(month).toEqual({ ok: false, reason: 'period_limit', resetsAt: new Date('2026-12-01T00:00:00Z') });
    const day = checkModelBudget(ent, 'openai', { periodMicros: 10, dayMicros: CONFIG.aerolite.openai.day }, windows);
    expect(day).toEqual({ ok: false, reason: 'day_limit', resetsAt: new Date('2026-11-19T00:00:00Z') });
    // Claude being exhausted does not touch the OpenAI allowance.
    expect(checkModelBudget(ent, 'openai', { periodMicros: 0, dayMicros: 0 }, windows).ok).toBe(true);
    expect(checkModelBudget(entitlementsFor('free'), 'openai', { periodMicros: 0, dayMicros: 0 }, windows)).toEqual({ ok: false, reason: 'no_hosted_models', resetsAt: null });
  });
  test('usage period follows Stripe billing when present', () => {
    const start = new Date('2026-11-10T08:00:00Z'), end = new Date('2026-12-10T08:00:00Z');
    expect(usagePeriod(now, { start, end })).toEqual({ start, end });
    expect(usagePeriod(new Date('2026-12-11T00:00:00Z'), { start, end }).start).toEqual(new Date('2026-12-01T00:00:00Z'));
  });
  test('usage parsing subtracts cached tokens from prompt tokens', () => {
    expect(usageFromJson({ prompt_tokens: 1000, completion_tokens: 50, prompt_tokens_details: { cached_tokens: 800 } })).toEqual({ input: 200, output: 50, cacheRead: 800, cacheWrite: 0 });
    expect(usageFromJson(undefined)).toBeNull();
    expect(estimateUsage(300, 30)).toEqual({ input: 100, output: 10 });
  });
  test('metered stream passes bytes through and finds the usage chunk', async () => {
    const sse = 'data: {"choices":[{"delta":{"content":"hi"}}]}\n\ndata: {"choices":[],"usage":{"prompt_tokens":12,"completion_tokens":3}}\n\ndata: [DONE]\n\n';
    let seen: unknown = undefined;
    const bytes = new TextEncoder().encode(sse);
    const source = new ReadableStream({ start(c) { c.enqueue(bytes.slice(0, 40)); c.enqueue(bytes.slice(40, 77)); c.enqueue(bytes.slice(77)); c.close(); } });
    const out = await new Response(source.pipeThrough(meteredStream(async (usage) => { seen = usage; }))).text();
    expect(out).toBe(sse);
    expect(seen).toEqual({ input: 12, output: 3, cacheRead: 0, cacheWrite: 0 });
  });
});

describe('share quotas', () => {
  const empty = { total: 0, inLastWeek: 0, oldestInLastWeek: null, latestCreatedAt: null };
  test('free: one link, ever', () => {
    expect(checkShareQuota(PLANS.free.shares, empty, now)).toEqual({ ok: true, remaining: 1 });
    expect(checkShareQuota(PLANS.free.shares, { ...empty, total: 1 }, now)).toEqual({ ok: false, reason: 'lifetime_used', nextAvailableAt: null });
  });
  test('aerolite: one per rolling week', () => {
    const oldest = new Date(now.getTime() - 2 * 86400e3);
    expect(checkShareQuota(PLANS.aerolite.shares, { ...empty, total: 9 }, now)).toEqual({ ok: true, remaining: 1 });
    expect(checkShareQuota(PLANS.aerolite.shares, { ...empty, total: 10, inLastWeek: 1, oldestInLastWeek: oldest }, now))
      .toEqual({ ok: false, reason: 'weekly_used', nextAvailableAt: new Date(oldest.getTime() + WEEK) });
  });
  test('aero: unlimited', () => {
    expect(checkShareQuota(PLANS.aero.shares, { ...empty, total: 1e6, inLastWeek: 1e4, oldestInLastWeek: now }, now)).toEqual({ ok: true, remaining: null });
  });
});

describe('storage hook', () => {
  test('defaults to 1 GB on Aerolite', () => {
    const gb = 1024 ** 3;
    expect(checkStorage(PLANS.aerolite, gb - 10, 10)).toEqual({ ok: true, remainingBytes: 0 });
    expect(checkStorage(PLANS.aerolite, gb - 10, 11)).toEqual({ ok: false, reason: 'storage_full', limitBytes: gb, usedBytes: gb - 10 });
    expect(checkStorage(PLANS.free, 0, 3 * 1024 ** 2).ok).toBe(true);
  });
});

describe('stripe signatures', () => {
  test('accepts a fresh valid signature and rejects tampering or replays', async () => {
    const secret = 'whsec_test', body = '{"id":"evt_1"}';
    const t = Math.floor(Date.now() / 1000);
    const header = await signStripePayload(body, secret, t);
    expect(await verifyStripeSignature(body, header, secret)).toBe(true);
    expect(await verifyStripeSignature(body + ' ', header, secret)).toBe(false);
    expect(await verifyStripeSignature(body, header, 'whsec_other')).toBe(false);
    expect(await verifyStripeSignature(body, header, secret, (t + 3600) * 1000)).toBe(false);
    expect(await verifyStripeSignature(body, undefined, secret)).toBe(false);
  });
});
