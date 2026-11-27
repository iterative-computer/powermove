/* Metering one Aero request: reserve, then settle. A request never spends more than the period's grant, and
   when its usage is uncertain it is charged the conservative upper bound.

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
import { and, eq, inArray, sql } from 'drizzle-orm';
import type { LanguageModelUsage, TextStreamPart, ToolSet } from 'ai';
import type { Data } from '../db/client';
import { aeroBlocks, creditLedger } from '../db/schema';
import { lockBalance, periodBalance, spendingPeriod } from '../billing/ledger';
import { type AeroModel, type Metered, modelById } from './catalogue';
import { type Pricing, ceilingRates, cost } from './pricing';
import type { Emit } from '../telemetry';
import type { Fetch } from './providers';
import { imageTokens } from './images';
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
    if (data !== null) { const tokens = imageTokens(provider, data); bound += (tokens ?? data.length) - data.length; }
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
type Stage = 'dispatched' | 'opened' | 'refused' | 'complete' | 'cancelled';
/** A provider answer that is a definitive refusal: a 4xx (the request was rejected before any generation), except
    a timeout, which says nothing about what the provider did. */
export const definitiveRefusal = (status: number) => status >= 400 && status < 500 && status !== 408;
interface Settlement { source: Source; usage: Usage; nanoUsd: number; fresh: number; estimated: boolean }
/** When the object's alarm checks on a call: soon after dispatch, then while it runs, then with backoff. */
export const ALARM_AFTER_MS = 30_000;
/** The longest a call may run, from dispatch. Past it the object stops the drain (and its watchdog alarm makes
    sure), and the call settles at its whole hold. */
export const MAX_DRAIN_MS = 15 * 60_000;
/** A call's deadlines: admission (the hold reserved and the call sent), upstream silence, and the whole call. Past
    any of them the call stops; a call already sent settles at its whole hold (its cost is uncertain). Mutable for
    tests only. */
export const DEADLINES = { admissionMs: 10_000, idleMs: 60_000, totalMs: MAX_DRAIN_MS };
/** Calls one account may have in flight. */
export const MAX_IN_FLIGHT = 3;
export const retryDelay = (attempt: number) => Math.min(60_000 * 2 ** attempt, 60 * 60_000);

export interface MeterOptions {
  data: Data;
  store: CallStore;
  userId: string;
  key: string;
  model: AeroModel;
  /** The client's own output limit, if it set one. */
  requestedMax: number | undefined;
  /** The most Aero may spend across all accounts in a UTC day, in nano-USD. */
  globalDailyNano: number;
  /** What calls cost (the AERO_PRICING secret). */
  pricing: Pricing;
  /** Server analytics (telemetry.ts): best-effort, never blocking. */
  emit?: Emit;
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
    const { store, model, userId, key } = this.options;
    // Fails closed: nothing leaves for the provider unless it is the one metered generation call, as metered.
    if (!generate || (init?.method ?? 'GET').toUpperCase() !== 'POST' || typeof init?.body !== 'string') throw new Error('aero: unmetered provider request refused');
    if (this.dispatched) throw new Error('aero: a call is dispatched once');
    this.dispatched = true;
    let body: Record<string, any>;
    try { body = JSON.parse(init.body) as Record<string, any>; } catch { throw new Error('aero: unmetered provider request refused'); }
    if (body.model !== model.providerModel || body.stream !== true) throw new Error('aero: unmetered provider request refused');
    let sent!: string;
    let done!: () => void;
    this.reserving = new Promise<void>(resolve => { done = resolve; });
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
      // Past the admission deadline the call is never sent: nothing was billed, so the hold settles at nothing.
      if (this.cancelled) { this.stage = 'cancelled'; await this.save({ call: this.call, stage: this.stage }); throw new V2Error(503, 'upstream_timeout'); }
      this.stage = 'dispatched';
      await this.save({ call: this.call, stage: this.stage });
    } catch (error) { this.admit.reject(error); throw error; } finally { done(); }
    this.admit.resolve();
    const response = await this.upstream(input, { ...init, body: sent });
    this.stage = response.ok ? 'opened' : definitiveRefusal(response.status) ? 'refused' : 'dispatched';
    await this.save({ stage: this.stage });
    return response;
  };

  private cancelled = false;
  /** The reservation in flight (a settle waits for it: a hold may commit after the stream already ended). */
  private reserving: Promise<void> | null = null;
  /** The admission deadline passed: the call must not be sent (a hold still being reserved settles at nothing). */
  cancel() { this.cancelled = true; this.admit.reject(new V2Error(503, 'upstream_timeout')); }

  /** Ends a pending admission when the call failed before it reached the provider. */
  abandon() { this.admit.reject(new V2Error(502, 'upstream_error')); }

  private async reserve(body: Record<string, any>): Promise<void> {
    const { model, requestedMax, userId, data } = this.options;
    const desired = Math.min(requestedMax ?? model.maxOutput, model.maxOutput);
    const floor = Math.min(MIN_OUTPUT, desired);
    const bound = inputBound(model.provider, body);
    const rates = ceilingRates(this.options.pricing, model, bound);
    await data.tx(async (tx) => {
      // Under the account's lock (which period changes take too), the period by the database clock.
      await lockBalance(tx, userId);
      const now = await spendingPeriod(tx, userId);
      if (!now) throw new V2Error(402, 'credits_exhausted');
      // An account under review (a settled cost once passed its hold) gets nothing until someone clears it.
      const [blocked] = await tx.select({ userId: aeroBlocks.userId }).from(aeroBlocks).where(eq(aeroBlocks.userId, userId)).limit(1);
      if (blocked) throw new V2Error(403, 'account_review');
      // At most MAX_IN_FLIGHT calls of one account at a time.
      const [{ running }] = await tx.select({ running: sql<number>`count(*)::int` }).from(creditLedger)
        .where(and(eq(creditLedger.userId, userId), eq(creditLedger.kind, 'hold'), sql`${creditLedger.holdExpiresAt} > clock_timestamp()`)) as [{ running: number }];
      if (running >= MAX_IN_FLIGHT) throw new V2Error(429, 'too_many_calls', 5);
      const available = (await periodBalance(tx, userId, now.period, this.options.pricing.creditNanoUsd)).remainingNano;
      const cap = Math.min(desired, Math.floor((available - bound * rates.input) / rates.output));
      if (cap < floor) throw new V2Error(402, 'credits_exhausted');
      const hold = bound * rates.input + cap * rates.output;
      // Across every account, a day's spend (and holds) stays under the global ceiling, taken under one lock.
      await tx.execute(sql`select pg_advisory_xact_lock(hashtext('aero-global-daily'))`);
      const [{ today }] = await tx.select({ today: sql<string>`coalesce(sum(case when ${creditLedger.kind} = 'spend' then ${creditLedger.costNanoUsd} else ${creditLedger.holdNanoUsd} end), 0)` }).from(creditLedger)
        .where(and(inArray(creditLedger.kind, ['spend', 'hold']), sql`${creditLedger.createdAt} >= date_trunc('day', clock_timestamp() at time zone 'UTC') at time zone 'UTC'`)) as [{ today: string }];
      if (Number(today) + hold > this.options.globalDailyNano) {
        console.error('aero global daily ceiling reached', Number(today), hold, this.options.globalDailyNano);
        this.options.emit?.('aero_ceiling_reached', userId, { today_usd: Number(today) / 1e9, hold_usd: hold / 1e9, ceiling_usd: this.options.globalDailyNano / 1e9, model: model.id });
        throw new V2Error(503, 'capacity_exhausted', 3600);
      }
      const claimed = await tx.insert(creditLedger).values({
        userId, kind: 'hold', idempotencyKey: this.options.key, credits: 0, subscriptionId: now.subscriptionId,
        periodStart: now.period.start, periodEnd: now.period.end, model: model.id, holdNanoUsd: hold,
        holdExpiresAt: sql`clock_timestamp() + ${`${HOLD_TTL_MS} milliseconds`}::interval`, outputCap: cap, priceVersion: this.options.pricing.version,
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
    await this.reserving;
    if (!this.held) return;
    await this.saving;
    await settleStored(this.options.store, this.options.data, this.options.pricing, this.options.emit);
  }
}

/** Settles a call from what its object stored: the live stream's end, or the alarm after the object died. Throws
    when the database write fails; the caller retries later. */
export async function settleStored(store: CallStore, data: Data, pricing: Pricing, emit?: Emit): Promise<'settled' | 'none'> {
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
    const price = cost(pricing, model, usage);
    if (price.nanoUsd > call.holdNano) {
      // The circuit breaker: a bound that didn't hold stops the account until someone reviews it.
      console.error('aero spend over its hold: account blocked for review', call.key, price.nanoUsd, call.holdNano);
      const blocked = await data.db.insert(aeroBlocks).values({ userId: call.userId, reason: `spend ${price.nanoUsd} over hold ${call.holdNano}`, key: call.key }).onConflictDoNothing().returning({ userId: aeroBlocks.userId });
      if (blocked.length) emit?.('aero_blocked', call.userId, { model: call.model, cost_usd: price.nanoUsd / 1e9, hold_usd: call.holdNano / 1e9 });
    }
    settlement = { source: 'reported', usage, nanoUsd: Math.min(price.nanoUsd, call.holdNano), fresh: price.fresh, estimated: false };
  } else if (stage === 'refused' || stage === 'cancelled') {
    settlement = { source: 'none', usage: ZERO, nanoUsd: 0, fresh: 0, estimated: false };
  } else {
    settlement = { source: 'hold', usage: ZERO, nanoUsd: call.holdNano, fresh: 0, estimated: true };
  }
  const attempts = (await store.get<number>('attempts')) ?? 0;
  await store.put({ attempts: attempts + 1 });
  const wrote = await writeSpend(data, call, settlement);
  // Once per call: only the write that turned the hold into its spend reports it. Usage facts only, no content.
  if (wrote && emit) {
    const u = settlement.usage;
    emit('aero_call_settled', call.userId, {
      model: call.model, provider: model.provider, usage_source: settlement.source, estimated: settlement.estimated,
      input_tokens: u.input, fresh_input_tokens: settlement.fresh, cache_read_tokens: u.cacheRead, cache_write_tokens: u.cacheWrite,
      cache_write_long_tokens: u.cacheWriteLong, output_tokens: u.output, reasoning_tokens: u.reasoning,
      cache_hit_ratio: u.input > 0 ? u.cacheRead / u.input : 0, cost_usd: settlement.nanoUsd / 1e9, credits: settlement.nanoUsd / pricing.creditNanoUsd,
      duration_ms: call.dispatchedAt ? Date.now() - call.dispatchedAt : null,
    });
  }
  await store.deleteAlarm();
  await store.deleteAll();
  return 'settled';
}

/** The hold becomes the spend in one conditional update (only a hold changes; a settled row stays as it is),
    retried with backoff. */
async function writeSpend(data: Data, call: Call, s: Settlement): Promise<boolean> {
  let lastError: unknown;
  for (let attempt = 0; attempt < WRITE_ATTEMPTS; attempt++) {
    try {
      const updated = await data.db.update(creditLedger).set({
        kind: 'spend', costNanoUsd: s.nanoUsd, estimated: s.estimated, usageSource: s.source,
        inputTokens: s.usage.input, freshInputTokens: s.fresh, cachedInputTokens: s.usage.cacheRead, cacheWriteTokens: s.usage.cacheWrite,
        cacheWriteLongTokens: s.usage.cacheWriteLong, outputTokens: s.usage.output, reasoningTokens: s.usage.reasoning,
      }).where(and(eq(creditLedger.idempotencyKey, call.key), eq(creditLedger.userId, call.userId), eq(creditLedger.kind, 'hold'))).returning({ id: creditLedger.id });
      if (updated.length === 0) console.info('aero spend already settled', call.key);
      return updated.length > 0;
    } catch (error) {
      lastError = error;
      await new Promise(resolve => setTimeout(resolve, 100 * 2 ** attempt));
    }
  }
  throw lastError;
}
