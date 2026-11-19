/* The hosted model proxy. The desktop's API-compatible runner talks to it
   exactly like any OpenAI-compatible endpoint, with the Powermove session as
   its bearer key. We check the plan, forward to Anthropic or OpenAI with our
   own key, meter usage from the response and record its list-price cost. */
import { Hono, type Context } from 'hono';
import { ApiError } from '@powermove/registry/wire';
import type { Env } from '../env';
import { usageEvents } from '../db/schema';
import { requireSession } from './session';
import { loadAccount, providerSpend } from '../billing/account';
import { costMicros, MODEL_PROVIDERS, MODELS, resolveModel, type HostedModel, type TokenUsage } from '../billing/plans';
import { checkModelBudget } from '../billing/quotas';
import { estimateUsage, meteredStream, usageFromJson } from '../billing/meter';

/** Output ceiling per request; keeps a single call from blowing far past the remaining budget. */
export const MAX_OUTPUT_TOKENS = 16_000;
const MAX_BODY_BYTES = 8 * 1024 * 1024;

function upstream(env: CloudflareBindings, model: HostedModel): { url: string; headers: Record<string, string> } {
  if (model.provider === 'anthropic') {
    const key = env.AEROLITE_ANTHROPIC_API_KEY;
    if (!key) throw new ApiError({ error: 'internal', detail: 'hosted Claude models are not configured' });
    return { url: `${env.AEROLITE_ANTHROPIC_BASE_URL ?? 'https://api.anthropic.com/v1'}/chat/completions`, headers: { 'x-api-key': key, Authorization: `Bearer ${key}` } };
  }
  const key = env.AEROLITE_OPENAI_API_KEY;
  if (!key) throw new ApiError({ error: 'internal', detail: 'hosted OpenAI models are not configured' });
  return { url: `${env.AEROLITE_OPENAI_BASE_URL ?? 'https://api.openai.com/v1'}/chat/completions`, headers: { Authorization: `Bearer ${key}` } };
}

async function record(c: Context<Env>, userId: string, model: HostedModel, usage: TokenUsage) {
  const data = c.var.freshData();
  try {
    await data.db.insert(usageEvents).values({
      userId, provider: model.provider, model: model.id, inputTokens: usage.input, outputTokens: usage.output,
      cacheReadTokens: usage.cacheRead ?? 0, cacheWriteTokens: usage.cacheWrite ?? 0, costMicros: costMicros(model, usage),
    });
  } catch (error) {
    console.error('usage record failed', (error as Error).message);
  } finally {
    await data.end();
  }
}

export const aiRoutes = new Hono<Env>()
  .get('/models', async (c) => {
    const s = requireSession(c);
    const { ent } = await loadAccount(c.var.data.db, s.userId);
    const data = MODEL_PROVIDERS.flatMap(p => ent.models?.[p].models ?? []).map(id => ({ id, object: 'model' as const, owned_by: MODELS[id]!.provider, name: MODELS[id]!.label }));
    return c.json({ object: 'list' as const, data });
  })
  .post('/chat/completions', async (c) => {
    const s = requireSession(c);
    const length = Number(c.req.header('content-length'));
    if (Number.isFinite(length) && length > MAX_BODY_BYTES) throw new ApiError({ error: 'too_large', limit: 'envelope' });
    const text = await c.req.text();
    if (text.length > MAX_BODY_BYTES) throw new ApiError({ error: 'too_large', limit: 'envelope' });
    let body: Record<string, unknown>;
    try { body = JSON.parse(text); } catch { throw new ApiError({ error: 'bad_request', detail: 'body must be JSON' }); }
    if (!body || typeof body !== 'object' || Array.isArray(body)) throw new ApiError({ error: 'bad_request' });

    const account = await loadAccount(c.var.data.db, s.userId);
    const candidate = typeof body.model === 'string' ? MODELS[body.model] : undefined;
    const resolved = resolveModel(account.ent, candidate?.provider ?? 'anthropic', body.model);
    if (!resolved.ok) {
      throw new ApiError({ error: 'forbidden', detail: resolved.reason === 'no_hosted_models'
        ? 'Built-in models need Powermove Aerolite.' : `${String(body.model)} is not included in ${account.ent.label}.` });
    }
    const model = resolved.model;
    const spend = await providerSpend(c.var.data.db, s.userId, model.provider, account.windows);
    const budget = checkModelBudget(account.ent, model.provider, spend, account.windows);
    if (!budget.ok) {
      if (budget.resetsAt) {
        c.header('Retry-After', String(Math.max(60, Math.ceil((budget.resetsAt.getTime() - Date.now()) / 1000))));
        c.header('X-Powermove-Limit-Resets', budget.resetsAt.toISOString());
      }
      const which = model.provider === 'anthropic' ? 'Claude' : 'OpenAI';
      throw new ApiError({ error: 'quota_exceeded', detail: budget.reason === 'day_limit'
        ? `You've used today's ${which} allowance. It refills tomorrow.` : `You've used this month's ${which} allowance.` });
    }

    // Never let the client raise its own ceiling past ours.
    for (const key of ['max_tokens', 'max_completion_tokens'] as const) {
      if (typeof body[key] === 'number') body[key] = Math.min(body[key] as number, MAX_OUTPUT_TOKENS);
    }
    if (body.max_tokens === undefined && body.max_completion_tokens === undefined) body.max_completion_tokens = MAX_OUTPUT_TOKENS;
    if (model.provider === 'anthropic' && body.max_completion_tokens !== undefined && body.max_tokens === undefined) {
      body.max_tokens = body.max_completion_tokens; delete body.max_completion_tokens;
    }
    const stream = body.stream === true;
    if (stream) body.stream_options = { ...(body.stream_options as object | undefined), include_usage: true };
    // OpenAI's abuse tooling keys on `user`; send our opaque account id, never the client's value.
    delete body.user;
    if (model.provider === 'openai') body.user = s.userId;

    const target = upstream(c.env, model);
    const fetcher = c.env.AEROLITE_UPSTREAM_FETCH ?? fetch;
    const requestText = JSON.stringify(body);
    const response = await fetcher(target.url, {
      method: 'POST', headers: { 'Content-Type': 'application/json', ...target.headers }, body: requestText, signal: c.req.raw.signal,
    });
    if (!response.ok) {
      const detail = await response.text().catch(() => '');
      console.warn('upstream error', model.id, response.status, detail.slice(0, 300));
      // Pass client errors through (bad tool schema, context too long); hide our own key/quota problems.
      if (response.status === 400 || response.status === 413 || response.status === 422) {
        return new Response(detail, { status: response.status, headers: { 'Content-Type': response.headers.get('content-type') ?? 'application/json' } });
      }
      throw new ApiError({ error: response.status === 429 ? 'rate_limited' : 'internal', detail: 'The model provider is busy. Try again in a moment.' });
    }

    if (!stream || !response.body) {
      const json = await response.json() as { usage?: Parameters<typeof usageFromJson>[0] };
      const usage = usageFromJson(json.usage) ?? estimateUsage(requestText.length, JSON.stringify(json).length);
      await record(c, s.userId, model, usage);
      return c.json(json);
    }
    const userId = s.userId;
    const metered = response.body.pipeThrough(meteredStream(async (usage, bytes) => {
      await record(c, userId, model, usage ?? estimateUsage(requestText.length, bytes));
    }));
    return new Response(metered, { status: 200, headers: { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' } });
  });
