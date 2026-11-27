/* The curated Aero catalogue and its prices. The user pays exactly what the provider bills: one credit is $0.005 of
   provider list price, and costs are kept in integer nano-USD (every list price is a whole number of nano-USD per
   token), so nothing is rounded per request. Prices are USD per million tokens.
   - cachedInput is a cache read; cacheWrite is a write at the default TTL (Anthropic's 5-minute ephemeral cache,
     OpenAI's 30 minutes), 1.25x input; cacheWriteLong is a write at the long TTL (Anthropic 1 hour, 2x input; OpenAI
     12 hours, 2.5x input). Aero never asks for the long TTL; it is priced in case a provider reports it.
   - `long` is OpenAI's long-context tier: a request whose input (cache reads and writes included) is over `above`
     tokens is priced at it in full. OpenAI's pricing page: "Long context: >272K input tokens".
   Prices, output limits and context sizes are models.dev's (https://models.dev/api.json, PRICE_VERSION), the owner's
   source of truth; the long-TTL write prices are the providers' own (models.dev doesn't list them). */
export type Provider = 'openai' | 'anthropic';
export interface Price { input: number; cachedInput: number; cacheWrite: number; cacheWriteLong: number; output: number }
export interface AeroModel {
  id: `${Provider}/${string}`;
  name: string;
  provider: Provider;
  providerModel: string;
  efforts: string[];
  defaultEffort: string;
  default?: true;
  context: number;
  /** The most output (reasoning included) one request can produce. */
  maxOutput: number;
  price: Price & { long?: Price & { above: number } };
}
export const PRICE_VERSION = 'models.dev 2026-11-26';
export const MODELS: readonly AeroModel[] = [
  { id: 'openai/gpt-6.2-sol', name: 'GPT-6.2 Sol', provider: 'openai', providerModel: 'gpt-6.2-sol', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', default: true, context: 1_050_000, maxOutput: 128_000, price: { input: 2, cachedInput: 0.1, cacheWrite: 2.5, cacheWriteLong: 5, output: 10, long: { above: 272_000, input: 4, cachedInput: 0.2, cacheWrite: 5, cacheWriteLong: 10, output: 15 } } },
  { id: 'openai/gpt-6.2-astra', name: 'GPT-6.2 Astra', provider: 'openai', providerModel: 'gpt-6.2-astra', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', context: 1_050_000, maxOutput: 128_000, price: { input: 8, cachedInput: 0.4, cacheWrite: 10, cacheWriteLong: 20, output: 40, long: { above: 272_000, input: 16, cachedInput: 0.8, cacheWrite: 20, cacheWriteLong: 40, output: 60 } } },
  { id: 'openai/gpt-6-luna', name: 'GPT-6 Luna', provider: 'openai', providerModel: 'gpt-6-luna', efforts: ['none', 'low', 'medium', 'high'], defaultEffort: 'low', context: 1_050_000, maxOutput: 128_000, price: { input: 0.1, cachedInput: 0.01, cacheWrite: 0.125, cacheWriteLong: 0.25, output: 0.5, long: { above: 272_000, input: 0.2, cachedInput: 0.02, cacheWrite: 0.25, cacheWriteLong: 0.5, output: 0.75 } } },
  { id: 'anthropic/claude-opus-5-5', name: 'Claude Opus 5.5', provider: 'anthropic', providerModel: 'claude-opus-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, maxOutput: 128_000, price: { input: 4, cachedInput: 0.2, cacheWrite: 5, cacheWriteLong: 8, output: 20 } },
  { id: 'anthropic/claude-sonnet-5-5', name: 'Claude Sonnet 5.5', provider: 'anthropic', providerModel: 'claude-sonnet-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, maxOutput: 128_000, price: { input: 2, cachedInput: 0.1, cacheWrite: 2.5, cacheWriteLong: 4, output: 10 } },
];
/** One credit in nano-USD: $0.005. */
export const CREDIT_NANO_USD = 5_000_000;
export const modelById = (id: string): AeroModel | undefined => MODELS.find(model => model.id === id);

/** USD per million tokens is micro-USD per token; a thousand times that is nano-USD per token, a whole number. */
const nano = (usdPerMillion: number) => Math.round(usdPerMillion * 1000);

/** The tier a request with `input` tokens of input (cache included) is priced at. */
export function tier(model: AeroModel, input: number): Price {
  const long = model.price.long;
  return long && input > long.above ? long : model.price;
}

/** Token counts as the provider reported them. `input` is the total, cache reads and writes included; `cacheWrite`
    is every cache write, `cacheWriteLong` the part of it written at the long TTL; `output` includes reasoning. */
export interface Metered { input: number; cacheRead: number; cacheWrite: number; cacheWriteLong: number; output: number }
export interface Cost { nanoUsd: number; fresh: number }
export function cost(model: AeroModel, usage: Metered): Cost {
  const cacheRead = Math.min(usage.cacheRead, usage.input);
  const cacheWrite = Math.min(usage.cacheWrite, usage.input - cacheRead);
  const writeLong = Math.min(usage.cacheWriteLong, cacheWrite);
  const fresh = usage.input - cacheRead - cacheWrite;
  const price = tier(model, usage.input);
  const nanoUsd = fresh * nano(price.input) + cacheRead * nano(price.cachedInput)
    + (cacheWrite - writeLong) * nano(price.cacheWrite) + writeLong * nano(price.cacheWriteLong) + usage.output * nano(price.output);
  return { nanoUsd, fresh };
}

/** The most `input` tokens can cost (all of it written to the default-TTL cache, the dearest way Aero sends input),
    and the price of one output token, at the tier that much input would be billed at. */
export function ceilingRates(model: AeroModel, input: number): { input: number; output: number } {
  const price = tier(model, input);
  return { input: nano(Math.max(price.input, price.cacheWrite)), output: nano(price.output) };
}
