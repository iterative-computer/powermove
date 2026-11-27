import { Hono, type Context } from 'hono';
import { streamText, type LanguageModel } from 'ai';
import { createAnthropic } from '@ai-sdk/anthropic';
import { createOpenAI } from '@ai-sdk/openai';
import type { Env } from '../env';
import { spendable } from '../billing/ledger';
import { consume } from '../abuse';
import { type AeroModel, MODELS, modelById } from '../aero/catalogue';
import { Meter } from '../aero/meter';
import { ChatRequest, UnsupportedRequest, prepare } from '../aero/translate';
import { ChunkEncoder } from '../aero/sse';
import { V2Error } from '../v2-error';

const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '[::1]']);
/** A development base URL (the AI simulator). Only loopback hosts, so a provider key can't leave for anywhere else. */
function localBase(raw: string | undefined): string | undefined {
  if (!raw) return undefined;
  const url = new URL(raw);
  if (!LOCAL_HOSTS.has(url.hostname)) throw new V2Error(503, 'inference_unavailable');
  return raw.replace(/\/$/, '');
}

/** The provider model, every call through the meter's fetch (it reserves and caps the request). */
function languageModel(env: CloudflareBindings, model: AeroModel, meter: Meter): LanguageModel {
  if (model.provider === 'openai') {
    if (!env.OPENAI_API_KEY) throw new V2Error(503, 'inference_unavailable');
    return createOpenAI({ apiKey: env.OPENAI_API_KEY, baseURL: localBase(env.AERO_OPENAI_BASE_URL), fetch: meter.fetch as typeof fetch }).responses(model.providerModel);
  }
  if (!env.ANTHROPIC_API_KEY) throw new V2Error(503, 'inference_unavailable');
  return createAnthropic({ apiKey: env.ANTHROPIC_API_KEY, baseURL: localBase(env.AERO_ANTHROPIC_BASE_URL), fetch: meter.fetch as typeof fetch })(model.providerModel);
}

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
    const parsed = ChatRequest.safeParse(await c.req.json().catch(() => null));
    if (!parsed.success) throw new V2Error(400, 'invalid_request');
    const request = parsed.data;
    const model = modelById(request.model);
    if (!model) throw new V2Error(400, 'unknown_model');
    const rate = await consume(c.var.data.db, 'aero_user', userId);
    if (!rate.ok) throw new V2Error(429, 'rate_limited', rate.retryAfterSeconds);
    const { remainingNano, subscriptionId, period } = await spendable(c.var.data.db, userId);
    if (remainingNano <= 0 || !subscriptionId || !period) throw new V2Error(402, 'credits_exhausted');
    let prepared;
    try { prepared = prepare(request, model, await cacheKey(c, userId, request)); } catch (error) {
      if (error instanceof UnsupportedRequest) throw new V2Error(400, 'invalid_request');
      throw error;
    }
    const clientId = c.req.header('X-Powermove-Request-Id');
    if (clientId !== undefined && !REQUEST_ID.test(clientId)) throw new V2Error(400, 'invalid_request');
    const requestId = clientId ?? c.var.requestId;
    // The request's one ledger row: a hold claimed (with the reservation) before the provider sees the call, so a
    // replayed request id is refused, never served twice.
    const key = `request:${userId}:${requestId}`;
    const meter = new Meter({ data: c.var.data, ledger: c.var.openData(), userId, key, model, subscriptionId, period, requestedMax: prepared.maxOutputTokens });
    const languageModelFor = languageModel(c.env, model, meter);
    const encoder = new ChunkEncoder(`chatcmpl-${requestId}`, model.id, model.provider);
    const abort = new AbortController();
    const keepAlive = (promise: Promise<void>) => { try { c.executionCtx.waitUntil(promise); } catch { /* no execution context in tests: the task just runs */ } };

    const result = streamText({
      model: languageModelFor,
      instructions: prepared.instructions,
      messages: prepared.messages,
      tools: prepared.tools,
      toolChoice: prepared.toolChoice,
      providerOptions: prepared.providerOptions,
      maxOutputTokens: prepared.maxOutputTokens,
      temperature: prepared.temperature,
      topP: prepared.topP,
      maxRetries: 1,
      includeRawChunks: true,
      abortSignal: abort.signal,
      onError: ({ error }) => { console.error('aero provider error', requestId, (error as Error)?.name ?? 'unknown'); },
    });
    // The pump runs as its own task (waitUntil on Workers), so it outlives a client that goes away. A client that
    // disconnects stops getting frames, but the provider stream is read to its end: its reported usage is what the
    // provider bills us, and the meter's output cap bounds it.
    const { readable, writable } = new TransformStream<Uint8Array, Uint8Array>();
    const writer = writable.getWriter();
    const bytes = new TextEncoder();
    let clientGone = false;
    // Workers signal a client that disconnected on the request (enable_request_signal); a write can then hang
    // instead of failing, so every write also gives way to that signal.
    const requestSignal = c.req.raw.signal;
    const disconnected = new Promise<void>(resolve => { if (requestSignal.aborted) resolve(); else requestSignal.addEventListener('abort', () => resolve(), { once: true }); });
    void disconnected.then(() => { clientGone = true; });
    const write = async (frame: string) => {
      if (clientGone) return;
      try { await Promise.race([writer.write(bytes.encode(frame)), disconnected]); } catch { clientGone = true; }
    };
    keepAlive((async () => {
      try {
        for await (const part of result.fullStream) {
          meter.observe(part);
          for (const frame of encoder.accept(part)) await write(frame);
        }
      } catch { encoder.failed = true; }
      meter.abandon();
      await meter.settle();
      if (clientGone) return;
      const frames = encoder.usage && !encoder.failed && meter.reported ? encoder.close(encoder.usage) : [ChunkEncoder.error('upstream_error')];
      for (const frame of frames) await write(frame);
      await writer.close().catch(() => {});
    })());
    // Answer only once the reservation is made: a balance that can't cover the request is a 402, not a stream.
    try { await meter.admission; } catch (error) {
      clientGone = true;
      abort.abort();
      if (error instanceof V2Error && error.status !== 502) throw error;
    }
    c.header('Content-Type', 'text/event-stream; charset=utf-8');
    c.header('Cache-Control', 'no-cache, no-transform');
    c.header('X-Accel-Buffering', 'no');
    c.header('X-Powermove-Request-Id', requestId);
    return c.body(readable);
  });

