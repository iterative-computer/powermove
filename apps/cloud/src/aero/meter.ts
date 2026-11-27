/* Metering one Aero request: reserve, then settle. The user pays what the provider bills us, and never gets
   more than the period's grant: when the bill is uncertain, the user pays the conservative upper bound (owner,
   2026-11-26: "If anything, we should overcharge the user, not undercharge").

   The call runs in a Durable Object (call.ts), which outlives the client and the Worker; what the meter learns is
   written to that object's storage the moment it learns it, so whatever happens to the stream, the object's alarm
   can finish the job.

   Reserve (in the provider fetch, where the exact provider request is known): under a per-account lock, find the
   current billing period by the database clock (none past its end), bound the request's input tokens from above
   by its bytes (a bound no estimate lowers), cap its output at what the balance allows and hold the most it can
   cost. The holds and spends of a period never pass its grant: a request the balance can't fully cover is
   refused (402). A recovery intent and the object's alarm are recorded before the hold, so no hold is stranded.
   The request goes to the provider once (no retries: an ambiguous lost response is never generated twice under
   one hold).
   Settle: the hold becomes the spend in one conditional update, retried, and a spend never exceeds its hold.
   - The provider's final usage event came: its exact cost.
   - The provider refused the call definitively (a 4xx before any generation, per its API semantics): nothing.
   - Anything else (a 5xx, no final usage event, a stream that failed or was cut, a settle that never happened,
     an object that died mid-call): the whole hold, marked estimated. A hold that never settles counts in full,
     and the scheduled sweep turns it into that spend. */
import { and, eq, sql } from 'drizzle-orm';
import type { LanguageModelUsage, TextStreamPart, ToolSet } from 'ai';
import type { Data } from '../db/client';
import { creditLedger } from '../db/schema';
import { lockBalance, periodBalance, spendingPeriod } from '../billing/ledger';
import { type AeroModel, type Metered, PRICE_VERSION, ceilingRates, cost, modelById } from './catalogue';
import type { Fetch } from './providers';
import { V2Error } from '../v2-error';

/** A hold this old is shown as spent (in full) rather than reserved; its call's object settles it in the end. */
export const HOLD_TTL_MS = 60 * 60 * 1000;
/** The least output a request is let through with: OpenAI's smallest max_output_tokens. */
export const MIN_OUTPUT = 16;
/** Tokens a provider adds to the request beyond its bytes: message framing (the JSON's own punctuation covers
    more), and Anthropic's tool-use system prompt when tools are offered (346 tokens on current models). */
export function promptOverhead(provider: 'openai' | 'anthropic', body: Record<string, unknown>): number {
  const tools = Array.isArray(body.tools) && body.tools.length > 0;
  return provider === 'anthropic' ? 64 + (tools ? 512 : 0) : 64;
}
/** Settle-write attempts in one go before the object's alarm takes over. */
const WRITE_ATTEMPTS = 4;

export type Source = 'reported' | 'hold' | 'none';
type Usage = Metered & { reasoning: number };

/** An upper bound on the provider request's input tokens: one token per UTF-8 byte of the whole request as sent
    (every token of a byte-level tokenizer covers at least one byte), plus the provider's prompt overhead. The one
    exception is a real image part of a message (where the provider reads pixels, not bytes): its base64 counts at
    a bound from its pixel size instead. Anything image-shaped elsewhere (a tool schema, say) is text and counts
    byte for byte. Documents are refused before they get here. */
export function inputBound(provider: 'openai' | 'anthropic', body: Record<string, any>): number {
  let bound = new TextEncoder().encode(JSON.stringify(body)).length + promptOverhead(provider, body);
  const image = (part: any) => {
    if (part?.type === 'document' || part?.type === 'input_file') throw new V2Error(400, 'invalid_request');
    let data: string | null = null;
    if (provider === 'anthropic' && part?.type === 'image' && part.source?.type === 'base64' && typeof part.source.data === 'string') data = part.source.data;
    if (provider === 'openai' && part?.type === 'input_image' && typeof part.image_url === 'string' && part.image_url.startsWith('data:')) data = part.image_url.slice(part.image_url.indexOf(',') + 1);
    // Base64 is plain ASCII that JSON doesn't escape: its bytes in the request are its length.
    if (data !== null) bound += imageBound(data) - data.length;
  };
  const messages: any[] = provider === 'anthropic' ? (Array.isArray(body.messages) ? body.messages : []) : (Array.isArray(body.input) ? body.input : []);
  for (const message of messages) {
    if (!Array.isArray(message?.content)) continue;
    for (const part of message.content) {
      image(part);
      // An Anthropic tool result can carry images of its own.
      if (provider === 'anthropic' && part?.type === 'tool_result' && Array.isArray(part.content)) for (const inner of part.content) image(inner);
    }
  }
  return bound;
}

/** An image's most input tokens (never more than its base64 counts byte for byte): a token per 16x16 pixels plus a constant (Anthropic bills about one per 750 pixels;
    OpenAI at most ~1.6 per 32x32 patch, or 170 per 512x512 tile plus 85). Without readable dimensions, its base64
    length (a provider can't take more tokens from an image than it has bytes, and refuses one it can't decode). */
export function imageBound(base64: string): number {
  const size = imageSize(base64);
  return size ? Math.min(base64.length, Math.ceil(size.width / 16) * Math.ceil(size.height / 16) + 1024) : base64.length;
}

function imageSize(base64: string): { width: number; height: number } | null {
  let bytes: Uint8Array;
  // The headers are near the start: decode the first 256 KiB (a whole number of base64 quads).
  try { bytes = Uint8Array.from(atob(base64.slice(0, 349_524)), c => c.charCodeAt(0)); } catch { return null; }
  const u16 = (i: number) => (bytes[i]! << 8) | bytes[i + 1]!;
  const u32 = (i: number) => ((bytes[i]! << 24) >>> 0) + (bytes[i + 1]! << 16) + (bytes[i + 2]! << 8) + bytes[i + 3]!;
  const le16 = (i: number) => bytes[i]! | (bytes[i + 1]! << 8);
  const le24 = (i: number) => bytes[i]! | (bytes[i + 1]! << 8) | (bytes[i + 2]! << 16);
  if (bytes.length >= 24 && bytes[0] === 0x89 && bytes[1] === 0x50 && bytes[2] === 0x4e && bytes[3] === 0x47) return { width: u32(16), height: u32(20) };
  if (bytes.length >= 10 && bytes[0] === 0x47 && bytes[1] === 0x49 && bytes[2] === 0x46) return { width: le16(6), height: le16(8) };
  if (bytes.length >= 30 && bytes[0] === 0x52 && bytes[1] === 0x49 && bytes[8] === 0x57 && bytes[9] === 0x45) {
    const chunk = String.fromCharCode(bytes[12]!, bytes[13]!, bytes[14]!, bytes[15]!);
    if (chunk === 'VP8X') return { width: le24(24) + 1, height: le24(27) + 1 };
    if (chunk === 'VP8 ') return { width: le16(26) & 0x3fff, height: le16(28) & 0x3fff };
    if (chunk === 'VP8L') { const b = bytes; return { width: 1 + (((b[22]! & 0x3f) << 8) | b[21]!), height: 1 + (((b[24]! & 0x0f) << 10) | (b[23]! << 2) | ((b[22]! & 0xc0) >> 6)) }; }
    return null;
  }
  if (bytes.length >= 4 && bytes[0] === 0xff && bytes[1] === 0xd8) {
    for (let i = 2; i + 9 < bytes.length;) {
      if (bytes[i] !== 0xff) { i++; continue; }
      const marker = bytes[i + 1]!;
      if (marker === 0xff) { i++; continue; }
      if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) { i += 2; continue; }
      if (marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc) return { width: u16(i + 7), height: u16(i + 5) };
      i += 2 + u16(i + 2);
    }
  }
  return null;
}

/** Usage as the AI SDK's provider converters reported it, with the long-TTL cache writes from the raw usage. */
export function meteredOf(usage: LanguageModelUsage): Metered & { reasoning: number } {
  const raw = (usage.raw ?? {}) as Record<string, any>;
  const writeLong = Number(raw.cache_creation?.ephemeral_1h_input_tokens ?? raw.input_tokens_details?.cache_write_tokens_details?.ttl_12h_tokens ?? 0);
  return {
    input: usage.inputTokens ?? 0,
    cacheRead: usage.inputTokenDetails?.cacheReadTokens ?? 0,
    cacheWrite: usage.inputTokenDetails?.cacheWriteTokens ?? 0,
    cacheWriteLong: Number.isFinite(writeLong) ? writeLong : 0,
    output: usage.outputTokens ?? 0,
    reasoning: usage.outputTokenDetails?.reasoningTokens ?? 0,
  };
}

/** Anthropic's raw usage object (message_start's, then message_delta's numbers over it) as Metered. */
export function anthropicRaw(usage: Record<string, any>): Metered & { reasoning: number } {
  const fresh = Number(usage.input_tokens ?? 0), write = Number(usage.cache_creation_input_tokens ?? 0), read = Number(usage.cache_read_input_tokens ?? 0);
  return { input: fresh + write + read, cacheRead: read, cacheWrite: write, cacheWriteLong: Number(usage.cache_creation?.ephemeral_1h_input_tokens ?? 0), output: Number(usage.output_tokens ?? 0), reasoning: Number(usage.output_tokens_details?.thinking_tokens ?? 0) };
}

/** OpenAI's raw usage object (response.completed's) as Metered. */
export function openaiRaw(usage: Record<string, any>): Usage {
  const details = usage.input_tokens_details ?? {};
  return { input: Number(usage.input_tokens ?? 0), cacheRead: Number(details.cached_tokens ?? 0), cacheWrite: Number(details.cache_write_tokens ?? 0), cacheWriteLong: Number(details.cache_write_tokens_details?.ttl_12h_tokens ?? 0), output: Number(usage.output_tokens ?? 0), reasoning: Number(usage.output_tokens_details?.reasoning_tokens ?? 0) };
}

const ZERO: Usage = { input: 0, cacheRead: 0, cacheWrite: 0, cacheWriteLong: 0, output: 0, reasoning: 0 };

/** The Durable Object storage the meter keeps its call in (a subset of DurableObjectStorage). */
export interface CallStore {
  get<T = unknown>(key: string): Promise<T | undefined>;
  put(entries: Record<string, unknown>): Promise<void>;
  deleteAll(): Promise<void>;
  setAlarm(at: number): Promise<void>;
  deleteAlarm(): Promise<void>;
}

/** What the store holds for a call. */
interface Call { userId: string; key: string; model: string; holdNano: number; outputCap: number; dispatchedAt: number }
/** Recorded before the hold is reserved, so an object that dies between the two still finds and bills its hold. */
interface Intent { userId: string; key: string; model: string }
/** Where the call got to: sent; answered with a stream; refused definitively; finished with its final usage. */
type Stage = 'dispatched' | 'opened' | 'refused' | 'complete';
/** A provider answer that is a definitive refusal: a 4xx (the request was rejected before any generation), except
    a timeout, which says nothing about what the provider did. */
export const definitiveRefusal = (status: number) => status >= 400 && status < 500 && status !== 408;
interface Settlement { source: Source; usage: Usage; nanoUsd: number; fresh: number; estimated: boolean }
/** When the object's alarm checks on a call: soon after dispatch, then while it runs, then with backoff. */
export const ALARM_AFTER_MS = 30_000;
/** The longest a call may stream (128k output tokens at a slow 30 a second is about 70 minutes). Past it the
    object's watchdog stops the drain and the call settles at its hold. */
export const MAX_DRAIN_MS = 90 * 60_000;
export const retryDelay = (attempt: number) => Math.min(60_000 * 2 ** attempt, 60 * 60_000);

export interface MeterOptions {
  data: Data;
  store: CallStore;
  userId: string;
  key: string;
  model: AeroModel;
  /** The client's own output limit, if it set one. */
  requestedMax: number | undefined;
  fetch?: Fetch;
}

export class Meter {
  /** Settles with the hold reserved, or a refusal (402, 409) the route answers instead of a stream. */
  readonly admission: Promise<void>;
  private admit!: { resolve: () => void; reject: (error: unknown) => void };
  private dispatched = false;
  private held = false;
  private call: Call | null = null;
  private stage: Stage | null = null;
  private started: Record<string, any> | null = null;
  private delta: Record<string, any> | null = null;
  private final: Usage | null = null;
  private sdk: Usage | null = null;
  /** Store writes, in order. The first failure stops the call (its object is gone). */
  private saving: Promise<void> = Promise.resolve();
  broken: unknown = null;
  private readonly upstream: Fetch;

  constructor(private readonly options: MeterOptions) {
    this.upstream = options.fetch ?? ((input, init) => fetch(input, init));
    this.admission = new Promise((resolve, reject) => { this.admit = { resolve, reject }; });
    this.admission.catch(() => {});
  }

  private save(entries: Record<string, unknown>) {
    this.saving = this.saving.then(() => this.options.store.put(entries)).catch((error) => { this.broken ??= error; throw error; });
    this.saving.catch(() => {});
    return this.saving;
  }

  /** The provider fetch: reserves, records and caps the one generation call. */
  readonly fetch: Fetch = async (input, init) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const path = new URL(url).pathname;
    const generate = this.options.model.provider === 'anthropic' ? /\/messages$/.test(path) : /\/responses$/.test(path);
    if (!generate || typeof init?.body !== 'string') return this.upstream(input, init);
    if (this.dispatched) throw new Error('aero: a call is dispatched once');
    this.dispatched = true;
    const body = JSON.parse(init.body) as Record<string, any>;
    const { store, model, userId, key } = this.options;
    let sent: string;
    try {
      // The recovery intent and the alarm come first: whatever happens after the hold commits, the alarm finds it.
      await this.save({ intent: { userId, key, model: model.id } satisfies Intent });
      await store.setAlarm(Date.now() + ALARM_AFTER_MS);
      try { await this.reserve(body); } catch (error) {
        // Nothing was held: the intent goes (the alarm then finds nothing to do).
        if (error instanceof V2Error) { await store.deleteAll(); await store.deleteAlarm(); }
        throw error;
      }
      const field = model.provider === 'anthropic' ? 'max_tokens' : 'max_output_tokens';
      body[field] = Math.min(typeof body[field] === 'number' ? body[field] : Infinity, this.call!.outputCap);
      sent = JSON.stringify(body);
      this.stage = 'dispatched';
      await this.save({ call: this.call, stage: this.stage });
    } catch (error) { this.admit.reject(error); throw error; }
    this.admit.resolve();
    const response = await this.upstream(input, { ...init, body: sent });
    this.stage = response.ok ? 'opened' : definitiveRefusal(response.status) ? 'refused' : 'dispatched';
    await this.save({ stage: this.stage });
    return response;
  };

  /** Ends a pending admission when the call failed before it reached the provider. */
  abandon() { this.admit.reject(new V2Error(502, 'upstream_error')); }

  private async reserve(body: Record<string, any>): Promise<void> {
    const { model, requestedMax, userId, data } = this.options;
    const desired = Math.min(requestedMax ?? model.maxOutput, model.maxOutput);
    const floor = Math.min(MIN_OUTPUT, desired);
    const bound = inputBound(model.provider, body);
    const rates = ceilingRates(model, bound);
    await data.tx(async (tx) => {
      // Under the account's lock (which period changes take too), the period by the database clock.
      await lockBalance(tx, userId);
      const now = await spendingPeriod(tx, userId);
      if (!now) throw new V2Error(402, 'credits_exhausted');
      const available = (await periodBalance(tx, userId, now.period)).remainingNano;
      const cap = Math.min(desired, Math.floor((available - bound * rates.input) / rates.output));
      if (cap < floor) throw new V2Error(402, 'credits_exhausted');
      const hold = bound * rates.input + cap * rates.output;
      const claimed = await tx.insert(creditLedger).values({
        userId, kind: 'hold', idempotencyKey: this.options.key, credits: 0, subscriptionId: now.subscriptionId,
        periodStart: now.period.start, periodEnd: now.period.end, model: model.id, holdNanoUsd: hold,
        holdExpiresAt: sql`clock_timestamp() + ${`${HOLD_TTL_MS} milliseconds`}::interval`, outputCap: cap, priceVersion: PRICE_VERSION,
      }).onConflictDoNothing({ target: creditLedger.idempotencyKey }).returning({ id: creditLedger.id });
      if (claimed.length === 0) throw new V2Error(409, 'duplicate_request');
      this.call = { userId, key: this.options.key, model: model.id, holdNano: hold, outputCap: cap, dispatchedAt: Date.now() };
    });
    this.held = true;
  }

  get holdNano() { return this.call?.holdNano ?? 0; }
  get outputCap() { return this.call?.outputCap ?? 0; }

  /** Watches the stream and records the usage the moment it is reported: the raw provider events carry it, the
      SDK's finish part its converted form. */
  observe(part: TextStreamPart<ToolSet>) {
    if (!this.held) return;
    if (part.type === 'finish') {
      this.sdk = meteredOf(part.totalUsage);
      if (this.final) void this.save({ sdk: this.sdk });
      return;
    }
    if (part.type !== 'raw') return;
    const raw = part.rawValue as Record<string, any> | null;
    if (!raw || typeof raw !== 'object') return;
    if (this.options.model.provider === 'anthropic') {
      if (raw.type === 'message_start' && raw.message?.usage) {
        this.started = { ...raw.message.usage };
        void this.save({ started: this.started });
      } else if (raw.type === 'message_delta' && raw.usage) {
        // Cumulative, but not final: only message_stop says the message (and its usage) is complete.
        this.delta = { ...this.delta, ...Object.fromEntries(Object.entries(raw.usage).filter(([, v]) => v != null)) };
      } else if (raw.type === 'message_stop' && this.started && this.delta) {
        this.complete(anthropicRaw({ ...this.started, ...this.delta }));
      }
    } else if ((raw.type === 'response.completed' || raw.type === 'response.incomplete') && raw.response?.usage) {
      // A finished response (incomplete: stopped at its output cap). response.failed, even with usage, is not.
      this.complete(openaiRaw(raw.response.usage));
    }
  }

  private complete(final: Usage) {
    this.final = final;
    this.stage = 'complete';
    void this.save({ final, stage: this.stage });
  }

  /** Whether the provider reported the request's final usage (a stream that ended without it was cut off). */
  get reported(): boolean { return this.final !== null; }

  /** Settles the call: the hold becomes its spend. Throws if it can't yet (the object's alarm retries). */
  async settle(): Promise<void> {
    if (!this.held) return;
    await this.saving;
    await settleStored(this.options.store, this.options.data);
  }
}

/** Settles a call from what its object stored: the live stream's end, or the alarm after the object died. Throws
    when the database write fails; the caller retries later. */
export async function settleStored(store: CallStore, data: Data): Promise<'settled' | 'none'> {
  const call = await store.get<Call>('call');
  if (!call) {
    // Killed between its intent and its record: if its hold committed, it is billed in full.
    const intent = await store.get<Intent>('intent');
    if (intent) {
      const [row] = await data.db.select({ hold: creditLedger.holdNanoUsd }).from(creditLedger).where(and(eq(creditLedger.idempotencyKey, intent.key), eq(creditLedger.kind, 'hold')));
      if (row) await writeSpend(data, { ...intent, holdNano: row.hold, outputCap: 0, dispatchedAt: 0 }, { source: 'hold', usage: ZERO, nanoUsd: row.hold, fresh: 0, estimated: true });
    }
    await store.deleteAlarm();
    await store.deleteAll();
    return 'none';
  }
  const model = modelById(call.model)!;
  const [stage, final, sdk] = await Promise.all([store.get<Stage>('stage'), store.get<Usage>('final'), store.get<Usage>('sdk')]);
  let settlement: Settlement;
  if (stage === 'complete' && final) {
    // Complete: the SDK's converted usage when it came (the converters the tests check), else the raw report. The
    // hold bounds the input by its bytes and the output by its cap, so the report can't pass it; a spend that
    // would is still held to it (a period never passes its grant) and logged.
    const usage = sdk ?? final;
    const price = cost(model, usage);
    if (price.nanoUsd > call.holdNano) console.error('aero spend over its hold', call.key, price.nanoUsd, call.holdNano);
    settlement = { source: 'reported', usage, nanoUsd: Math.min(price.nanoUsd, call.holdNano), fresh: price.fresh, estimated: false };
  } else if (stage === 'refused') {
    settlement = { source: 'none', usage: ZERO, nanoUsd: 0, fresh: 0, estimated: false };
  } else {
    settlement = { source: 'hold', usage: ZERO, nanoUsd: call.holdNano, fresh: 0, estimated: true };
  }
  const attempts = (await store.get<number>('attempts')) ?? 0;
  await store.put({ attempts: attempts + 1 });
  await writeSpend(data, call, settlement);
  await store.deleteAlarm();
  await store.deleteAll();
  return 'settled';
}

/** The hold becomes the spend in one conditional update (only a hold changes; a settled row stays as it is),
    retried with backoff. */
async function writeSpend(data: Data, call: Call, s: Settlement) {
  let lastError: unknown;
  for (let attempt = 0; attempt < WRITE_ATTEMPTS; attempt++) {
    try {
      const updated = await data.db.update(creditLedger).set({
        kind: 'spend', costNanoUsd: s.nanoUsd, estimated: s.estimated, usageSource: s.source,
        inputTokens: s.usage.input, freshInputTokens: s.fresh, cachedInputTokens: s.usage.cacheRead, cacheWriteTokens: s.usage.cacheWrite,
        cacheWriteLongTokens: s.usage.cacheWriteLong, outputTokens: s.usage.output, reasoningTokens: s.usage.reasoning,
      }).where(and(eq(creditLedger.idempotencyKey, call.key), eq(creditLedger.userId, call.userId), eq(creditLedger.kind, 'hold'))).returning({ id: creditLedger.id });
      if (updated.length === 0) console.info('aero spend already settled', call.key);
      return;
    } catch (error) {
      lastError = error;
      await new Promise(resolve => setTimeout(resolve, 100 * 2 ** attempt));
    }
  }
  throw lastError;
}
