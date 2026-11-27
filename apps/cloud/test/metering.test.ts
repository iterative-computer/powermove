/* Aero metering against hand-computed provider bills. Each case drives the AI simulator with the provider's own raw
   usage JSON, through the real AI SDK providers and their usage converters, the route and the ledger. Prices are
   nano-USD per token (USD per million x 1000): Opus 4,000 in / 200 cache read / 5,000 cache write / 8,000 1h write /
   20,000 out; Sonnet 2,000 / 100 / 2,500 / 4,000 / 10,000; Sol 2,000 / 100 / 2,500 / 5,000 (12h) / 10,000, over
   272k input 4,000 / 200 / 5,000 / 10,000 / 15,000; Luna 100 / 10 / 125 / 250 / 500. */
import { expect, test } from 'bun:test';
import { and, eq, ne } from 'drizzle-orm';
import { withData } from './db';
import { billing, frames, fxReduce, stack, subscribe, type Stack } from './billing-helpers';
import { creditLedger } from '../src/db/schema';
import { CREDIT_NANO_USD, modelById } from '../src/aero/catalogue';
import { HOLD_TTL_MS, imageBound, inputBound } from '../src/aero/meter';
import { displayCredits } from '../src/billing/ledger';
import type { Data } from '../src/db/client';
import type { Script } from '../scripts/ai-simulator';

const SYSTEM = { role: 'system', content: 'You are Aero, the agent in Powermove.' };
const TOOLS = [{ type: 'function', function: { name: 'read_file', description: 'Read a file', parameters: { type: 'object', properties: { path: { type: 'string' } } } } }];
let ids = 0;
const chat = (s: Stack, body: Record<string, unknown>, id = `req-metering-${++ids}`) =>
  s.request('/v2/aero/chat/completions', { json: { stream: true, stream_options: { include_usage: true }, ...body } }, { ...s.user.headers, 'X-Powermove-Request-Id': id });
const rows = (data: Data) => data.db.select().from(creditLedger).where(ne(creditLedger.kind, 'grant'));
const rowFor = async (data: Data, s: Stack, id: string) => (await data.db.select().from(creditLedger).where(eq(creditLedger.idempotencyKey, `request:${s.user.id}:${id}`)))[0]!;
/** Sets the user's period balance to exactly `nano` remaining, with one spend. */
async function leave(data: Data, s: Stack, nano: number) {
  await data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'spend', idempotencyKey: `request:${s.user.id}:spent-${++ids}`, credits: 0, costNanoUsd: 1000 * CREDIT_NANO_USD - nano, model: 'openai/gpt-6-luna' });
}

interface Case {
  name: string;
  model: string;
  script: Script;
  /** The hand-computed bill, nano-USD. */
  bill: number;
  tokens: { input: number; fresh: number; cacheRead: number; cacheWrite: number; cacheWriteLong?: number; output: number; reasoning?: number };
  source?: 'reported' | 'partial';
}
const CASES: Case[] = [
  { name: 'Anthropic cold: the whole prompt written to the cache', model: 'anthropic/claude-opus-5-5',
    script: { start: { input_tokens: 12, cache_creation_input_tokens: 48_000, cache_read_input_tokens: 0, output_tokens: 1 }, delta: { output_tokens: 850 } },
    bill: 12 * 4_000 + 48_000 * 5_000 + 850 * 20_000, tokens: { input: 48_012, fresh: 12, cacheRead: 0, cacheWrite: 48_000, output: 850 } },
  { name: 'Anthropic warm: the prefix read, the new turn written', model: 'anthropic/claude-opus-5-5',
    script: { start: { input_tokens: 9, cache_creation_input_tokens: 1_200, cache_read_input_tokens: 48_000, output_tokens: 1 }, delta: { output_tokens: 400 } },
    bill: 9 * 4_000 + 1_200 * 5_000 + 48_000 * 200 + 400 * 20_000, tokens: { input: 49_209, fresh: 9, cacheRead: 48_000, cacheWrite: 1_200, output: 400 } },
  { name: 'Anthropic partially cached, nothing written', model: 'anthropic/claude-sonnet-5-5',
    script: { start: { input_tokens: 20_000, cache_creation_input_tokens: 0, cache_read_input_tokens: 30_000, output_tokens: 1 }, delta: { output_tokens: 120 } },
    bill: 20_000 * 2_000 + 30_000 * 100 + 120 * 10_000, tokens: { input: 50_000, fresh: 20_000, cacheRead: 30_000, cacheWrite: 0, output: 120 } },
  { name: 'Anthropic reasoning-heavy: thinking is inside output, priced once', model: 'anthropic/claude-opus-5-5',
    script: { start: { input_tokens: 3_000, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 1 }, delta: { output_tokens: 60_000, output_tokens_details: { thinking_tokens: 58_000 } } },
    bill: 3_000 * 4_000 + 60_000 * 20_000, tokens: { input: 3_000, fresh: 3_000, cacheRead: 0, cacheWrite: 0, output: 60_000, reasoning: 58_000 } },
  { name: 'Anthropic 1-hour cache writes are priced at 2x input', model: 'anthropic/claude-opus-5-5',
    script: { start: { input_tokens: 10, cache_creation_input_tokens: 2_000, cache_read_input_tokens: 0, cache_creation: { ephemeral_5m_input_tokens: 1_000, ephemeral_1h_input_tokens: 1_000 }, output_tokens: 1 }, delta: { output_tokens: 10 } },
    bill: 10 * 4_000 + 1_000 * 5_000 + 1_000 * 8_000 + 10 * 20_000, tokens: { input: 2_010, fresh: 10, cacheRead: 0, cacheWrite: 2_000, cacheWriteLong: 1_000, output: 10 } },
  { name: 'OpenAI cold', model: 'openai/gpt-6.2-sol',
    script: { usage: { input_tokens: 40_000, input_tokens_details: { cached_tokens: 0 }, output_tokens: 900, output_tokens_details: { reasoning_tokens: 600 }, total_tokens: 40_900 } },
    bill: 40_000 * 2_000 + 900 * 10_000, tokens: { input: 40_000, fresh: 40_000, cacheRead: 0, cacheWrite: 0, output: 900, reasoning: 600 } },
  { name: 'OpenAI warm, with a 30-minute cache write', model: 'openai/gpt-6.2-sol',
    script: { usage: { input_tokens: 41_000, input_tokens_details: { cached_tokens: 39_936, cache_write_tokens: 1_064, cache_write_tokens_details: { ttl_30m_tokens: 1_064, ttl_12h_tokens: 0 } }, output_tokens: 500, output_tokens_details: { reasoning_tokens: 0 }, total_tokens: 41_500 } },
    bill: 39_936 * 100 + 1_064 * 2_500 + 500 * 10_000, tokens: { input: 41_000, fresh: 0, cacheRead: 39_936, cacheWrite: 1_064, output: 500 } },
  { name: 'OpenAI long context: over 272k input (cached included), the whole request at the tier', model: 'openai/gpt-6.2-sol',
    script: { usage: { input_tokens: 300_000, input_tokens_details: { cached_tokens: 250_000 }, output_tokens: 2_000, output_tokens_details: { reasoning_tokens: 1_500 }, total_tokens: 302_000 } },
    bill: 50_000 * 4_000 + 250_000 * 200 + 2_000 * 15_000, tokens: { input: 300_000, fresh: 50_000, cacheRead: 250_000, cacheWrite: 0, output: 2_000, reasoning: 1_500 } },
  { name: 'OpenAI 12-hour cache writes are priced at 2.5x input', model: 'openai/gpt-6.2-sol',
    script: { usage: { input_tokens: 10_000, input_tokens_details: { cached_tokens: 0, cache_write_tokens: 4_000, cache_write_tokens_details: { ttl_30m_tokens: 1_000, ttl_12h_tokens: 3_000 } }, output_tokens: 0, total_tokens: 10_000 } },
    bill: 6_000 * 2_000 + 1_000 * 2_500 + 3_000 * 5_000, tokens: { input: 10_000, fresh: 6_000, cacheRead: 0, cacheWrite: 4_000, cacheWriteLong: 3_000, output: 0 } },
  { name: 'OpenAI tiny request: fractions of a micro-USD are kept, not rounded', model: 'openai/gpt-6-luna',
    script: { usage: { input_tokens: 7, input_tokens_details: { cached_tokens: 3 }, output_tokens: 1, output_tokens_details: { reasoning_tokens: 0 }, total_tokens: 8 } },
    bill: 4 * 100 + 3 * 10 + 1 * 500, tokens: { input: 7, fresh: 4, cacheRead: 3, cacheWrite: 0, output: 1 } },
];

for (const c of CASES) {
  test(`bill: ${c.name}`, async () =>
    withData(async (data) => {
      const s = await stack(data);
      try {
        await subscribe(s);
        s.ai.queue.push(c.script);
        const r = await chat(s, { model: c.model, messages: [SYSTEM, { role: 'user', content: 'Hello' }] }, 'req-case-0001');
        expect(r.status).toBe(200);
        fxReduce(await frames(r));
        const row = await rowFor(data, s, 'req-case-0001');
        expect(row).toMatchObject({
          kind: 'spend', usageSource: c.source ?? 'reported', estimated: false, model: c.model,
          inputTokens: c.tokens.input, freshInputTokens: c.tokens.fresh, cachedInputTokens: c.tokens.cacheRead, cacheWriteTokens: c.tokens.cacheWrite,
          cacheWriteLongTokens: c.tokens.cacheWriteLong ?? 0, outputTokens: c.tokens.output, reasoningTokens: c.tokens.reasoning ?? 0,
        });
        expect(row.costNanoUsd).toBe(c.bill);
        expect(row.priceVersion).toBeTruthy();
        const used = Math.ceil(c.bill / CREDIT_NANO_USD);
        expect((await billing(s)).credits).toEqual({ included: 1000, used, remaining: 1000 - used });
      } finally { await s.stop(); }
    }));
}

test('a model switch mid-thread: the new model has no cache, so the whole context is fresh input and a cache write', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const thread = [SYSTEM, { role: 'user', content: 'Plan the edit.' }, { role: 'assistant', content: 'Here is the plan.' }, { role: 'user', content: 'Do it.' }];
      s.ai.queue.push({ start: { input_tokens: 5, cache_creation_input_tokens: 0, cache_read_input_tokens: 40_000, output_tokens: 1 }, delta: { output_tokens: 300 } });
      fxReduce(await frames(await chat(s, { model: 'anthropic/claude-sonnet-5-5', messages: thread }, 'req-switch-0001')));
      s.ai.queue.push({ start: { input_tokens: 5, cache_creation_input_tokens: 40_000, cache_read_input_tokens: 0, output_tokens: 1 }, delta: { output_tokens: 300 } });
      fxReduce(await frames(await chat(s, { model: 'anthropic/claude-opus-5-5', messages: thread }, 'req-switch-0002')));
      const [first, second] = [await rowFor(data, s, 'req-switch-0001'), await rowFor(data, s, 'req-switch-0002')];
      expect(first.costNanoUsd).toBe(5 * 2_000 + 40_000 * 100 + 300 * 10_000);
      expect(second.costNanoUsd).toBe(5 * 4_000 + 40_000 * 5_000 + 300 * 20_000);
      expect(second).toMatchObject({ model: 'anthropic/claude-opus-5-5', cachedInputTokens: 0, cacheWriteTokens: 40_000, freshInputTokens: 5 });
      // The new model got the whole context, every turn of it.
      const [a, b] = s.ai.requests.map(r => r.body);
      expect(b!.messages.length).toBe(a!.messages.length);
      expect(b!.model).toBe('claude-opus-5-5');
    } finally { await s.stop(); }
  }));

test('a provider error mid-stream bills only what is proven; a refused call costs nothing', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // Anthropic: message_start proved the input (and its one output token).
      s.ai.queue.push({ start: { input_tokens: 100, cache_creation_input_tokens: 0, cache_read_input_tokens: 50, output_tokens: 1 }, fail: 'error' });
      let r = await chat(s, { model: 'anthropic/claude-opus-5-5', messages: [{ role: 'user', content: 'x' }] }, 'req-error-0001');
      expect((await frames(r)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
      expect(await rowFor(data, s, 'req-error-0001')).toMatchObject({ kind: 'spend', usageSource: 'partial', estimated: false, costNanoUsd: 100 * 4_000 + 50 * 200 + 1 * 20_000 });
      // A stream cut off with no usage at all: Anthropic still has message_start.
      s.ai.queue.push({ start: { input_tokens: 7, cache_creation_input_tokens: 0, cache_read_input_tokens: 0, output_tokens: 1 }, fail: 'cut' });
      r = await chat(s, { model: 'anthropic/claude-opus-5-5', messages: [{ role: 'user', content: 'x' }] }, 'req-error-0002');
      expect((await frames(r)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
      expect(await rowFor(data, s, 'req-error-0002')).toMatchObject({ usageSource: 'partial', costNanoUsd: 7 * 4_000 + 20_000 });
      // OpenAI reports nothing before the end: the hold is billed, marked estimated.
      for (const [fail, id] of [['error', 'req-error-0003'], ['cut', 'req-error-0004']] as const) {
        s.ai.queue.push({ fail });
        r = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] }, id);
        expect((await frames(r)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
        const row = await rowFor(data, s, id);
        expect(row).toMatchObject({ kind: 'spend', usageSource: 'hold', estimated: true });
        expect(row.costNanoUsd).toBe(row.holdNanoUsd);
        expect(row.costNanoUsd).toBeGreaterThan(0);
      }
      // The provider refused the call (HTTP 400): no stream, nothing billed.
      s.ai.queue.push({ fail: 'http' });
      r = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] }, 'req-error-0005');
      expect((await frames(r)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
      expect(await rowFor(data, s, 'req-error-0005')).toMatchObject({ kind: 'spend', usageSource: 'none', costNanoUsd: 0 });
    } finally { await s.stop(); }
  }));

test('the output cap: at most what the balance allows (and the model max), never past the balance', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const opus = modelById('anthropic/claude-opus-5-5')!;
      // Plenty: the model's max.
      fxReduce(await frames(await chat(s, { model: opus.id, messages: [{ role: 'user', content: 'x' }] }, 'req-cap-0001')));
      expect(s.ai.requests.at(-1)!.body.max_tokens).toBe(opus.maxOutput);
      expect(s.ai.counts).toHaveLength(0);
      // The client's own limit stands when it is lower.
      fxReduce(await frames(await chat(s, { model: 'openai/gpt-6.2-sol', max_completion_tokens: 900, messages: [{ role: 'user', content: 'x' }] }, 'req-cap-0002')));
      expect(s.ai.requests.at(-1)!.body.max_output_tokens).toBe(900);
      // Little left: the cap shrinks to fit, from the provider's exact input count.
      await data.db.delete(creditLedger).where(and(eq(creditLedger.userId, s.user.id), ne(creditLedger.kind, 'grant')));
      await leave(data, s, 100_000_000); // $0.10
      const r = await chat(s, { model: opus.id, messages: [{ role: 'user', content: 'x' }] }, 'req-cap-0003');
      expect(r.status).toBe(200);
      fxReduce(await frames(r));
      expect(s.ai.counts).toHaveLength(1);
      const row = await rowFor(data, s, 'req-cap-0003');
      const sent = s.ai.requests.at(-1)!.body.max_tokens;
      expect(sent).toBe(row.outputCap!);
      expect(sent).toBeLessThan(opus.maxOutput);
      expect(sent).toBeGreaterThanOrEqual(1024);
      expect(row.holdNanoUsd).toBeLessThanOrEqual(100_000_000);
      // What the cap and the input bound allow can never cost more than the balance: the hold covers both.
      expect(sent * 20_000).toBeLessThanOrEqual(100_000_000);
      // Less than a minimal request can cost: 402, and the provider never sees it.
      await data.db.delete(creditLedger).where(and(eq(creditLedger.userId, s.user.id), ne(creditLedger.kind, 'grant')));
      await leave(data, s, 1_000_000); // $0.001
      const before = s.ai.requests.length;
      const refused = await chat(s, { model: opus.id, messages: [{ role: 'user', content: 'x' }] }, 'req-cap-0004');
      expect(refused.status).toBe(402);
      expect(await refused.json() as unknown).toEqual({ error: { code: 'credits_exhausted' } });
      expect(s.ai.requests.length).toBe(before);
    } finally { await s.stop(); }
  }));

test('parallel requests near zero: each sees the others’ holds, so together they never spend past the balance', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const body = (n: number) => ({ model: 'openai/gpt-6-luna', max_completion_tokens: 2000, messages: [{ role: 'user', content: `Write a lot [slow] ${n}` }] });
      // One request to learn what a request of this shape holds.
      fxReduce(await frames(await chat(s, body(0), 'req-probe-0000')));
      const hold = (await rowFor(data, s, 'req-probe-0000')).holdNanoUsd;
      await data.db.delete(creditLedger).where(and(eq(creditLedger.userId, s.user.id), ne(creditLedger.kind, 'grant')));
      const balance = 2 * hold + Math.floor(hold / 10);
      await leave(data, s, balance);
      const answers = await Promise.all([1, 2, 3, 4, 5].map(n => chat(s, body(n), `req-parallel-000${n}`)));
      const statuses = answers.map(r => r.status).sort();
      expect(statuses).toEqual([200, 200, 402, 402, 402]);
      for (const r of answers) await r.text();
      const requests = (await rows(data)).filter(row => row.idempotencyKey.includes('req-parallel'));
      expect(requests).toHaveLength(2);
      expect(requests.reduce((sum, row) => sum + row.holdNanoUsd, 0)).toBeLessThanOrEqual(balance);
      expect(requests.reduce((sum, row) => sum + row.costNanoUsd, 0)).toBeLessThanOrEqual(balance);
      for (const row of requests) expect(row).toMatchObject({ kind: 'spend', usageSource: 'reported' });
    } finally { await s.stop(); }
  }));

test('holds: a running request reserves its hold; an expired one (its Worker died) reserves nothing but keeps what it proved', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const insert = (id: string, hold: number, proven: number, expires: Date) => data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'hold', idempotencyKey: `request:${s.user.id}:${id}`, credits: 0, model: 'anthropic/claude-opus-5-5', holdNanoUsd: hold, costNanoUsd: proven, holdExpiresAt: expires });
      await insert('live', 100 * CREDIT_NANO_USD, 3 * CREDIT_NANO_USD, new Date(Date.now() + HOLD_TTL_MS));
      await insert('dead', 200 * CREDIT_NANO_USD, 2 * CREDIT_NANO_USD, new Date(Date.now() - 1000));
      // Used: what both proved (5). Remaining: 1000 - 5 - the live hold's other 97.
      expect((await billing(s)).credits).toEqual({ included: 1000, used: 5, remaining: 898 });
    } finally { await s.stop(); }
  }));

test('a period reset: last period’s spends and holds count for nothing in the new one', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const { subscriptionId } = await subscribe(s);
      s.ai.queue.push({ usage: { input_tokens: 1_000_000, input_tokens_details: { cached_tokens: 0 }, output_tokens: 0, total_tokens: 1_000_000 } });
      fxReduce(await frames(await chat(s, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: 'x' }] }, 'req-period-0001')));
      // 1M input over 272k: $4 = 800 credits.
      expect((await billing(s)).credits).toEqual({ included: 1000, used: 800, remaining: 200 });
      await new Promise(resolve => setTimeout(resolve, 1100));
      await s.stripe.renew(subscriptionId);
      expect((await billing(s)).credits).toEqual({ included: 1000, used: 0, remaining: 1000 });
      fxReduce(await frames(await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] }, 'req-period-0002')));
      const spent = (await rowFor(data, s, 'req-period-0002')).costNanoUsd;
      expect((await billing(s)).credits.used).toBe(Math.ceil(spent / CREDIT_NANO_USD));
    } finally { await s.stop(); }
  }));

test('display credits: derived from exact nano-USD, used rounds up, remaining down, the days add up to used', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      expect(displayCredits({ includedNano: 1000 * CREDIT_NANO_USD, usedNano: 1, heldNano: 0, remainingNano: 1000 * CREDIT_NANO_USD - 1 })).toEqual({ included: 1000, used: 1, remaining: 999 });
      expect(displayCredits({ includedNano: 1000 * CREDIT_NANO_USD, usedNano: 0, heldNano: 0, remainingNano: 1000 * CREDIT_NANO_USD })).toEqual({ included: 1000, used: 0, remaining: 1000 });
      await subscribe(s);
      // Three requests of 0.4 credits each: 1.2 credits spent, shown as 2 used, never 3.
      for (const id of ['a', 'b', 'c']) await data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'spend', idempotencyKey: `request:${s.user.id}:${id}`, credits: 0, costNanoUsd: 2_000_000, model: 'openai/gpt-6-luna' });
      const state = await billing(s);
      expect(state.credits).toEqual({ included: 1000, used: 2, remaining: 998 });
      expect(state.daily.reduce((sum: number, d: { credits: number }) => sum + d.credits, 0)).toBe(2);
    } finally { await s.stop(); }
  }));

test('Anthropic cache breakpoints: never more than four, all on the 5-minute TTL, whatever the thread shape', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const call = { id: 'call_1', type: 'function', function: { name: 'read_file', arguments: '{"path":"a"}' } };
      const shapes: Array<Record<string, unknown>> = [
        { messages: [{ role: 'user', content: 'one' }] },
        { messages: [SYSTEM, { role: 'user', content: 'one' }], tools: TOOLS },
        { messages: [SYSTEM, { role: 'developer', content: 'more rules' }, { role: 'user', content: 'Q1' }, { role: 'assistant', content: 'A1' }, { role: 'user', content: 'Q2' }], tools: TOOLS },
        { messages: [SYSTEM, { role: 'user', content: 'Q1' }, { role: 'assistant', content: null, tool_calls: [call] }, { role: 'tool', tool_call_id: 'call_1', content: 'ok' }, { role: 'assistant', content: 'A' }, { role: 'user', content: 'Q2' }, { role: 'assistant', content: null, tool_calls: [{ ...call, id: 'call_2' }] }, { role: 'tool', tool_call_id: 'call_2', content: 'ok' }], tools: TOOLS },
      ];
      for (const [i, shape] of shapes.entries()) {
        fxReduce(await frames(await chat(s, { model: 'anthropic/claude-opus-5-5', ...shape }, `req-marks-000${i}`)));
        const sent = JSON.stringify(s.ai.requests.at(-1)!.body);
        const marks = sent.match(/"cache_control":\{[^}]*\}/g) ?? [];
        expect(marks.length).toBeGreaterThanOrEqual(1);
        expect(marks.length).toBeLessThanOrEqual(4);
        for (const mark of marks) expect(mark).toBe('"cache_control":{"type":"ephemeral"}');
      }
    } finally { await s.stop(); }
  }));

test('the input bound is above what the provider counts, and images are bounded by pixels, not base64', async () => {
  const body = { model: 'm', system: [{ type: 'text', text: 'é'.repeat(1000) }], messages: [{ role: 'user', content: [{ type: 'text', text: 'hello world '.repeat(500) }] }] };
  // A byte-level tokenizer can't take more tokens than bytes: 2,000 bytes of é and 6,000 of ASCII.
  expect(inputBound(body)).toBeGreaterThanOrEqual(8_000);
  // A 1x1 PNG's bound is its pixel bound, and a big base64 payload doesn't count byte for byte.
  const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';
  expect(imageBound(png)).toBe(1 + 1024);
  const withImage = (data: string) => inputBound({ messages: [{ role: 'user', content: [{ type: 'image', source: { type: 'base64', media_type: 'image/png', data } }] }] });
  const padded = Buffer.concat([Buffer.from(png, 'base64'), Buffer.alloc(300_000)]).toString('base64');
  expect(withImage(padded)).toBe(withImage(png));
  // Unreadable: its base64 length.
  expect(imageBound('notanimage')).toBe(10);
});
