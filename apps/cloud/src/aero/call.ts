/* One Aero call, owned by a Durable Object of its own (named by the request's ledger key), so the call outlives the
   client and the Worker that took the request: the object streams the reply back while anyone listens, reads the
   provider stream to its end regardless, and settles the call's hold. Forwarding to the client never holds up
   that drain: frames queue up to a bound, and a client that falls further behind is cut off. Everything the
   meter learns is in the object's storage, and its alarm checks on the call: while the stream runs it waits (up
   to MAX_DRAIN_MS, then stops the drain), and if the object died mid-call (an eviction, a deploy) it settles from
   storage, retrying with backoff until the spend is written. Pending provider I/O keeps the object alive
   (wrangler.jsonc: durable_object_io_tasks_prevent_eviction). */
import { streamText } from 'ai';
import type { Data } from '../db/client';
import { dataFor } from '../db/client';
import { modelById } from './catalogue';
import { ALARM_AFTER_MS, type CallStore, MAX_DRAIN_MS, Meter, retryDelay, settleStored } from './meter';
import { endpointFor, languageModel } from './providers';
import { ChatRequest, UnsupportedRequest, prepare } from './translate';
import { ChunkEncoder } from './sse';
import { V2Error } from '../v2-error';
import { productionProblems } from '../production';

/** What the Worker hands the object: the client's request, already validated, and who it is for. */
export interface CallStart { request: unknown; userId: string; requestId: string; key: string; cacheKey: string }

const refuse = (error: V2Error) => Response.json({ error: { code: error.code } }, { status: error.status });

/** The Durable Object state the call uses (a subset of DurableObjectState). */
export interface CallState { storage: CallStore & { get<T = unknown>(key: string): Promise<T | undefined> }; waitUntil?(promise: Promise<unknown>): void }

/** Frames queued for a slow client before it is cut off. */
export const CLIENT_BUFFER_BYTES = 1 << 20;

export class AeroCall {
  /** The start that owns this object's call (memory only: an object that died has lost it). Claimed before the
      first await, and kept in storage as `start`, so a concurrent or later duplicate never touches the call. */
  private owner: { token: string; abort: AbortController } | null = null;

  constructor(private readonly ctx: CallState, private readonly env: CloudflareBindings, private readonly data: (env: CloudflareBindings) => Data = dataFor) {}

  async fetch(request: Request): Promise<Response> {
    if (productionProblems(this.env).length) return refuse(new V2Error(503, 'misconfigured'));
    if (this.owner) return refuse(new V2Error(409, 'duplicate_request'));
    const owner = { token: crypto.randomUUID(), abort: new AbortController() };
    this.owner = owner;
    const release = () => { if (this.owner === owner) this.owner = null; };
    try { return await this.start(request, owner, release); } catch (error) {
      release();
      if (error instanceof V2Error) return refuse(error);
      throw error;
    }
  }

  private async start(request: Request, owner: { token: string; abort: AbortController }, release: () => void): Promise<Response> {
    const start = await request.json() as CallStart;
    const parsed = ChatRequest.safeParse(start.request);
    if (!parsed.success) throw new V2Error(400, 'invalid_request');
    const model = modelById(parsed.data.model);
    if (!model) throw new V2Error(400, 'unknown_model');
    let prepared;
    try { prepared = prepare(parsed.data, model, start.cacheKey); } catch (error) {
      if (error instanceof UnsupportedRequest) throw new V2Error(400, 'invalid_request');
      throw error;
    }
    endpointFor(this.env, model.provider);
    // Compare-and-set on the start token: a start already recorded here (by an instance that has since died) owns
    // the call, and its alarm settles it.
    if (await this.ctx.storage.get('start')) throw new V2Error(409, 'duplicate_request');
    await this.ctx.storage.put({ start: owner.token });
    const data = this.data(this.env);
    const meter = new Meter({ data, store: this.ctx.storage, userId: start.userId, key: start.key, model, requestedMax: prepared.maxOutputTokens });
    const encoder = new ChunkEncoder(`chatcmpl-${start.requestId}`, model.id, model.provider);
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
      abortSignal: owner.abort.signal,
      onError: ({ error }) => { console.error('aero provider error', start.requestId, (error as Error)?.name ?? 'unknown'); },
    });
    // The drain never waits on the client: frames are queued (never awaited), and a client that lets more than
    // CLIENT_BUFFER_BYTES pile up, or disconnects, is dropped while the provider stream is read to its end.
    const { readable, writable } = new TransformStream<Uint8Array, Uint8Array>(undefined, new ByteLengthQueuingStrategy({ highWaterMark: CLIENT_BUFFER_BYTES }));
    const writer = writable.getWriter();
    const bytes = new TextEncoder();
    let clientGone = false;
    if (request.signal.aborted) clientGone = true;
    else request.signal.addEventListener('abort', () => { clientGone = true; }, { once: true });
    const write = (frame: string) => {
      if (clientGone) return;
      if (writer.desiredSize !== null && writer.desiredSize <= 0) { clientGone = true; writer.abort().catch(() => {}); return; }
      writer.write(bytes.encode(frame)).catch(() => { clientGone = true; });
    };
    const pump = (async () => {
      try {
        try {
          for await (const part of result.fullStream) {
            if (meter.broken) break;
            meter.observe(part);
            for (const frame of encoder.accept(part)) write(frame);
          }
        } catch { encoder.failed = true; }
        meter.abandon();
        // Storage failed: this object is gone, and its successor's alarm settles the call.
        if (meter.broken) { owner.abort.abort(); writer.abort().catch(() => {}); return; }
        try { await meter.settle(); } catch (error) {
          console.error('aero settle deferred', start.key, (error as Error)?.name ?? 'unknown');
          await this.ctx.storage.setAlarm(Date.now() + retryDelay(0)).catch(() => {});
        }
        const frames = encoder.usage && !encoder.failed && meter.reported ? encoder.close(encoder.usage) : [ChunkEncoder.error('upstream_error')];
        for (const frame of frames) write(frame);
        if (!clientGone) writer.close().catch(() => {});
      } finally {
        release();
        await data.end().catch(() => {});
      }
    })();
    this.ctx.waitUntil?.(pump);
    // Answer only once the reservation is made: a balance that can't cover the request is a 402, not a stream.
    try { await meter.admission; } catch (error) {
      clientGone = true;
      owner.abort.abort();
      if (error instanceof V2Error && error.status !== 502) throw error;
    }
    return new Response(readable, { headers: { 'content-type': 'text/event-stream; charset=utf-8', 'cache-control': 'no-cache, no-transform', 'x-accel-buffering': 'no' } });
  }

  /** The call's check-in. While this instance drains the call, wait, up to MAX_DRAIN_MS from dispatch; past that,
      stop the drain (the call then settles at its hold). With no drain here (the object died), settle from
      storage, or try again later. */
  async alarm(): Promise<void> {
    if (this.owner) {
      const call = await this.ctx.storage.get<{ dispatchedAt: number }>('call');
      if (call && Date.now() - call.dispatchedAt > MAX_DRAIN_MS) {
        console.error('aero drain over its limit');
        this.owner.abort.abort();
      }
      await this.ctx.storage.setAlarm(Date.now() + ALARM_AFTER_MS);
      return;
    }
    if (!await this.ctx.storage.get('call') && !await this.ctx.storage.get('intent')) return;
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
