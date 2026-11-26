import { Hono, type Context } from 'hono';
import { and, eq } from 'drizzle-orm';
import { streamText, type LanguageModel } from 'ai';
import { createAnthropic } from '@ai-sdk/anthropic';
import { createOpenAI } from '@ai-sdk/openai';
import type { Env } from '../env';
import type { Data } from '../db/client';
import { creditLedger } from '../db/schema';
import { spendable } from '../billing/ledger';
import { consume } from '../abuse';
import { type AeroModel, MODELS, cost, modelById } from '../aero/catalogue';
import { ChatRequest, UnsupportedRequest, prepare } from '../aero/translate';
import { ChunkEncoder, type Usage } from '../aero/sse';
import { V2Error } from '../v2-error';

const LOCAL_HOSTS = new Set(['localhost', '127.0.0.1', '[::1]']);
/** A development base URL (the AI simulator). Only loopback hosts, so a provider key can't leave for anywhere else. */
function localBase(raw: string | undefined): string | undefined {
  if (!raw) return undefined;
  const url = new URL(raw);
  if (!LOCAL_HOSTS.has(url.hostname)) throw new V2Error(503, 'inference_unavailable');
  return raw.replace(/\/$/, '');
}

function languageModel(env: CloudflareBindings, model: AeroModel): LanguageModel {
  if (model.provider === 'openai') {
    if (!env.OPENAI_API_KEY) throw new V2Error(503, 'inference_unavailable');
    return createOpenAI({ apiKey: env.OPENAI_API_KEY, baseURL: localBase(env.AERO_OPENAI_BASE_URL) }).responses(model.providerModel);
  }
  if (!env.ANTHROPIC_API_KEY) throw new V2Error(503, 'inference_unavailable');
  return createAnthropic({ apiKey: env.ANTHROPIC_API_KEY, baseURL: localBase(env.AERO_ANTHROPIC_BASE_URL) })(model.providerModel);
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

/** A rough token count (four characters a token) for a stream cut off before the provider reported usage. */
const estimate = (chars: number) => Math.ceil(chars / 4);

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
    const { remaining, subscriptionId } = await spendable(c.var.data.db, userId);
    if (remaining <= 0) throw new V2Error(402, 'credits_exhausted');
    let prepared;
    try { prepared = prepare(request, model, await cacheKey(c, userId, request)); } catch (error) {
      if (error instanceof UnsupportedRequest) throw new V2Error(400, 'invalid_request');
      throw error;
    }
    const languageModelFor = languageModel(c.env, model);
    const clientId = c.req.header('X-Powermove-Request-Id');
    if (clientId !== undefined && !REQUEST_ID.test(clientId)) throw new V2Error(400, 'invalid_request');
    const requestId = clientId ?? c.var.requestId;
    // The request's one ledger row, claimed before the call: a replayed request id is refused, never served twice.
    const key = `request:${userId}:${requestId}`;
    const claimed = await c.var.data.db.insert(creditLedger).values({ userId, kind: 'spend', idempotencyKey: key, credits: 0, subscriptionId, model: model.id, estimated: true })
      .onConflictDoNothing({ target: creditLedger.idempotencyKey }).returning({ id: creditLedger.id });
    if (claimed.length === 0) throw new V2Error(409, 'duplicate_request');

    const promptChars = JSON.stringify([prepared.instructions, prepared.messages, request.tools ?? null]).length;
    const ledger: Data = c.var.openData();
    const abort = new AbortController();
    const encoder = new ChunkEncoder(`chatcmpl-${requestId}`, model.id, model.provider);
    let settled: Promise<void> | null = null;
    /** Writes the final numbers into the claimed row, once. Reported usage when the provider sent it, else an estimate. */
    const settle = (usage: Usage | null) => {
      settled ??= (async () => {
        const known = usage ?? (encoder.streamedChars > 0 ? { input: estimate(promptChars), cacheRead: 0, cacheWrite: 0, output: estimate(encoder.streamedChars), reasoning: 0 } : null);
        const metered = known ?? { input: 0, cacheRead: 0, cacheWrite: 0, output: 0, reasoning: 0 };
        const price = cost(model, metered);
        try {
          await ledger.db.update(creditLedger).set({
            credits: price.credits, costMicroUsd: price.microUsd, estimated: usage === null && known !== null,
            inputTokens: metered.input, cachedInputTokens: metered.cacheRead, cacheWriteTokens: metered.cacheWrite, outputTokens: metered.output,
          }).where(and(eq(creditLedger.idempotencyKey, key), eq(creditLedger.userId, userId)));
        } catch (error) {
          console.error('aero ledger write failed', requestId, (error as Error).name);
        } finally {
          await ledger.end();
        }
      })();
      return settled;
    };
    const keepAlive = (promise: Promise<void>) => { try { c.executionCtx.waitUntil(promise); } catch { /* no execution context in tests */ } };

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
      abortSignal: abort.signal,
      onError: ({ error }) => { console.error('aero provider error', requestId, (error as Error)?.name ?? 'unknown'); },
    });
    const parts = result.fullStream[Symbol.asyncIterator]();
    const text = new TextEncoder();
    let done = false;
    const body = new ReadableStream<Uint8Array>({
      async pull(controller) {
        if (done) return;
        const next = await parts.next().catch(() => ({ done: true as const, value: undefined }));
        if (next.done) {
          done = true;
          if (encoder.usage && !encoder.failed) {
            await settle(encoder.usage);
            for (const frame of encoder.close(encoder.usage)) controller.enqueue(text.encode(frame));
          } else {
            await settle(encoder.usage);
            controller.enqueue(text.encode(ChunkEncoder.error(abort.signal.aborted ? 'aborted' : 'upstream_error')));
          }
          controller.close();
          return;
        }
        for (const frame of encoder.accept(next.value)) controller.enqueue(text.encode(frame));
      },
      cancel() {
        // The client went away: stop the provider call and settle with what is known.
        done = true;
        abort.abort();
        void parts.return?.();
        keepAlive(settle(encoder.usage));
      },
    });
    c.header('Content-Type', 'text/event-stream; charset=utf-8');
    c.header('Cache-Control', 'no-cache, no-transform');
    c.header('X-Accel-Buffering', 'no');
    c.header('X-Powermove-Request-Id', requestId);
    return c.body(body);
  });

