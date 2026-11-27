import type { Data } from '../src/db/client';
import { createApp } from '../src/app';
import { makeEnv } from './env';
import { seedSession } from './helpers';
import { FAKE_SECRET_KEY, FAKE_WEBHOOK_SECRET, startFakeStripe, type FakeStripe } from '../scripts/fake-stripe';
import { startAiSimulator, type Simulator } from '../scripts/ai-simulator';
import { FakeCalls } from './calls';
import { desc, eq } from 'drizzle-orm';
import { subscriptions } from '../src/db/schema';

export interface Stack {
  app: ReturnType<typeof createApp>;
  env: CloudflareBindings;
  stripe: FakeStripe;
  ai: Simulator;
  /** The AERO_CALLS Durable Objects. */
  calls: FakeCalls;
  user: Awaited<ReturnType<typeof seedSession>>;
  request(path: string, init?: RequestInit & { json?: unknown }, headers?: Record<string, string>): Promise<Response>;
  stop(): Promise<void>;
}

/** The Worker on PGlite, with the fake Stripe delivering its webhooks straight to it, the AI simulator behind the AI
    SDK, and the Aero calls' Durable Objects in process. `callData` is the connection the call objects get (a test
    can make it fail). */
export async function stack(data: Data, email = 'jude@example.com', options: { callData?: (env: CloudflareBindings) => Data } = {}): Promise<Stack> {
  const app = createApp({ data: () => data });
  let deliver: (payload: string, signature: string) => Promise<number> = async () => 0;
  const stripe = startFakeStripe({ seedPrices: true, deliver: (payload, signature) => deliver(payload, signature) });
  const ai = startAiSimulator();
  let env: CloudflareBindings;
  const calls = new FakeCalls(() => env, options.callData ?? (() => data));
  env = {
    ...makeEnv(data),
    AERO_CALLS: calls as unknown as DurableObjectNamespace,
    STRIPE_SECRET_KEY: FAKE_SECRET_KEY, STRIPE_WEBHOOK_SECRET: FAKE_WEBHOOK_SECRET, STRIPE_API_BASE: stripe.url,
    // The exact price ids, as production configures them (the Worker never resolves a plan by lookup key).
    STRIPE_PRICE_AEROLITE: priceId(stripe, 'powermove_aerolite_monthly'), STRIPE_PRICE_AERO: priceId(stripe, 'powermove_aero_monthly'),
    OPENAI_API_KEY: 'simulator', ANTHROPIC_API_KEY: 'simulator',
    AERO_OPENAI_BASE_URL: `${ai.url}/openai/v1`, AERO_ANTHROPIC_BASE_URL: `${ai.url}/anthropic/v1`,
  };
  deliver = async (payload, signature) => (await app.request('/v2/stripe/webhook', { method: 'POST', headers: { 'content-type': 'application/json', 'stripe-signature': signature }, body: payload }, env)).status;
  const user = await seedSession(data, env, email);
  const request = async (path: string, init: RequestInit & { json?: unknown } = {}, headers: Record<string, string> = user.headers): Promise<Response> => {
    const { json, ...rest } = init;
    return app.request(path, { ...rest, ...(json !== undefined && { method: rest.method ?? 'POST', body: JSON.stringify(json) }), headers: { 'content-type': 'application/json', ...headers, ...(rest.headers as Record<string, string>) } }, env);
  };
  return { app, env, stripe, ai, calls, user, request, stop: async () => { await stripe.stop(); await ai.stop(); } };
}

/** The fake's price id for a lookup key. */
export const priceId = (stripe: FakeStripe, lookupKey: string) => [...stripe.state.prices.values()].find(price => price.lookup_key === lookupKey)!.id as string;

/** Subscribes the stack's user through Checkout and the fake's Pay button. */
export async function subscribe(s: Stack, plan: 'aerolite' | 'aero' = 'aerolite') {
  const r = await s.request('/v2/billing/checkout', { json: { plan } });
  if (r.status !== 200) throw new Error(`checkout ${r.status} ${await r.text()}`);
  const { url } = await r.json() as { url: string };
  const sessionId = url.split('/').at(-1)!;
  const success = await s.stripe.pay(sessionId);
  const subscriptionId = s.stripe.state.sessions.get(sessionId)!.subscription as string;
  return { url, sessionId, success, subscriptionId };
}

export async function billing(s: Stack) { return await (await s.request('/v2/billing')).json() as any; }

/** The SSE frames of a response body, parsed. `[DONE]` stays a string. */
export async function frames(response: Response): Promise<Array<Record<string, any> | '[DONE]'>> {
  const text = await response.text();
  return text.split('\n\n').filter(Boolean).map(block => {
    const data = block.split('\n').filter(line => line.startsWith('data: ')).map(line => line.slice(6)).join('\n');
    return data === '[DONE]' ? '[DONE]' : JSON.parse(data);
  });
}

/** fx's Chat Completions reducer rules (apps/native/vendor/fx/src/gateway/chat_completions_protocol.zig), ported to
    check a whole stream: what it would accept, and the reply it would build. Throws on anything it would refuse. */
export function fxReduce(stream: Array<Record<string, any> | '[DONE]'>) {
  let phase: 'open' | 'finished' | 'done' = 'open';
  let id: string | undefined, model: string | undefined, finish: string | undefined, content = '', reasoning = '';
  const details: unknown[] = [];
  const tools: Array<{ id?: string; name: string; arguments: string }> = [];
  let usage: { input?: number; output?: number; total?: number; cached?: number } = {};
  const fail = (why: string) => { throw new Error(`fx would refuse: ${why}`); };
  const acceptUsage = (u: any) => {
    if (typeof u !== 'object' || u === null) fail('usage not an object');
    const input = u.prompt_tokens, output = u.completion_tokens, total = u.total_tokens;
    for (const n of [input, output, total]) if (n !== undefined && (!Number.isInteger(n) || n < 0)) fail('usage count');
    if (total !== undefined && input !== undefined && output !== undefined && total !== input + output) fail('total != input + output');
    const cached = u.prompt_tokens_details?.cached_tokens;
    usage = { input, output, total, cached: Number.isInteger(cached) && cached <= input ? cached : undefined };
  };
  for (const frame of stream) {
    if (frame === '[DONE]') { if (phase !== 'finished') fail('[DONE] before finish'); phase = 'done'; continue; }
    if (phase === 'done') fail('data after [DONE]');
    if (frame.error != null) fail('error event');
    if (frame.id !== undefined) { if (id && id !== frame.id) fail('id changed'); id = frame.id; }
    if (frame.model !== undefined) { if (model && model !== frame.model) fail('model changed'); model = frame.model; }
    if (!Array.isArray(frame.choices)) fail('no choices');
    if (frame.choices.length > 1) fail('more than one choice');
    if (frame.choices.length === 0) { if (phase !== 'finished') fail('empty choices before finish'); if (!frame.usage) fail('empty choices without usage'); acceptUsage(frame.usage); continue; }
    const choice = frame.choices[0];
    if (choice.index !== 0) fail('choice index');
    const delta = choice.delta;
    if (typeof delta !== 'object' || delta === null) fail('no delta');
    if (phase === 'finished') {
      if (choice.finish_reason !== finish) fail('inconsistent finish');
      for (const [key, value] of Object.entries(delta)) if (!(key === 'role' && value === 'assistant') && !(key === 'content' && (value === null || value === ''))) fail('delta after finish');
      if (!frame.usage) fail('post-finish choice without usage');
      acceptUsage(frame.usage);
      continue;
    }
    if (delta.role !== undefined && delta.role !== 'assistant') fail('role');
    if (delta.function_call != null || delta.audio != null) fail('function_call/audio');
    for (const key of ['reasoning', 'reasoning_content']) if (delta[key] != null) { if (typeof delta[key] !== 'string') fail('reasoning type'); reasoning += delta[key]; }
    if (delta.reasoning_details != null) { if (!Array.isArray(delta.reasoning_details)) fail('reasoning_details'); for (const item of delta.reasoning_details) { if (typeof item !== 'object' || item === null) fail('reasoning_details item'); details.push(item); } }
    if (delta.content != null) { if (typeof delta.content !== 'string') fail('content type'); content += delta.content; }
    if (delta.tool_calls != null) {
      if (!Array.isArray(delta.tool_calls)) fail('tool_calls');
      const seen = new Set<number>();
      for (const call of delta.tool_calls) {
        if (!Number.isInteger(call.index)) fail('tool index');
        if (seen.has(call.index)) fail('tool index twice in a delta');
        seen.add(call.index);
        while (tools.length <= call.index) tools.push({ name: '', arguments: '' });
        const tool = tools[call.index]!;
        if (call.type != null && call.type !== 'function') fail('tool type');
        if (call.id) { if (tool.id && tool.id !== call.id) fail('tool id changed'); tool.id = call.id; }
        if (call.function?.name != null) tool.name += call.function.name;
        if (call.function?.arguments != null) tool.arguments += call.function.arguments;
      }
    }
    if (frame.usage != null) acceptUsage(frame.usage);
    if (choice.finish_reason != null) {
      if (!['stop', 'tool_calls', 'length', 'content_filter'].includes(choice.finish_reason)) fail('finish reason');
      finish = choice.finish_reason; phase = 'finished';
    }
  }
  if (phase !== 'done') fail('incomplete stream');
  if (finish !== 'length' && (finish === 'tool_calls') !== (tools.length > 0)) fail('finish_reason disagrees with tool calls');
  for (const tool of tools) { if (!/^[A-Za-z0-9_-]{1,128}$/.test(tool.name)) fail('tool name'); JSON.parse(tool.arguments); }
  return { id, model, finish, content, reasoning, details, tools, usage };
}

/** The user's current billing period, as the subscription says. */
export async function periodOf(data: Data, userId: string) {
  const [sub] = await data.db.select().from(subscriptions).where(eq(subscriptions.userId, userId)).orderBy(desc(subscriptions.currentPeriodStart)).limit(1);
  return { periodStart: sub!.currentPeriodStart, periodEnd: sub!.currentPeriodEnd };
}
