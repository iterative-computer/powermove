/* What Aero costs: the AERO_PRICING secret, never the repository. JSON:
   { "version": string, "creditNanoUsd": integer, "models": { "<catalogue model id>": Price & { "long"?: Price & { "above": integer } } } }
   with Price = { "input", "cachedInput", "cacheWrite", "cacheWriteLong", "output" } in USD per million tokens.
   - creditNanoUsd: one credit, in nano-USD. Costs are integer nano-USD, so every price must be a whole number of
     nano-USD per token (USD per million × 1000 an integer): nothing is rounded per request.
   - cachedInput is a cache read; cacheWrite a write at the default TTL; cacheWriteLong a write at the long TTL
     (priced in case a provider reports one; Aero never asks for it).
   - long: a tier for requests whose input (cache reads and writes included) is over `above` tokens, priced at it
     in full.
   Every catalogue model must be priced; anything else makes the pricing unusable: Aero (/v2/aero) then answers 503
   `misconfigured`, and the rest of the Worker shows credit figures as zero. */
import { z } from 'zod';
import { type AeroModel, MODELS, type Metered } from './catalogue';
import { V2Error } from '../v2-error';

/** A whole number of nano-USD per token. */
const UsdPerMillion = z.number().nonnegative().finite().refine(value => Math.abs(value * 1000 - Math.round(value * 1000)) < 1e-9, 'not a whole number of nano-USD per token');
const Price = z.object({ input: UsdPerMillion, cachedInput: UsdPerMillion, cacheWrite: UsdPerMillion, cacheWriteLong: UsdPerMillion, output: UsdPerMillion }).strict();
const ModelPrice = Price.extend({ long: Price.extend({ above: z.number().int().positive() }).strict().optional() }).strict();
const PricingJson = z.object({
  version: z.string().min(1).max(200),
  creditNanoUsd: z.number().int().positive(),
  models: z.record(z.string(), ModelPrice),
}).strict().refine(pricing => MODELS.every(model => pricing.models[model.id] !== undefined), 'a catalogue model has no price');
export type Price = z.infer<typeof Price>;
export type Pricing = z.infer<typeof PricingJson>;

const parsed = new Map<string, Pricing | null>();
/** The pricing in `raw` (the secret's text), or null when it is missing or malformed. */
export function parsePricing(raw: string | undefined): Pricing | null {
  if (!raw) return null;
  if (!parsed.has(raw)) {
    let value: unknown;
    try { value = JSON.parse(raw); } catch { value = null; }
    const result = PricingJson.safeParse(value);
    parsed.set(raw, result.success ? result.data : null);
  }
  return parsed.get(raw)!;
}

/** One credit in nano-USD, or null without a usable pricing (logged by name only). */
export function creditUnit(env: CloudflareBindings): number | null {
  const pricing = parsePricing(env.AERO_PRICING);
  if (!pricing) console.error('AERO_PRICING missing or malformed');
  return pricing?.creditNanoUsd ?? null;
}

/** This environment's pricing; 503 `misconfigured` (logged by name only) without a usable one. Aero only. */
export function pricingOf(env: CloudflareBindings): Pricing {
  const pricing = parsePricing(env.AERO_PRICING);
  if (!pricing) {
    console.error('AERO_PRICING missing or malformed');
    throw new V2Error(503, 'misconfigured');
  }
  return pricing;
}

/** USD per million tokens is micro-USD per token; a thousand times that is nano-USD per token, a whole number. */
const nano = (usdPerMillion: number) => Math.round(usdPerMillion * 1000);

/** The tier a request with `input` tokens of input (cache included) is priced at. */
export function tier(pricing: Pricing, model: AeroModel, input: number): Price {
  const price = pricing.models[model.id]!;
  return price.long && input > price.long.above ? price.long : price;
}

export interface Cost { nanoUsd: number; fresh: number }
export function cost(pricing: Pricing, model: AeroModel, usage: Metered): Cost {
  const cacheRead = Math.min(usage.cacheRead, usage.input);
  const cacheWrite = Math.min(usage.cacheWrite, usage.input - cacheRead);
  const writeLong = Math.min(usage.cacheWriteLong, cacheWrite);
  const fresh = usage.input - cacheRead - cacheWrite;
  const price = tier(pricing, model, usage.input);
  const nanoUsd = fresh * nano(price.input) + cacheRead * nano(price.cachedInput)
    + (cacheWrite - writeLong) * nano(price.cacheWrite) + writeLong * nano(price.cacheWriteLong) + usage.output * nano(price.output);
  return { nanoUsd, fresh };
}

/** The most `input` tokens can cost (all of it written to the default-TTL cache or fresh, whichever is dearer),
    and the price of one output token, at the tier that much input would be billed at. */
export function ceilingRates(pricing: Pricing, model: AeroModel, input: number): { input: number; output: number } {
  const price = tier(pricing, model, input);
  return { input: nano(Math.max(price.input, price.cacheWrite)), output: nano(price.output) };
}
