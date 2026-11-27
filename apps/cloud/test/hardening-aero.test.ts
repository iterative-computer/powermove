/* Aero hardening: body caps before parsing, request bounds, images, abuse ceilings, the circuit breaker and call
   deadlines. */
import { afterEach, expect, test } from 'bun:test';
import { eq } from 'drizzle-orm';
import { withData } from './db';
import { frames, stack, subscribe, type Stack } from './billing-helpers';
import { aeroBlocks, creditLedger } from '../src/db/schema';
import { modelById } from '../src/aero/catalogue';
import { PRICING } from './pricing';
import { DEADLINES, MAX_DRAIN_MS, Meter } from '../src/aero/meter';
import type { Data } from '../src/db/client';

const chat = (s: Stack, body: Record<string, unknown>, id = `req-h-${crypto.randomUUID().slice(0, 8)}`) =>
  s.request('/v2/aero/chat/completions', { json: { stream: true, ...body } }, { ...s.user.headers, 'X-Powermove-Request-Id': id });
const rowFor = async (data: Data, s: Stack, id: string) => (await data.db.select().from(creditLedger).where(eq(creditLedger.idempotencyKey, `request:${s.user.id}:${id}`)))[0];
const until = async (check: () => Promise<boolean>, ms = 8000) => { for (let i = 0; i < ms / 20 && !await check(); i++) await new Promise(resolve => setTimeout(resolve, 20)); };
/** A PNG header for any size (the rest of the file needn't be real: only the header is read). */
const png = (width: number, height: number) => Buffer.concat([Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52]), Buffer.from([width >>> 24, (width >>> 16) & 255, (width >>> 8) & 255, width & 255, height >>> 24, (height >>> 16) & 255, (height >>> 8) & 255, height & 255]), Buffer.alloc(8)]).toString('base64');
const image = (base64: string, type = 'image/png') => ({ type: 'image_url', image_url: { url: `data:${type};base64,${base64}` } });
const saved = { ...DEADLINES };
afterEach(() => { Object.assign(DEADLINES, saved); });

test('4. the body is capped before it is parsed, and who, how often and the plan are checked before that', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      // Signed out: 401 whatever the body. Free: 402 before the (unparseable) body is read.
      expect((await s.request('/v2/aero/chat/completions', { method: 'POST', body: '{not json' }, {})).status).toBe(401);
      expect((await s.request('/v2/aero/chat/completions', { method: 'POST', body: '{not json' })).status).toBe(402);
      await subscribe(s);
      // A declared length over 8 MiB is refused outright; a streamed body is cut off as it passes the cap.
      expect((await s.request('/v2/aero/chat/completions', { method: 'POST', body: '{}', headers: { 'content-length': String(8 * 1024 * 1024 + 1) } })).status).toBe(413);
      const big = new ReadableStream<Uint8Array>({ start(controller) { for (let i = 0; i < 9; i++) controller.enqueue(new Uint8Array(1024 * 1024).fill(32)); controller.close(); } });
      const streamed = await s.app.request('/v2/aero/chat/completions', { method: 'POST', body: big, headers: { ...s.user.headers, 'content-type': 'application/json' }, duplex: 'half' } as RequestInit, s.env);
      expect(streamed.status).toBe(413);
      // The webhook: 512 KiB.
      const hook = await s.app.request('/v2/stripe/webhook', { method: 'POST', body: 'x'.repeat(600 * 1024), headers: { 'stripe-signature': 't=1,v1=x' } }, s.env);
      expect(hook.status).toBe(413);
      expect(s.ai.requests).toHaveLength(0);
    } finally { await s.stop(); }
  }));

test('4. requests are bounded: message count, tool schema size and depth', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const deep = (n: number): Record<string, unknown> => (n === 0 ? { type: 'string' } : { type: 'object', properties: { a: deep(n - 1) } });
      for (const body of [
        { model: 'openai/gpt-6-luna', messages: Array.from({ length: 2_001 }, () => ({ role: 'user', content: 'x' })) },
        { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }], tools: [{ type: 'function', function: { name: 'deep', parameters: deep(30) } }] },
        { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }], tools: [{ type: 'function', function: { name: 'big', parameters: { type: 'object', description: 'x'.repeat(70_000) } } }] },
      ]) expect((await chat(s, body)).status).toBe(400);
      expect(s.ai.requests).toHaveLength(0);
    } finally { await s.stop(); }
  }));

test('3. images: decoded from their own header; undecodable, unsupported, oversized or too many are refused', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const ask = (content: unknown[]) => chat(s, { model: 'anthropic/claude-sonnet-5-5', max_completion_tokens: 2_000, messages: [{ role: 'user', content: [{ type: 'text', text: 'look' }, ...content] }] });
      expect((await ask([image('AAAAAAAA')])).status).toBe(400);
      expect((await ask([image(Buffer.from('BM fake bitmap').toString('base64'), 'image/png')])).status).toBe(400);
      expect((await ask([image(png(9000, 10))])).status).toBe(400);
      expect((await ask(Array.from({ length: 21 }, () => image(png(10, 10))))).status).toBe(400);
      expect(s.ai.requests).toHaveLength(0);
      // A real one goes, typed by its header (not by what the client claimed).
      const ok = await ask([image(png(400, 300), 'image/webp')]);
      expect(ok.status).toBe(200);
      await ok.text();
      expect(JSON.stringify(s.ai.requests[0]!.body)).toContain('"media_type":"image/png"');
    } finally { await s.stop(); }
  }));

test('3. the circuit breaker: a settled cost over its hold blocks the account until someone reviews it', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      s.ai.queue.push({ usage: { input_tokens: 1_000_000, input_tokens_details: { cached_tokens: 0 }, output_tokens: 0, total_tokens: 1_000_000 } });
      await (await chat(s, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: 'x' }] })).text();
      expect(await data.db.select().from(aeroBlocks)).toHaveLength(1);
      const next = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] });
      expect(next.status).toBe(403);
      expect(await next.json() as unknown).toEqual({ error: { code: 'account_review' } });
      expect(s.ai.requests).toHaveLength(1);
    } finally { await s.stop(); }
  }));

test('5. abuse ceilings: a per-IP edge limit, three calls in flight per account, and a global daily dollar ceiling', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // The edge limit (Workers Rate Limiting, by client IP).
      const limited = { ...s.env, RL_AERO: { limit: async () => ({ success: false }) } as RateLimit };
      expect((await s.app.request('/v2/aero/chat/completions', { method: 'POST', body: '{}', headers: { ...s.user.headers, 'content-type': 'application/json' } }, limited)).status).toBe(429);
      // Three in flight, the fourth refused.
      const slow = { model: 'openai/gpt-6-luna', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'Write a lot [slow]' }] };
      const answers = await Promise.all([1, 2, 3, 4].map(() => chat(s, slow)));
      expect(answers.map(r => r.status).sort()).toEqual([200, 200, 200, 429]);
      const refused = answers.find(r => r.status === 429)!;
      expect(await refused.json() as unknown).toEqual({ error: { code: 'too_many_calls' } });
      for (const r of answers) if (r !== refused) await r.text();
      // The global ceiling: across all accounts, a UTC day's spend and holds stay under AERO_GLOBAL_DAILY_USD.
      const capped = { ...s.env, AERO_GLOBAL_DAILY_USD: '0.001' };
      s.calls.env = () => capped;
      const r = await s.app.request('/v2/aero/chat/completions', { method: 'POST', body: JSON.stringify({ ...slow, stream: true }), headers: { ...s.user.headers, 'content-type': 'application/json' } }, capped);
      expect(r.status).toBe(503);
      expect(await r.json() as unknown).toEqual({ error: { code: 'capacity_exhausted' } });
    } finally { await s.stop(); }
  }));

test('5. provider egress fails closed: anything but the one metered generation call is refused', async () =>
  withData(async (data) => {
    const sent: string[] = [];
    const meter = new Meter({
      data, store: { get: async () => undefined, put: async () => {}, deleteAll: async () => {}, setAlarm: async () => {}, deleteAlarm: async () => {} },
      userId: 'u', key: 'request:u:x', model: modelById('openai/gpt-6-luna')!, requestedMax: undefined, globalDailyNano: 50e9, pricing: PRICING,
      fetch: async (input) => { sent.push(String(input)); return new Response('{}'); },
    });
    await expect(meter.fetch('https://api.openai.com/v1/files', { method: 'GET' })).rejects.toThrow('unmetered');
    await expect(meter.fetch('https://api.openai.com/v1/responses/input_tokens', { method: 'POST', body: '{}' })).rejects.toThrow('unmetered');
    await expect(meter.fetch('https://api.openai.com/v1/responses', { method: 'POST', body: JSON.stringify({ model: 'gpt-6.2-astra', stream: true }) })).rejects.toThrow('unmetered');
    expect(sent).toEqual([]);
  }));

test('9. deadlines: admission, upstream silence and the whole call; past them the call stops and an uncertain one bills its hold', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      // Upstream silence: the provider goes quiet after its first text.
      DEADLINES.idleMs = 300;
      s.ai.queue.push({ fail: 'stall' });
      const idle = await chat(s, { model: 'anthropic/claude-sonnet-5-5', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'x' }] }, 'req-idle-0001');
      expect((await frames(idle)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
      await until(async () => (await rowFor(data, s, 'req-idle-0001'))?.kind === 'spend');
      let row = (await rowFor(data, s, 'req-idle-0001'))!;
      expect(row).toMatchObject({ usageSource: 'hold', estimated: true });
      expect(row.costNanoUsd).toBe(row.holdNanoUsd);
      // The whole call: a slow stream past the total deadline.
      Object.assign(DEADLINES, saved);
      DEADLINES.totalMs = 400;
      const total = await chat(s, { model: 'openai/gpt-6-luna', max_completion_tokens: 2_000, messages: [{ role: 'user', content: 'Write a lot [slow]' }] }, 'req-total-0001');
      expect((await frames(total)).at(-1)).toEqual({ error: { code: 'upstream_error' } });
      await until(async () => (await rowFor(data, s, 'req-total-0001'))?.kind === 'spend');
      row = (await rowFor(data, s, 'req-total-0001'))!;
      expect(row).toMatchObject({ usageSource: 'hold', estimated: true });
      expect(MAX_DRAIN_MS).toBe(15 * 60_000);
    } finally { await s.stop(); }
  }));

/** The call objects' connection, every select inside a transaction delayed by `ms` (a slow database). */
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

test('9. an admission that cannot finish in time is a 503, and the call is never sent (nothing billed)', async () =>
  withData(async (data) => {
    const s = await stack(data, 'jude@example.com', { callData: slowSelects(data, 150) });
    try {
      await subscribe(s);
      DEADLINES.admissionMs = 100;
      const late = await chat(s, { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'x' }] }, 'req-late-0001');
      expect(late.status).toBe(503);
      expect(await late.json() as unknown).toEqual({ error: { code: 'upstream_timeout' } });
      await until(async () => (await rowFor(data, s, 'req-late-0001'))?.kind === 'spend', 3000);
      expect(s.ai.requests).toHaveLength(0);
      expect(await rowFor(data, s, 'req-late-0001')).toMatchObject({ kind: 'spend', usageSource: 'none', costNanoUsd: 0 });
    } finally { await s.stop(); }
  }));
