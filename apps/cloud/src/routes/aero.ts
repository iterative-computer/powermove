import { Hono, type Context } from 'hono';
import type { Env } from '../env';
import { spendable } from '../billing/ledger';
import { clientIp, consume } from '../abuse';
import { AERO_BODY_BYTES, readJsonLimited } from '../body';
import { MODELS, modelById } from '../aero/catalogue';
import type { CallStart } from '../aero/call';
import { endpointFor } from '../aero/providers';
import { pricingOf } from '../aero/pricing';
import { ChatRequest, UnsupportedRequest, prepare } from '../aero/translate';
import { V2Error } from '../v2-error';

async function sha256(text: string): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text));
  return Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0')).join('');
}

/** The thread's prompt cache key, scoped to the account. The client may name its thread (`prompt_cache_key` or
    `X-Powermove-Thread`); otherwise the thread's opening (system prompt and first user turn) names it, which no
    later turn changes. */
async function cacheKey(c: Context<Env>, userId: string, request: ChatRequest): Promise<string> {
  const named = request.prompt_cache_key ?? c.req.header('X-Powermove-Thread');
  const opening = named ?? JSON.stringify([request.messages.find(m => m.role === 'system' || m.role === 'developer')?.content ?? null, request.messages.find(m => m.role === 'user')?.content ?? null]);
  return `pm-${(await sha256(`${userId}\n${opening}`)).slice(0, 40)}`;
}

const REQUEST_ID = /^[A-Za-z0-9._:-]{8,128}$/;

export const aero = new Hono<Env>()
  // Aero (and only Aero) needs the pricing secret: without a usable one every Aero route answers 503 `misconfigured`.
  .use('*', async (c, next) => { pricingOf(c.env); await next(); })
  .get('/models', (c) => {
    if (!c.var.session) throw new V2Error(401, 'unauthorized');
    return c.json({
      data: MODELS.map(model => ({ id: model.id, name: model.name, efforts: model.efforts, defaultEffort: model.defaultEffort, context: model.context, ...(model.default && { default: true }) })),
    });
  })
  .post('/chat/completions', async (c) => {
    const session = c.var.session;
    if (!session) throw new V2Error(401, 'unauthorized');
    const userId = session.userId;
    // Who, how often and whether the plan can pay come first; only then is the body read (capped) and parsed.
    const edge = await c.env.RL_AERO?.limit({ key: clientIp(c) });
    if (edge && !edge.success) throw new V2Error(429, 'rate_limited', 60);
    const rate = await consume(c.var.data.db, 'aero_user', userId);
    if (!rate.ok) throw new V2Error(429, 'rate_limited', rate.retryAfterSeconds);
    const { remainingNano, subscriptionId, period } = await spendable(c.var.data.db, userId, pricingOf(c.env).creditNanoUsd);
    if (remainingNano <= 0 || !subscriptionId || !period) throw new V2Error(402, 'credits_exhausted');
    const parsed = ChatRequest.safeParse(await readJsonLimited(c, AERO_BODY_BYTES));
    if (!parsed.success) throw new V2Error(400, 'invalid_request');
    const request = parsed.data;
    const model = modelById(request.model);
    if (!model) throw new V2Error(400, 'unknown_model');
    const thread = await cacheKey(c, userId, request);
    // Validated here so a bad request is a 400 before any call starts (the call's object prepares it again).
    try { prepare(request, model, thread); } catch (error) {
      if (error instanceof UnsupportedRequest) throw new V2Error(400, 'invalid_request');
      throw error;
    }
    const clientId = c.req.header('X-Powermove-Request-Id');
    if (clientId !== undefined && !REQUEST_ID.test(clientId)) throw new V2Error(400, 'invalid_request');
    const requestId = clientId ?? c.var.requestId;
    // The call runs in a Durable Object of its own, named by the request's one ledger row: it reserves, calls the
    // provider once, and settles, whatever happens to this Worker or the client.
    const key = `request:${userId}:${requestId}`;
    const calls = c.env.AERO_CALLS;
    if (!calls) throw new V2Error(503, 'inference_unavailable');
    endpointFor(c.env, model.provider);
    const start: CallStart = { request, userId, requestId, key, cacheKey: thread };
    const response = await calls.get(calls.idFromName(key)).fetch('https://aero-call/start', { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(start), signal: c.req.raw.signal });
    if (!response.ok) return new Response(response.body, { status: response.status, headers: { 'content-type': 'application/json' } });
    c.header('Content-Type', 'text/event-stream; charset=utf-8');
    c.header('Cache-Control', 'no-cache, no-transform');
    c.header('X-Accel-Buffering', 'no');
    c.header('X-Powermove-Request-Id', requestId);
    return c.body(response.body as ReadableStream);
  });

