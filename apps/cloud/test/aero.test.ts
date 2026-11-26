import { expect, test } from 'bun:test';
import { eq } from 'drizzle-orm';
import { withData } from './db';
import { billing, frames, fxReduce, stack, subscribe, type Stack } from './billing-helpers';
import { creditLedger } from '../src/db/schema';
import { consume } from '../src/abuse';
import { CREDIT_MICRO_USD, MODELS, cost, modelById } from '../src/aero/catalogue';

const TOOLS = [{ type: 'function', function: { name: 'read_file', description: 'Read a file', parameters: { type: 'object', properties: { path: { type: 'string' } }, required: ['path'] } } }];
const SYSTEM = { role: 'system', content: 'You are Aero, the agent in Powermove.' };
const chat = (s: Stack, body: Record<string, unknown>, headers: Record<string, string> = {}) =>
  s.request('/v2/aero/chat/completions', { json: { stream: true, stream_options: { include_usage: true }, ...body } }, { ...s.user.headers, ...headers });
const spends = async (s: Stack, data: Parameters<Parameters<typeof withData>[0]>[0]) => data.db.select().from(creditLedger).where(eq(creditLedger.kind, 'spend'));

test('the catalogue: v1 shape, provider ids, efforts, one default', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      const r = await s.request('/v2/aero/models');
      expect(r.status).toBe(200);
      const { data: models } = await r.json() as { data: any[] };
      expect(models.map(m => m.id)).toEqual(['openai/gpt-6.2-sol', 'openai/gpt-6.2-astra', 'openai/gpt-6-luna', 'anthropic/claude-opus-5-5', 'anthropic/claude-sonnet-5-5']);
      expect(models.filter(m => m.default)).toHaveLength(1);
      for (const m of models) { expect(m.id).toMatch(/^(openai|anthropic)\/[A-Za-z0-9._-]+$/); expect(m.efforts).toContain(m.defaultEffort); }
    } finally { await s.stop(); }
  }));

test('credits are ceil(cost_usd * 200), cached input at the cached rate', () => {
  const sol = modelById('openai/gpt-6.2-sol')!;
  expect(CREDIT_MICRO_USD).toBe(5_000);
  // 1M fresh input at $1.75 = $1.75 = 350 credits.
  expect(cost(sol, { input: 1_000_000, cacheRead: 0, cacheWrite: 0, output: 0 })).toEqual({ microUsd: 1_750_000, credits: 350 });
  // The same input fully cached costs a tenth.
  expect(cost(sol, { input: 1_000_000, cacheRead: 1_000_000, cacheWrite: 0, output: 0 }).credits).toBe(35);
  // Anything above zero is at least one credit.
  expect(cost(sol, { input: 1, cacheRead: 0, cacheWrite: 0, output: 0 }).credits).toBe(1);
  const opus = modelById('anthropic/claude-opus-5-5')!;
  expect(cost(opus, { input: 1_000_000, cacheRead: 0, cacheWrite: 1_000_000, output: 0 }).credits).toBe(1250);
  for (const model of MODELS) expect(model.price.cachedInput).toBeLessThan(model.price.input);
});

test('402 at zero credits: Free accounts and spent plans', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      let r = await chat(s, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: 'hi' }] });
      expect(r.status).toBe(402);
      expect(await r.json() as unknown).toEqual({ error: { code: 'credits_exhausted' } });
      await subscribe(s);
      await data.db.insert(creditLedger).values({ userId: s.user.id, kind: 'spend', idempotencyKey: 'request:all', credits: 1000, model: 'openai/gpt-6.2-sol' });
      r = await chat(s, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: 'hi' }] });
      expect(r.status).toBe(402);
      expect(s.ai.requests).toHaveLength(0);
    } finally { await s.stop(); }
  }));

test('bad requests answer codes, never text; the rate limit answers 429', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      for (const body of [{ model: 'openai/gpt-6.2-sol', messages: [] }, { model: 'nope/model', messages: [{ role: 'user', content: 'x' }] }, { model: 'openai/gpt-6.2-sol', reasoning_effort: 'max', messages: [{ role: 'user', content: 'x' }] }, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: [{ type: 'image_url', image_url: { url: 'https://example.com/a.png' } }] }] }]) {
        const r = await chat(s, body);
        expect(r.status).toBe(400);
        const json = await r.json() as any;
        expect(Object.keys(json.error)).toEqual(['code']);
      }
      for (let i = 0; i < 60; i++) await consume(data.db, 'aero_user', s.user.id);
      const r = await chat(s, { model: 'openai/gpt-6.2-sol', messages: [{ role: 'user', content: 'x' }] });
      expect(r.status).toBe(429);
      expect(await r.json() as unknown).toEqual({ error: { code: 'rate_limited' } });
      expect(Number(r.headers.get('retry-after'))).toBeGreaterThan(0);
    } finally { await s.stop(); }
  }));

for (const model of ['openai/gpt-6.2-sol', 'anthropic/claude-opus-5-5']) {
  test(`${model}: the SSE stream is what fx's openai-chat-completions reducer accepts, tool calls and reasoning included, and it meters one ledger row`, async () =>
    withData(async (data) => {
      const s = await stack(data);
      try {
        await subscribe(s);
        const r = await chat(s, { model, reasoning_effort: 'high', messages: [SYSTEM, { role: 'user', content: 'Read the project file.' }], tools: TOOLS }, { 'X-Powermove-Request-Id': 'req-tool-turn-1' });
        expect(r.status).toBe(200);
        expect(r.headers.get('content-type')).toStartWith('text/event-stream');
        const stream = await frames(r);
        const reply = fxReduce(stream);
        expect(reply.model).toBe(model);
        expect(reply.finish).toBe('tool_calls');
        expect(reply.tools).toHaveLength(1);
        expect(reply.tools[0]!.name).toBe('read_file');
        expect(JSON.parse(reply.tools[0]!.arguments)).toEqual({ note: 'from the simulator' });
        expect(reply.content).toContain('read_file');
        expect(reply.reasoning).toBe('Thinking about the request.');
        expect(reply.details).toHaveLength(1);
        expect(stream.at(-1)).toBe('[DONE]');
        const last = stream.at(-2) as any;
        expect(last.choices).toEqual([]);
        expect(last.usage.prompt_tokens).toBeGreaterThan(0);
        // One ledger row, from the provider's reported usage.
        const rows = await spends(s, data);
        expect(rows).toHaveLength(1);
        const row = rows[0]!;
        expect(row.idempotencyKey).toBe(`request:${s.user.id}:req-tool-turn-1`);
        expect(row.estimated).toBe(false);
        expect(row.inputTokens).toBe(last.usage.prompt_tokens);
        expect(row.outputTokens).toBe(last.usage.completion_tokens);
        expect(row.cachedInputTokens).toBe(last.usage.prompt_tokens_details.cached_tokens);
        expect(row.credits).toBe(cost(modelById(model)!, { input: row.inputTokens, cacheRead: row.cachedInputTokens, cacheWrite: row.cacheWriteTokens, output: row.outputTokens }).credits);
        expect(row.credits).toBeGreaterThan(0);
        expect((await billing(s)).credits).toEqual({ included: 1000, used: row.credits, remaining: 1000 - row.credits });

        // The tool turn goes back with the replayed reasoning; the provider sees its own continuity data again.
        const call = reply.tools[0]!;
        const next = await chat(s, { model, reasoning_effort: 'high', tools: TOOLS, messages: [
          SYSTEM, { role: 'user', content: 'Read the project file.' },
          { role: 'assistant', content: reply.content, reasoning_content: reply.reasoning, reasoning_details: reply.details, tool_calls: [{ id: call.id, type: 'function', function: { name: call.name, arguments: call.arguments } }] },
          { role: 'tool', tool_call_id: call.id, content: '{"ok":true}' },
        ] }, { 'X-Powermove-Request-Id': 'req-tool-turn-2' });
        const second = fxReduce(await frames(next));
        expect(second.finish).toBe('stop');
        expect(second.tools).toHaveLength(0);
        expect(second.content).toBe('The tool finished. Done.');
        expect(second.usage.cached).toBeGreaterThan(0);
        const sent = s.ai.requests.at(-1)!.body;
        if (model.startsWith('anthropic/')) {
          const assistant = sent.messages.find((m: any) => m.role === 'assistant');
          expect(assistant.content[0]).toMatchObject({ type: 'thinking', signature: 'sim-signature' });
        } else {
          expect(JSON.stringify(sent.input)).toContain('sim-encrypted-reasoning');
        }
        expect(await spends(s, data)).toHaveLength(2);
      } finally { await s.stop(); }
    }));
}

test('caching is on for both providers: a stable prompt_cache_key per thread, Anthropic breakpoints on system, tools and the newest message', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const thread = [SYSTEM, { role: 'user', content: 'First question.' }];
      await (await chat(s, { model: 'openai/gpt-6.2-sol', messages: thread })).text();
      await (await chat(s, { model: 'openai/gpt-6.2-sol', messages: [...thread, { role: 'assistant', content: 'Answer.' }, { role: 'user', content: 'Follow-up.' }] })).text();
      await (await chat(s, { model: 'openai/gpt-6.2-sol', messages: [SYSTEM, { role: 'user', content: 'Another thread.' }] })).text();
      const keys = s.ai.requests.filter(r => r.provider === 'openai').map(r => r.body.prompt_cache_key);
      expect(keys[0]).toMatch(/^pm-[0-9a-f]{40}$/);
      expect(keys[1]).toBe(keys[0]);
      expect(keys[2]).not.toBe(keys[0]);
      expect(s.ai.requests[0]!.body.store).toBe(false);

      await (await chat(s, { model: 'anthropic/claude-sonnet-5-5', tools: TOOLS, messages: [SYSTEM, { role: 'user', content: 'Q1' }, { role: 'assistant', content: 'A1' }, { role: 'user', content: 'Q2' }] })).text();
      const sent = s.ai.requests.at(-1)!.body;
      expect(sent.system.at(-1).cache_control).toEqual({ type: 'ephemeral' });
      expect(sent.tools.at(-1).cache_control).toEqual({ type: 'ephemeral' });
      expect(sent.messages.at(-1).content.at(-1).cache_control).toEqual({ type: 'ephemeral' });
      // The previous request's last message is a breakpoint too, so this turn reads the conversation from the cache.
      expect(sent.messages[0].content.at(-1).cache_control).toEqual({ type: 'ephemeral' });
      const marks = JSON.stringify(sent).match(/"cache_control"/g) ?? [];
      expect(marks.length).toBeLessThanOrEqual(4);
      expect(sent.thinking?.type).toBe('adaptive');
      expect(sent.output_config?.effort).toBe('high');
    } finally { await s.stop(); }
  }));

test('a replayed request id is refused and meters nothing twice', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const body = { model: 'openai/gpt-6-luna', messages: [{ role: 'user', content: 'once' }] };
      const first = await chat(s, body, { 'X-Powermove-Request-Id': 'req-replayed-0001' });
      expect(first.status).toBe(200);
      await first.text();
      const again = await chat(s, body, { 'X-Powermove-Request-Id': 'req-replayed-0001' });
      expect(again.status).toBe(409);
      expect(await again.json() as unknown).toEqual({ error: { code: 'duplicate_request' } });
      expect(await spends(s, data)).toHaveLength(1);
      expect(s.ai.requests).toHaveLength(1);
    } finally { await s.stop(); }
  }));

test('a client that aborts mid-stream still gets exactly one ledger row, and the provider call stops', async () =>
  withData(async (data) => {
    const s = await stack(data);
    try {
      await subscribe(s);
      const r = await chat(s, { model: 'anthropic/claude-opus-5-5', messages: [{ role: 'user', content: 'Write a lot [slow]' }] }, { 'X-Powermove-Request-Id': 'req-aborted-0001' });
      expect(r.status).toBe(200);
      const reader = r.body!.getReader();
      const decoder = new TextDecoder();
      let seen = '';
      while (!seen.includes('word3')) seen += decoder.decode((await reader.read()).value);
      await reader.cancel();
      let rows = await spends(s, data);
      for (let i = 0; i < 50 && (rows.length !== 1 || rows[0]!.credits === 0); i++) { await new Promise(resolve => setTimeout(resolve, 20)); rows = await spends(s, data); }
      expect(rows).toHaveLength(1);
      expect(rows[0]!.estimated).toBe(true);
      expect(rows[0]!.outputTokens).toBeGreaterThan(0);
      expect(rows[0]!.credits).toBeGreaterThan(0);
      await new Promise(resolve => setTimeout(resolve, 300));
      expect(await spends(s, data)).toHaveLength(1);
      expect(s.ai.aborted).toBe(1);
    } finally { await s.stop(); }
  }));
