/* The call outlives everything around it: billing periods that end mid-flight, objects evicted mid-stream, a
   database that fails at settle time; and the smallest request a balance can really cover gets through. */
import { expect, test } from 'bun:test';
import { eq, sql } from 'drizzle-orm';
import { withData } from './db';
import { billing, frames, fxReduce, periodOf, stack, subscribe, type Stack } from './billing-helpers';
import { creditLedger, subscriptions } from '../src/db/schema';
import { modelById } from '../src/aero/catalogue';
import { CREDIT_NANO_USD, cost } from './pricing';
import { MIN_OUTPUT } from '../src/aero/meter';
import type { CallStart } from '../src/aero/call';
import type { Data } from '../src/db/client';

const chat = (s: Stack, body: Record<string, unknown>, id: string) =>
  s.request('/v2/aero/chat/completions', { json: { stream: true, stream_options: { include_usage: true }, ...body } }, { ...s.user.headers, 'X-Powermove-Request-Id': id });
const keyOf = (s: Stack, id: string) => `request:${s.user.id}:${id}`;
const rowFor = async (data: Data, s: Stack, id: string) => (await data.db.select().from(creditLedger).where(eq(creditLedger.idempotencyKey, keyOf(s, id))))[0];
/** Reads a stream until `marker` shows up, then hangs up. */
async function readUntil(r: Response, marker: string) {
  const reader = r.body!.getReader();
  const decoder = new TextDecoder();
  let seen = '';
  while (!seen.includes(marker)) {
    const { value, done } = await reader.read();
    if (done) throw new Error(`stream ended before ${marker}`);
    seen += decoder.decode(value);
  }
  await reader.cancel();
}
const until = async (check: () => Promise<boolean>) => { for (let i = 0; i < 300 && !await check(); i++) await new Promise(resolve => setTimeout(resolve, 20)); };

/** The call objects' connection, its `update` failing while `failures` lasts (a database outage at settle time). */
function flaky(data: Data) {
  const state = { failures: 0 };
  const db = new Proxy(data.db, {
    get(target, prop) {
      if (prop === 'update' && state.failures > 0) { state.failures--; return () => { throw new Error('database unavailable'); }; }
      const value = Reflect.get(target, prop, target);
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
  return { state, callData: () => ({ ...data, db, end: async () => {} }) };
}

test('a period that has ended admits nothing until its renewal arrives; a call counts in the period it was admitted in', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const { subscriptionId } = await subscribe(s);
      const first = await periodOf(data, s.user.id);
      // The period ends before the renewal webhook comes: the Worker refuses, and so does the call itself (which
      // decides the period under the account lock).
      await data.db.update(subscriptions).set({ currentPeriodEnd: new Date(Date.now() - 1000) }).where(eq(subscriptions.id, subscriptionId));
      let r = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] }, 'req-expired-0001');
      expect(r.status).toBe(402);
      const start: CallStart = { request: { model: 'openai/gpt-6-luna', stream: true, messages: [{ role: 'user', content: 'x' }] }, userId: s.user.id, requestId: 'req-expired-0002', key: keyOf(s, 'req-expired-0002'), cacheKey: 'pm-test' };
      const direct = await s.calls.get(s.calls.idFromName(start.key)).fetch('https://aero-call/start', { method: 'POST', body: JSON.stringify(start) });
      expect(direct.status).toBe(402);
      expect(await direct.json() as unknown).toEqual({ error: { code: 'credits_exhausted' } });
      expect(s.ai.requests).toHaveLength(0);
      await data.db.update(subscriptions).set({ currentPeriodEnd: first.periodEnd }).where(eq(subscriptions.id, subscriptionId));

      // Admitted in the first period, still streaming when the renewal lands: it counts in the first period.
      r = await chat(s, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: 'Write a lot [slow]' }] }, 'req-straddle-0001');
      expect(r.status).toBe(200);
      await new Promise(resolve => setTimeout(resolve, 1100));
      await s.stripe.renew(subscriptionId);
      fxReduce(await frames(r));
      const row = (await rowFor(data, s, 'req-straddle-0001'))!;
      expect(row).toMatchObject({ kind: 'spend', usageSource: 'reported' });
      expect(row.periodStart.getTime()).toBe(first.periodStart.getTime());
      expect(row.periodEnd.getTime()).toBe(first.periodEnd.getTime());
      expect((await billing(s)).credits).toEqual({ included: 1000, used: 0, remaining: 1000 });
    } finally { await s.stop(); }
  }));

test('an object evicted mid-stream, before any usage: its alarm bills the whole hold, marked estimated, never nothing', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const r = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'Write a lot [slow]' }] }, 'req-evicted-0001');
      expect(r.status).toBe(200);
      await readUntil(r, 'word10');
      s.calls.evict(keyOf(s, 'req-evicted-0001'));
      // The dead instance stops; its row is still a hold (counted in full), and its alarm is set.
      await new Promise(resolve => setTimeout(resolve, 100));
      const held = (await rowFor(data, s, 'req-evicted-0001'))!;
      expect(held.kind).toBe('hold');
      expect(s.calls.alarms.has(keyOf(s, 'req-evicted-0001'))).toBe(true);
      expect(await s.calls.runAlarms()).toBe(1);
      expect(await rowFor(data, s, 'req-evicted-0001')).toMatchObject({ kind: 'spend', usageSource: 'hold', estimated: true, costNanoUsd: held.holdNanoUsd });
      // Settled once: the object's storage is empty and no alarm is left.
      expect(s.calls.stored.get(keyOf(s, 'req-evicted-0001'))!.size).toBe(0);
      expect(await s.calls.runAlarms()).toBe(0);
    } finally { await s.stop(); }
  }));

test('a reported cost above the hold is billed at the hold: a spend never passes what was reserved', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // A tiny prompt that the provider says took 1M tokens of input ($4 at the long tier): far over its hold.
      s.ai.queue.push({ usage: { input_tokens: 1_000_000, input_tokens_details: { cached_tokens: 0 }, output_tokens: 0, total_tokens: 1_000_000 } });
      fxReduce(await frames(await chat(s, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: 'x' }] }, 'req-over-0001')));
      const row = (await rowFor(data, s, 'req-over-0001'))!;
      expect(row).toMatchObject({ kind: 'spend', usageSource: 'reported', inputTokens: 1_000_000 });
      expect(row.holdNanoUsd).toBeLessThan(4_000_000_000);
      expect(row.costNanoUsd).toBe(row.holdNanoUsd);
    } finally { await s.stop(); }
  }));

test('the hard cap: under parallel requests, failures, evictions and lost settles, a period never spends past its grant', async () =>
  withData(async (data) => {
    const { state, callData } = flaky(data);
    const s = await stack(data, 'jude@example.com', { callData });
    try {
      await subscribe(s);
      const period = await periodOf(data, s.user.id);
      // $0.08 left of the $5 grant.
      await data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'spend', idempotencyKey: keyOf(s, 'spent'), credits: 0, costNanoUsd: 1000 * CREDIT_NANO_USD - 80_000_000, model: 'openai/gpt-6-luna', ...period });
      // Every kind of reply: an error, a cut stream, a failed response, a refusal, a 5xx, normal replies; and the
      // settle writes fail for a while.
      s.ai.queue.push(
        { fail: 'error' }, { fail: 'cut' }, { fail: 'http' }, { fail: 'http500' }, {}, { fail: 'failed' },
        { fail: 'cut' }, {}, {},
      );
      // The database is down for the whole burst: every in-process settle fails, and alarms finish them.
      state.failures = Number.POSITIVE_INFINITY;
      const ids = Array.from({ length: 10 }, (_, i) => `req-cap-${String(i).padStart(4, '0')}`);
      const models = ['openai/gpt-6.2-sol', 'anthropic/claude-sonnet-5-5', 'openai/gpt-6-luna'];
      const answers = await Promise.all(ids.map((id, i) => chat(s, { model: models[i % 3], max_completion_tokens: 4_000, messages: [{ role: 'user', content: `Request ${i}` }] }, id)));
      // Refused: by the balance (402), or by the account's three-calls-in-flight cap (429).
      expect(answers.some(r => r.status === 402 || r.status === 429)).toBe(true);
      expect(answers.every(r => [200, 402, 429].includes(r.status))).toBe(true);
      for (const r of answers) await r.text();
      await new Promise(resolve => setTimeout(resolve, 2000));
      const deferred = s.calls.alarms.size;
      expect(deferred).toBeGreaterThan(0);
      const pending = (await data.db.select().from(creditLedger).where(eq(creditLedger.userId, s.user.id))).filter(row => row.kind === 'hold');
      expect(pending.length).toBe(deferred);
      // Some objects die before their alarms; then the database is back and every alarm runs.
      for (const id of ids.slice(0, 3)) s.calls.evict(keyOf(s, id));
      state.failures = 0;
      while (await s.calls.runAlarms() > 0) { /* until every call has settled */ }
      const rows = await data.db.select().from(creditLedger).where(eq(creditLedger.userId, s.user.id));
      const requests = rows.filter(row => row.kind !== 'grant');
      // What the provider billed: a reported call its usage at list price (which the hold covers); any other call at
      // most its hold, which is what it was charged.
      const calls = requests.filter(row => row.idempotencyKey !== keyOf(s, 'spent'));
      let upstream = 1000 * CREDIT_NANO_USD - 80_000_000;
      for (const row of calls) {
        expect(row.kind).toBe('spend');
        const billed = row.usageSource === 'reported'
          ? cost(modelById(row.model!)!, { input: row.inputTokens, cacheRead: row.cachedInputTokens, cacheWrite: row.cacheWriteTokens, cacheWriteLong: row.cacheWriteLongTokens, output: row.outputTokens }).nanoUsd
          : row.holdNanoUsd;
        expect(billed).toBeLessThanOrEqual(row.holdNanoUsd);
        if (row.usageSource === 'reported') expect(row.costNanoUsd).toBe(billed);
        upstream += billed;
      }
      expect(upstream).toBeLessThanOrEqual(1000 * CREDIT_NANO_USD);
      const settled = requests.filter(row => row.kind === 'spend').reduce((sum, row) => sum + row.costNanoUsd, 0);
      const holds = requests.filter(row => row.kind === 'hold').reduce((sum, row) => sum + row.holdNanoUsd, 0);
      expect(settled + holds).toBeLessThanOrEqual(1000 * CREDIT_NANO_USD);
      expect((await billing(s)).credits.remaining).toBeGreaterThanOrEqual(0);
    } finally { await s.stop(); }
  }), 20_000);

test('the settle write is retried: a transient failure still lands the exact spend, and a long outage is finished by the alarm', async () =>
  withData(async (data) => {
    const { state, callData } = flaky(data);
    const s = await stack(data, 'jude@example.com', { callData });
    try {
      await subscribe(s);
      const script = { start: { input_tokens: 40, cache_creation_input_tokens: 0, cache_read_input_tokens: 1_000, output_tokens: 1 }, delta: { output_tokens: 90 } };
      const bill = 40 * 4_000 + 1_000 * 200 + 90 * 20_000;
      // Two failed writes, then it lands in process.
      state.failures = 2;
      s.ai.queue.push(script);
      fxReduce(await frames(await chat(s, { model: 'anthropic/claude-opus-5-5', messages: [{ role: 'user', content: 'x' }] }, 'req-flaky-0001')));
      expect(await rowFor(data, s, 'req-flaky-0001')).toMatchObject({ kind: 'spend', usageSource: 'reported', costNanoUsd: bill });
      expect(state.failures).toBe(0);
      // Every in-process attempt fails: the reported usage waits in the object's storage, and its alarm writes it.
      state.failures = 4;
      s.ai.queue.push(script);
      const r = await chat(s, { model: 'anthropic/claude-opus-5-5', messages: [{ role: 'user', content: 'x' }] }, 'req-flaky-0002');
      await r.text();
      await until(async () => s.calls.alarms.has(keyOf(s, 'req-flaky-0002')) && state.failures === 0);
      expect((await rowFor(data, s, 'req-flaky-0002'))!.kind).toBe('hold');
      // The object is gone too by the time the alarm fires: a fresh instance settles from storage.
      s.calls.evict(keyOf(s, 'req-flaky-0002'));
      expect(await s.calls.runAlarms()).toBe(1);
      expect(await rowFor(data, s, 'req-flaky-0002')).toMatchObject({ kind: 'spend', usageSource: 'reported', estimated: false, costNanoUsd: bill, outputTokens: 90 });
    } finally { await s.stop(); }
  }));

test('the smallest request a balance can cover gets through: provider-specific overhead and a 16-token minimum output', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const period = await periodOf(data, s.user.id);
      // $0.00015 left. A short Luna request costs far less than that; the old 4,096-token padding refused it.
      await data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'spend', idempotencyKey: keyOf(s, 'spent'), credits: 0, costNanoUsd: 1000 * CREDIT_NANO_USD - 150_000, model: 'openai/gpt-6-luna', ...period });
      const r = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] }, 'req-small-0001');
      expect(r.status).toBe(200);
      await r.text();
      const row = (await rowFor(data, s, 'req-small-0001'))!;
      expect(row.outputCap!).toBeGreaterThanOrEqual(MIN_OUTPUT);
      expect(row.holdNanoUsd).toBeLessThanOrEqual(150_000);
      expect(s.ai.requests[0]!.body.max_output_tokens).toBe(row.outputCap);
      const [{ total }] = await data.db.select({ total: sql<string>`sum(cost_nano_usd)` }).from(creditLedger).where(eq(creditLedger.userId, s.user.id)) as [{ total: string }];
      expect(Number(total)).toBeLessThanOrEqual(1000 * CREDIT_NANO_USD);
    } finally { await s.stop(); }
  }));
