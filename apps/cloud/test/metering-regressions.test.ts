/* One test per finding of the second metering review (each failed on 7f2c9d9d6). Under the owner's policy any doubt
   bills the whole hold, and what the provider bills can never pass the period's grant. */
import { expect, test } from 'bun:test';
import { and, eq, ne } from 'drizzle-orm';
import { withData } from './db';
import { frames, fxReduce, periodOf, stack, subscribe, type Stack } from './billing-helpers';
import { creditLedger, subscriptions } from '../src/db/schema';
import { modelById } from '../src/aero/catalogue';
import { CREDIT_NANO_USD, cost } from './pricing';
import { MAX_DRAIN_MS } from '../src/aero/meter';
import { sweepHolds } from '../src/billing/ledger';
import type { Data } from '../src/db/client';
import type { CallStart } from '../src/aero/call';

const chat = (s: Stack, body: Record<string, unknown>, id: string) =>
  s.request('/v2/aero/chat/completions', { json: { stream: true, stream_options: { include_usage: true }, ...body } }, { ...s.user.headers, 'X-Powermove-Request-Id': id });
const keyOf = (s: Stack, id: string) => `request:${s.user.id}:${id}`;
const rowFor = async (data: Data, s: Stack, id: string) => (await data.db.select().from(creditLedger).where(eq(creditLedger.idempotencyKey, keyOf(s, id))))[0];
const until = async (check: () => Promise<boolean>, ms = 6000) => { for (let i = 0; i < ms / 20 && !await check(); i++) await new Promise(resolve => setTimeout(resolve, 20)); };
/** Leaves exactly `nano` of the period's grant. */
async function leave(data: Data, s: Stack, nano: number) {
  await data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'spend', idempotencyKey: keyOf(s, `spent-${nano}`), credits: 0, costNanoUsd: 1000 * CREDIT_NANO_USD - nano, model: 'openai/gpt-6-luna', ...await periodOf(data, s.user.id) });
}
/** What the provider billed for a settled row: its reported tokens at the catalogue rates (not what the ledger charged). */
const upstream = (row: typeof creditLedger.$inferSelect) => cost(modelById(row.model!)!, { input: row.inputTokens, cacheRead: row.cachedInputTokens, cacheWrite: row.cacheWriteTokens, cacheWriteLong: row.cacheWriteLongTokens, output: row.outputTokens }).nanoUsd;

test('2. admission never trusts a low token count: what the provider bills stays within the balance', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // A provider count that under-reports a hundredfold, and a 40 KB prompt the provider really bills ~10k tokens for.
      s.ai.countScale = 0.01;
      const prompt = 'Describe the timeline in detail. '.repeat(1_250);
      await leave(data, s, 30_000_000); // $0.03
      let r = await chat(s, { model: 'anthropic/claude-opus-5-5', messages: [{ role: 'user', content: prompt }] }, 'req-bound-0001');
      // The byte bound alone puts this request past $0.03: refused, and the provider never billed anything.
      expect(r.status).toBe(402);
      expect(s.ai.requests).toHaveLength(0);
      // With room for the byte bound, it runs, and the provider's bill fits inside the hold.
      await data.db.delete(creditLedger).where(and(eq(creditLedger.userId, s.user.id), ne(creditLedger.kind, 'grant')));
      await leave(data, s, 400_000_000);
      r = await chat(s, { model: 'anthropic/claude-opus-5-5', max_completion_tokens: 2_000, messages: [{ role: 'user', content: prompt }] }, 'req-bound-0002');
      expect(r.status).toBe(200);
      fxReduce(await frames(r));
      const row = (await rowFor(data, s, 'req-bound-0002'))!;
      expect(row.usageSource).toBe('reported');
      expect(upstream(row)).toBe(row.costNanoUsd);
      expect(upstream(row)).toBeLessThanOrEqual(row.holdNanoUsd);
      expect(upstream(row)).toBeLessThanOrEqual(400_000_000);
    } finally { await s.stop(); }
  }));

test('3. only a final usage event is complete: response.failed with usage and an Anthropic stream cut after its delta bill the hold', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      s.ai.queue.push({ fail: 'failed' });
      let r = await chat(s, { model: 'openai/gpt-6-luna', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'x' }] }, 'req-terminal-0001');
      expect((await frames(r)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
      let row = (await rowFor(data, s, 'req-terminal-0001'))!;
      expect(row).toMatchObject({ kind: 'spend', usageSource: 'hold', estimated: true });
      expect(row.costNanoUsd).toBe(row.holdNanoUsd);
      s.ai.queue.push({ start: { input_tokens: 10, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 1 }, delta: { output_tokens: 5 }, fail: 'cut-after-delta' });
      r = await chat(s, { model: 'anthropic/claude-sonnet-5-5', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'x' }] }, 'req-terminal-0002');
      expect((await frames(r)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
      row = (await rowFor(data, s, 'req-terminal-0002'))!;
      expect(row).toMatchObject({ kind: 'spend', usageSource: 'hold', estimated: true });
      expect(row.costNanoUsd).toBe(row.holdNanoUsd);
    } finally { await s.stop(); }
  }));

test('4. a concurrent duplicate start gets 409 and leaves the original owning its drain: the alarm waits, the call settles exactly', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // Two starts of one call reach its object at the same moment.
      const start: CallStart = { request: { model: 'openai/gpt-6-luna', stream: true, max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'Write a lot [slow]' }] }, userId: s.user.id, requestId: 'req-dup-0001', key: keyOf(s, 'req-dup-0001'), cacheKey: 'pm-test' };
      const object = s.calls.get(s.calls.idFromName(start.key));
      const answers = await Promise.all([1, 2].map(() => object.fetch('https://aero-call/start', { method: 'POST', body: JSON.stringify(start) })));
      expect(answers.map(r => r.status).sort()).toEqual([200, 409]);
      const original = answers.find(r => r.status === 200)!;
      // The loser has finished (its 409 sent, its own task wound down); the alarm comes while the original still
      // streams: it must wait, not settle.
      await new Promise(resolve => setTimeout(resolve, 200));
      expect(await s.calls.runAlarms()).toBe(1);
      expect((await rowFor(data, s, 'req-dup-0001'))!.kind).toBe('hold');
      fxReduce(await frames(original));
      await until(async () => (await rowFor(data, s, 'req-dup-0001'))!.kind === 'spend');
      expect(await rowFor(data, s, 'req-dup-0001')).toMatchObject({ kind: 'spend', usageSource: 'reported', estimated: false });
      expect(s.ai.requests).toHaveLength(1);
    } finally { await s.stop(); }
  }));

/** The call objects' connection, with every select inside a transaction delayed by `ms` before it runs (a slow
    database between taking the account lock and reading the subscription). */
function slowSelects(data: Data, ms: number) {
  const delayed = (builder: any): any => new Proxy(builder, {
    get(target, prop) {
      const value = Reflect.get(target, prop, target);
      if (prop === 'then') return (resolve: any, reject: any) => new Promise(r => setTimeout(r, ms)).then(() => target.then(resolve, reject));
      return typeof value === 'function' ? (...args: unknown[]) => { const out = value.apply(target, args); return out && typeof out === 'object' ? delayed(out) : out; } : value;
    },
  });
  const wrap = (tx: any) => new Proxy(tx, { get(target, prop) { const value = Reflect.get(target, prop, target); return prop === 'select' ? (...args: unknown[]) => delayed(value.apply(target, args)) : typeof value === 'function' ? value.bind(target) : value; } });
  return () => ({ ...data, tx: <T>(fn: (tx: any) => Promise<T>) => data.tx(tx => fn(wrap(tx))), end: async () => {} });
}

test('5. the period is decided by the database clock when the subscription is read, not by a time taken before', async () =>
  withData(async (data) => {
    const s = await stack(data, 'jude@example.com', { callData: slowSelects(data, 400) });
    try {
      const { subscriptionId } = await subscribe(s);
      // The period ends 200 ms from now: live when the Worker checks, ended by the time the admission (under the
      // account lock, its reads slowed down) gets the subscription back.
      await new Promise(resolve => setTimeout(resolve, 50));
      await data.db.update(subscriptions).set({ currentPeriodEnd: new Date(Date.now() + 200) }).where(eq(subscriptions.id, subscriptionId));
      const r = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] }, 'req-clock-0001');
      expect(r.status).toBe(402);
      expect(s.ai.requests).toHaveLength(0);
    } finally { await s.stop(); }
  }));

test('6. no stranded holds: an object killed between its hold and its call record is billed by its alarm, and the sweep bills expired holds', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // The object dies writing its call record, right after the hold committed.
      s.calls.dieOnPut(keyOf(s, 'req-strand-0001'), 'call');
      const r = await chat(s, { model: 'openai/gpt-6-luna', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'x' }] }, 'req-strand-0001');
      await r.text().catch(() => {});
      const held = (await rowFor(data, s, 'req-strand-0001'))!;
      expect(held.kind).toBe('hold');
      expect(s.ai.requests).toHaveLength(0);
      // Its alarm was armed before the hold: a fresh instance finds the intent and bills the whole hold.
      s.calls.evict(keyOf(s, 'req-strand-0001'));
      expect(await s.calls.runAlarms()).toBe(1);
      expect(await rowFor(data, s, 'req-strand-0001')).toMatchObject({ kind: 'spend', usageSource: 'hold', estimated: true, costNanoUsd: held.holdNanoUsd });
      // The sweep: a hold past its expiry becomes a spend of the whole hold; a live one is left alone.
      const period = await periodOf(data, s.user.id);
      const hold = (id: string, expires: Date) => data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'hold', idempotencyKey: keyOf(s, id), credits: 0, model: 'openai/gpt-6-luna', holdNanoUsd: 7_000_000, holdExpiresAt: expires, ...period });
      await hold('expired', new Date(Date.now() - 1000));
      await hold('live', new Date(Date.now() + 60_000));
      expect(await sweepHolds(data.db)).toBe(1);
      expect(await rowFor(data, s, 'expired')).toMatchObject({ kind: 'spend', usageSource: 'hold', estimated: true, costNanoUsd: 7_000_000 });
      expect((await rowFor(data, s, 'live'))!.kind).toBe('hold');
    } finally { await s.stop(); }
  }));

test('7. a client that never reads does not stall the drain, and the watchdog stops a drain that runs too long', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // Nobody reads this response: the call still drains and settles from its reported usage.
      const stalled = await chat(s, { model: 'openai/gpt-6-luna', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'Write a lot [slow]' }] }, 'req-stall-0001');
      expect(stalled.status).toBe(200);
      await until(async () => (await rowFor(data, s, 'req-stall-0001'))?.kind === 'spend');
      expect(await rowFor(data, s, 'req-stall-0001')).toMatchObject({ kind: 'spend', usageSource: 'reported' });
      // A drain past MAX_DRAIN_MS: the watchdog alarm stops it, and the call settles at its whole hold.
      const r = await chat(s, { model: 'openai/gpt-6-luna', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'Write a lot [slow]' }] }, 'req-watchdog-0001');
      expect(r.status).toBe(200);
      const stored = s.calls.stored.get(keyOf(s, 'req-watchdog-0001'))!;
      stored.set('call', { ...stored.get('call') as object, dispatchedAt: Date.now() - MAX_DRAIN_MS - 1 });
      await s.calls.runAlarms();
      await until(async () => (await rowFor(data, s, 'req-watchdog-0001'))?.kind === 'spend');
      const row = (await rowFor(data, s, 'req-watchdog-0001'))!;
      expect(row).toMatchObject({ kind: 'spend', usageSource: 'hold', estimated: true });
      expect(row.costNanoUsd).toBe(row.holdNanoUsd);
      await r.text();
    } finally { await s.stop(); }
  }));
