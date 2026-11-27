/* The curated Aero catalogue and its credit prices. One credit is $0.005 (half a cent) of provider list price:
   credits = ceil(cost_usd * 200), so Aerolite's 1,000 credits are about $5 of inference. Prices are USD per million tokens. Cached input is what a cache read costs;
   cacheWrite is what writing the cache costs. Prices and context sizes are models.dev's (https://models.dev/api.json,
   2026-11-26), the owner's source of truth. `long` is the provider's long-context tier: a request whose input exceeds
   `above` tokens is priced at it in full. */
export type Provider = 'openai' | 'anthropic';
export interface AeroModel {
  id: `${Provider}/${string}`;
  name: string;
  provider: Provider;
  providerModel: string;
  efforts: string[];
  defaultEffort: string;
  default?: true;
  context: number;
  price: Price & { long?: Price & { above: number } };
}
export interface Price { input: number; cachedInput: number; cacheWrite: number; output: number }
export const MODELS: readonly AeroModel[] = [
  { id: 'openai/gpt-6.2-sol', name: 'GPT-6.2 Sol', provider: 'openai', providerModel: 'gpt-6.2-sol', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', default: true, context: 1_050_000, price: { input: 2, cachedInput: 0.1, cacheWrite: 2.5, output: 10, long: { above: 272_000, input: 4, cachedInput: 0.2, cacheWrite: 5, output: 15 } } },
  { id: 'openai/gpt-6.2-astra', name: 'GPT-6.2 Astra', provider: 'openai', providerModel: 'gpt-6.2-astra', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', context: 1_050_000, price: { input: 8, cachedInput: 0.4, cacheWrite: 10, output: 40, long: { above: 272_000, input: 16, cachedInput: 0.8, cacheWrite: 20, output: 60 } } },
  { id: 'openai/gpt-6-luna', name: 'GPT-6 Luna', provider: 'openai', providerModel: 'gpt-6-luna', efforts: ['none', 'low', 'medium', 'high'], defaultEffort: 'low', context: 1_050_000, price: { input: 0.1, cachedInput: 0.01, cacheWrite: 0.125, output: 0.5, long: { above: 272_000, input: 0.2, cachedInput: 0.02, cacheWrite: 0.25, output: 0.75 } } },
  { id: 'anthropic/claude-opus-5-5', name: 'Claude Opus 5.5', provider: 'anthropic', providerModel: 'claude-opus-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, price: { input: 4, cachedInput: 0.2, cacheWrite: 5, output: 20 } },
  { id: 'anthropic/claude-sonnet-5-5', name: 'Claude Sonnet 5.5', provider: 'anthropic', providerModel: 'claude-sonnet-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, price: { input: 2, cachedInput: 0.1, cacheWrite: 2.5, output: 10 } },
];
/** One credit in micro-USD: $0.005. */
export const CREDIT_MICRO_USD = 5_000;
export const modelById = (id: string): AeroModel | undefined => MODELS.find(model => model.id === id);

/** Token counts as the provider reported them. `input` includes cache reads and writes. */
export interface Metered { input: number; cacheRead: number; cacheWrite: number; output: number }
export function cost(model: AeroModel, usage: Metered): { microUsd: number; credits: number } {
  const cacheRead = Math.min(usage.cacheRead, usage.input);
  const cacheWrite = Math.min(usage.cacheWrite, usage.input - cacheRead);
  const fresh = usage.input - cacheRead - cacheWrite;
  // USD per million tokens times tokens is micro-USD.
  const long = model.price.long;
  const price: Price = long && usage.input > long.above ? long : model.price;
  const micro = fresh * price.input + cacheRead * price.cachedInput + cacheWrite * price.cacheWrite + usage.output * price.output;
  const microUsd = Math.round(micro * 1000) / 1000;
  return { microUsd: Math.ceil(microUsd), credits: Math.ceil(microUsd / CREDIT_MICRO_USD) };
}
