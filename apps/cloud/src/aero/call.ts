/* One Aero call, owned by a Durable Object of its own (named by the request's ledger key), so the call outlives the
   client and the Worker that took the request: the object streams the reply back while anyone listens, reads the
   provider stream to its end regardless, and settles the call's hold. Everything the meter learns is in the
   object's storage, and its alarm checks on the call: while the stream runs it waits, and if the object died
   mid-call (an eviction, a deploy) it settles from storage, retrying with backoff until the spend is written. */
import { streamText } from 'ai';
import type { Data } from '../db/client';
import { dataFor } from '../db/client';
import { modelById } from './catalogue';
import { ALARM_AFTER_MS, type CallStore, Meter, retryDelay, settleStored } from './meter';
import { endpointFor, languageModel } from './providers';
import { ChatRequest, UnsupportedRequest, prepare } from './translate';
import { ChunkEncoder } from './sse';
import { V2Error } from '../v2-error';

/** What the Worker hands the object: the client's request, already validated, and who it is for. */
export interface CallStart { request: unknown; userId: string; requestId: string; key: string; cacheKey: string }

const refuse = (error: V2Error) => Response.json({ error: { code: error.code } }, { status: error.status });

/** The Durable Object state the call uses (a subset of DurableObjectState). */
export interface CallState { storage: CallStore & { get<T = unknown>(key: string): Promise<T | undefined> }; waitUntil?(promise: Promise<unknown>): void }

export class AeroCall {
  /** Whether this instance is reading the call's stream (memory only: an object that died has lost it). */
  private running = false;

  constructor(private readonly ctx: CallState, private readonly env: CloudflareBindings, private readonly data: (env: CloudflareBindings) => Data = dataFor) {}

  async fetch(request: Request): Promise<Response> {
    try { return await this.start(request); } catch (error) {
      if (error instanceof V2Error) return refuse(error);
      throw error;
    }
  }

  private async start(request: Request): Promise<Response> {
    const start = await request.json() as CallStart;
    if (await this.ctx.storage.get('call')) throw new V2Error(409, 'duplicate_request');
    const parsed = ChatRequest.safeParse(start.request);
    if (!parsed.success) throw new V2Error(400, 'invalid_request');
    const model = modelById(parsed.data.model);
    if (!model) throw new V2Error(400, 'unknown_model');
    let prepared;
    try { prepared = prepare(parsed.data, model, start.cacheKey); } catch (error) {
      if (error instanceof UnsupportedRequest) throw new V2Error(400, 'invalid_request');
      throw error;
    }
    const endpoint = endpointFor(this.env, model.provider);
    const data = this.data(this.env);
    const meter = new Meter({ data, store: this.ctx.storage, endpoint, userId: start.userId, key: start.key, model, requestedMax: prepared.maxOutputTokens });
    const encoder = new ChunkEncoder(`chatcmpl-${start.requestId}`, model.id, model.provider);
    const abort = new AbortController();
    const result = streamText({
      model: languageModel(this.env, model, meter.fetch),
      instructions: prepared.instructions,
      messages: prepared.messages,
      tools: prepared.tools,
      toolChoice: prepared.toolChoice,
      providerOptions: prepared.providerOptions,
      maxOutputTokens: prepared.maxOutputTokens,
      temperature: prepared.temperature,
      topP: prepared.topP,
      // Never twice: a retry after an ambiguous failure could be generated (and billed) twice under one hold.
      maxRetries: 0,
      includeRawChunks: true,
      abortSignal: abort.signal,
      onError: ({ error }) => { console.error('aero provider error', start.requestId, (error as Error)?.name ?? 'unknown'); },
    });
    // A client that disconnects stops getting frames; the provider stream is still read to its end (its usage is
    // what the provider bills us, and the meter's output cap bounds it). A write can hang on a gone client
    // instead of failing, so every write also gives way to the request's abort signal.
    const { readable, writable } = new TransformStream<Uint8Array, Uint8Array>();
    const writer = writable.getWriter();
    const bytes = new TextEncoder();
    let clientGone = false;
    const disconnected = new Promise<void>(resolve => { if (request.signal.aborted) resolve(); else request.signal.addEventListener('abort', () => resolve(), { once: true }); });
    void disconnected.then(() => { clientGone = true; });
    const write = async (frame: string) => {
      if (clientGone) return;
      try { await Promise.race([writer.write(bytes.encode(frame)), disconnected]); } catch { clientGone = true; }
    };
    this.running = true;
    const pump = (async () => {
      try {
        try {
          for await (const part of result.fullStream) {
            if (meter.broken) break;
            meter.observe(part);
            for (const frame of encoder.accept(part)) await write(frame);
          }
        } catch { encoder.failed = true; }
        meter.abandon();
        // Storage failed: this object is gone, and its successor's alarm settles the call.
        if (meter.broken) { abort.abort(); return; }
        try { await meter.settle(); } catch (error) {
          console.error('aero settle deferred', start.key, (error as Error)?.name ?? 'unknown');
          await this.ctx.storage.setAlarm(Date.now() + retryDelay(0)).catch(() => {});
        }
        const frames = encoder.usage && !encoder.failed && meter.reported ? encoder.close(encoder.usage) : [ChunkEncoder.error('upstream_error')];
        for (const frame of frames) await write(frame);
        await writer.close().catch(() => {});
      } finally {
        this.running = false;
        await data.end().catch(() => {});
      }
    })();
    this.ctx.waitUntil?.(pump);
    // Answer only once the reservation is made: a balance that can't cover the request is a 402, not a stream.
    try { await meter.admission; } catch (error) {
      clientGone = true;
      abort.abort();
      if (error instanceof V2Error && error.status !== 502) throw error;
    }
    return new Response(readable, { headers: { 'content-type': 'text/event-stream; charset=utf-8', 'cache-control': 'no-cache, no-transform', 'x-accel-buffering': 'no' } });
  }

  /** The call's check-in: wait while its stream runs here; otherwise settle it from storage, or try again later. */
  async alarm(): Promise<void> {
    if (this.running) { await this.ctx.storage.setAlarm(Date.now() + ALARM_AFTER_MS); return; }
    if (!await this.ctx.storage.get('call')) return;
    const data = this.data(this.env);
    try {
      await settleStored(this.ctx.storage, data);
    } catch (error) {
      console.error('aero settle retry', (error as Error)?.name ?? 'unknown');
      await this.ctx.storage.setAlarm(Date.now() + retryDelay((await this.ctx.storage.get<number>('attempts')) ?? 0));
    } finally {
      await data.end().catch(() => {});
    }
  }
}
