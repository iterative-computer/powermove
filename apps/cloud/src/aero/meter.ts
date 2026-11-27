/* Metering one Aero request: reserve, then settle. The user pays exactly what the provider bills us.

   Reserve (in the provider fetch, where the exact provider request is known): bound the request's input tokens from
   above, cap its output at what the balance allows, and hold the most it can cost, inside one transaction that
   locks the account, so parallel requests see each other's holds and none can spend past the balance.
   Settle (when the stream ends, whether or not the client is still there): replace the hold with the exact cost of
   the provider's reported usage. Without a report, bill only what is proven (Anthropic's message_start reports the
   input); a stream that opened and reported nothing at all bills its hold, marked estimated. A request the
   provider refused (no stream) costs nothing. A hold whose request never settles (the Worker died) expires and
   from then on counts only what it had proven. */
import { and, eq, sql } from 'drizzle-orm';
import type { LanguageModelUsage, TextStreamPart, ToolSet } from 'ai';
import type { Data } from '../db/client';
import { creditLedger } from '../db/schema';
import { periodBalance, type Period } from '../billing/ledger';
import { type AeroModel, type Metered, PRICE_VERSION, ceilingRates, cost } from './catalogue';
import { V2Error } from '../v2-error';

/** How long a hold reserves credits. A request still running after this is settled all the same when it ends. */
export const HOLD_TTL_MS = 60 * 60 * 1000;
/** The least output a request is let through with; under this the balance can't cover a useful reply. */
export const MIN_OUTPUT = 1024;
/** An output cap this large needs no exact input count; under it (and under what was asked) the provider counts. */
const COMFORT_OUTPUT = 32_768;
/** Tokens a provider adds around the request (tool-use and thinking system prompts, message framing). */
const FRAMING_TOKENS = 4096;

type Source = 'reported' | 'partial' | 'hold' | 'none';
type Fetch = (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>;

/** An upper bound on the provider request's input tokens: one token per UTF-8 byte of the request as sent (every
    token of a byte-level tokenizer covers at least one byte), images at a bound from their pixel size instead of
    their base64, plus the provider's framing. */
export function inputBound(body: unknown): number {
  let images = 0;
  const json = JSON.stringify(body, (_key, value: unknown) => {
    if (value && typeof value === 'object') {
      const v = value as Record<string, any>;
      if (v.type === 'image' && v.source?.type === 'base64' && typeof v.source.data === 'string') {
        images += imageBound(v.source.data);
        return { ...v, source: { ...v.source, data: '' } };
      }
      if (v.type === 'input_image' && typeof v.image_url === 'string' && v.image_url.startsWith('data:')) {
        images += imageBound(v.image_url.slice(v.image_url.indexOf(',') + 1));
        return { ...v, image_url: '' };
      }
    }
    return value;
  });
  return new TextEncoder().encode(json).length + images + FRAMING_TOKENS;
}

/** An image's most input tokens: a token per 16x16 pixels plus a constant (Anthropic bills about one per 750 pixels;
    OpenAI at most ~1.6 per 32x32 patch, or 170 per 512x512 tile plus 85). Without readable dimensions, its base64
    length (a provider can't take more tokens from an image than it has bytes, and refuses one it can't decode). */
export function imageBound(base64: string): number {
  const size = imageSize(base64);
  return size ? Math.ceil(size.width / 16) * Math.ceil(size.height / 16) + 1024 : base64.length;
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
function anthropicRaw(usage: Record<string, any>): Metered & { reasoning: number } {
  const fresh = Number(usage.input_tokens ?? 0), write = Number(usage.cache_creation_input_tokens ?? 0), read = Number(usage.cache_read_input_tokens ?? 0);
  return { input: fresh + write + read, cacheRead: read, cacheWrite: write, cacheWriteLong: Number(usage.cache_creation?.ephemeral_1h_input_tokens ?? 0), output: Number(usage.output_tokens ?? 0), reasoning: Number(usage.output_tokens_details?.thinking_tokens ?? 0) };
}

const ZERO: Metered & { reasoning: number } = { input: 0, cacheRead: 0, cacheWrite: 0, cacheWriteLong: 0, output: 0, reasoning: 0 };

export interface MeterOptions {
  /** The request's connection: the reservation runs before the response goes out. */
  data: Data;
  /** A connection that outlives the response, for the progress and settle writes. The meter ends it. */
  ledger: Data;
  userId: string;
  key: string;
  model: AeroModel;
  subscriptionId: string;
  period: Period;
  /** The client's own output limit, if it set one. */
  requestedMax: number | undefined;
  fetch?: Fetch;
}

export class Meter {
  /** Settles with the hold reserved, or a refusal (402, 409) the route answers instead of a stream. */
  readonly admission: Promise<void>;
  private admit!: { resolve: () => void; reject: (error: unknown) => void };
  private refusal: unknown = null;
  private admitting: Promise<void> | null = null;
  holdNano = 0;
  outputCap = 0;
  private held = false;
  private opened = false;
  private started: Record<string, any> | null = null;
  private delta: Record<string, any> | null = null;
  private finalRaw = false;
  private sdkUsage: LanguageModelUsage | null = null;
  private progress: Promise<unknown> = Promise.resolve();
  private settling: Promise<void> | null = null;
  private readonly upstream: Fetch;

  constructor(private readonly options: MeterOptions) {
    this.upstream = options.fetch ?? ((input, init) => fetch(input, init));
    this.admission = new Promise((resolve, reject) => { this.admit = { resolve, reject }; });
    this.admission.catch(() => {});
  }

  /** The provider fetch: reserves on the first generation call, caps its output, and notes whether a stream opened. */
  readonly fetch: Fetch = async (input, init) => {
    const url = typeof input === 'string' ? input : input instanceof URL ? input.href : input.url;
    const generate = this.options.model.provider === 'anthropic' ? /\/messages$/.test(new URL(url).pathname) : /\/responses$/.test(new URL(url).pathname);
    if (!generate || typeof init?.body !== 'string') return this.upstream(input, init);
    const body = JSON.parse(init.body) as Record<string, any>;
    this.admitting ??= this.reserve(url, init, body);
    await this.admitting;
    const field = this.options.model.provider === 'anthropic' ? 'max_tokens' : 'max_output_tokens';
    body[field] = Math.min(typeof body[field] === 'number' ? body[field] : Infinity, this.outputCap);
    const response = await this.upstream(input, { ...init, body: JSON.stringify(body) });
    if (response.ok) this.opened = true;
    return response;
  };

  /** Ends a pending admission when the call failed before it reached the provider. */
  abandon() { if (!this.held && !this.refusal) this.admit.reject(new V2Error(502, 'upstream_error')); }

  private async reserve(url: string, init: RequestInit, body: Record<string, any>): Promise<void> {
    try {
      const { model, requestedMax } = this.options;
      const desired = Math.min(requestedMax ?? model.maxOutput, model.maxOutput);
      const floor = Math.min(MIN_OUTPUT, desired);
      let bound = inputBound(body);
      const capFor = (available: number, input: number) => {
        const rates = ceilingRates(model, input);
        return Math.min(desired, Math.floor((available - input * rates.input) / rates.output));
      };
      const before = (await periodBalance(this.options.data.db, this.options.userId, this.options.period)).remainingNano;
      if (capFor(before, bound) < Math.min(desired, COMFORT_OUTPUT)) {
        const counted = await this.count(url, init, body);
        // Counts are the provider's estimate of what it bills; a margin keeps the bound above the bill.
        if (counted !== null) bound = Math.min(bound, counted + Math.ceil(counted / 20) + FRAMING_TOKENS);
      }
      await this.options.data.tx(async (tx) => {
        await tx.execute(sql`select pg_advisory_xact_lock(hashtext(${`aero-balance:${this.options.userId}`}))`);
        const available = (await periodBalance(tx, this.options.userId, this.options.period)).remainingNano;
        const cap = capFor(available, bound);
        if (cap < floor) throw new V2Error(402, 'credits_exhausted');
        const rates = ceilingRates(model, bound);
        const hold = bound * rates.input + cap * rates.output;
        const claimed = await tx.insert(creditLedger).values({
          userId: this.options.userId, kind: 'hold', idempotencyKey: this.options.key, credits: 0, subscriptionId: this.options.subscriptionId,
          model: model.id, holdNanoUsd: hold, holdExpiresAt: new Date(Date.now() + HOLD_TTL_MS), outputCap: cap, priceVersion: PRICE_VERSION,
        }).onConflictDoNothing({ target: creditLedger.idempotencyKey }).returning({ id: creditLedger.id });
        if (claimed.length === 0) throw new V2Error(409, 'duplicate_request');
        this.holdNano = hold;
        this.outputCap = cap;
      });
      this.held = true;
      this.admit.resolve();
    } catch (error) {
      this.refusal = error;
      this.admit.reject(error);
      throw error;
    }
  }

  /** The provider's own count of the request's input tokens (free, no generation), or null if it can't say. */
  private async count(url: string, init: RequestInit, body: Record<string, any>): Promise<number | null> {
    const anthropic = this.options.model.provider === 'anthropic';
    const fields = anthropic ? ['model', 'system', 'messages', 'tools', 'tool_choice', 'thinking'] : ['model', 'instructions', 'input', 'tools', 'tool_choice', 'parallel_tool_calls', 'reasoning', 'text'];
    const subset = Object.fromEntries(fields.filter(field => body[field] !== undefined).map(field => [field, body[field]]));
    try {
      const response = await this.upstream(anthropic ? `${url}/count_tokens` : `${url}/input_tokens`, { method: 'POST', headers: init.headers, body: JSON.stringify(subset), signal: AbortSignal.timeout(10_000) });
      if (!response.ok) return null;
      const counted = (await response.json() as { input_tokens?: unknown }).input_tokens;
      return typeof counted === 'number' && Number.isInteger(counted) && counted >= 0 ? counted : null;
    } catch { return null; }
  }

  /** Watches the stream: the raw provider events say whether and what usage was reported; the SDK's finish part
      carries its converted usage. */
  observe(part: TextStreamPart<ToolSet>) {
    if (part.type === 'finish') { this.sdkUsage = part.totalUsage; return; }
    if (part.type !== 'raw') return;
    const raw = part.rawValue as Record<string, any> | null;
    if (!raw || typeof raw !== 'object') return;
    if (this.options.model.provider === 'anthropic') {
      if (raw.type === 'message_start' && raw.message?.usage) {
        const started: Record<string, any> = { ...raw.message.usage };
        this.started = started;
        // Proven as soon as it arrives, so a request whose Worker dies still bills its input.
        const proven = cost(this.options.model, anthropicRaw(started)).nanoUsd;
        this.progress = this.options.ledger.db.update(creditLedger).set({ costNanoUsd: proven })
          .where(and(eq(creditLedger.idempotencyKey, this.options.key), eq(creditLedger.kind, 'hold'))).catch(() => {});
      } else if (raw.type === 'message_delta' && raw.usage) {
        this.delta = { ...this.delta, ...Object.fromEntries(Object.entries(raw.usage).filter(([, v]) => v != null)) };
      } else if (raw.type === 'message_stop') {
        this.finalRaw = this.delta !== null;
      }
    } else if ((raw.type === 'response.completed' || raw.type === 'response.incomplete' || raw.type === 'response.failed') && raw.response?.usage) {
      this.finalRaw = true;
    }
  }

  /** Whether the provider reported the request's final usage (a stream that ended without it was cut off). */
  get reported(): boolean { return this.finalRaw && this.sdkUsage !== null; }

  /** Writes the request's spend, once: the hold becomes the exact cost. */
  settle(): Promise<void> {
    this.settling ??= (async () => {
      if (!this.held) { await this.options.ledger.end(); return; }
      let usage: Metered & { reasoning: number };
      let source: Source;
      if (this.reported) { usage = meteredOf(this.sdkUsage!); source = 'reported'; }
      else if (this.started) { usage = anthropicRaw({ ...this.started, ...this.delta }); source = 'partial'; }
      else if (this.opened) { usage = ZERO; source = 'hold'; }
      else { usage = ZERO; source = 'none'; }
      const price = cost(this.options.model, usage);
      try {
        await this.progress;
        await this.options.ledger.db.update(creditLedger).set({
          kind: 'spend', costNanoUsd: source === 'hold' ? this.holdNano : price.nanoUsd, estimated: source === 'hold', usageSource: source,
          inputTokens: usage.input, freshInputTokens: price.fresh, cachedInputTokens: usage.cacheRead, cacheWriteTokens: usage.cacheWrite,
          cacheWriteLongTokens: usage.cacheWriteLong, outputTokens: usage.output, reasoningTokens: usage.reasoning,
        }).where(and(eq(creditLedger.idempotencyKey, this.options.key), eq(creditLedger.userId, this.options.userId)));
        if (price.nanoUsd > this.holdNano) console.error('aero spend over its hold', this.options.key, price.nanoUsd, this.holdNano);
      } catch (error) {
        console.error('aero ledger write failed', this.options.key, (error as Error).name);
      } finally {
        await this.options.ledger.end();
      }
    })();
    return this.settling;
  }
}
