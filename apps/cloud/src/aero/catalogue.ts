/* The curated Aero catalogue and its credit prices. One credit is $0.005 (half a cent) of provider list price:
   credits = ceil(cost_usd * 200), so Aerolite's 1,000 credits are about $5 of inference. Prices are USD per million tokens. Cached input is what a cache read costs;
   cacheWrite is what Anthropic charges to write the cache (OpenAI writes for free, so its rate is the input rate).
   PENDING(owner): confirm the list prices below; they are the defaults the contract names. */
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
  price: { input: number; cachedInput: number; cacheWrite: number; output: number };
}
export const MODELS: readonly AeroModel[] = [
  { id: 'openai/gpt-6.2-sol', name: 'GPT-6.2 Sol', provider: 'openai', providerModel: 'gpt-6.2-sol', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', default: true, context: 400_000, price: { input: 1.75, cachedInput: 0.175, cacheWrite: 1.75, output: 14 } },
  { id: 'openai/gpt-6.2-astra', name: 'GPT-6.2 Astra', provider: 'openai', providerModel: 'gpt-6.2-astra', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', context: 400_000, price: { input: 0.5, cachedInput: 0.05, cacheWrite: 0.5, output: 4 } },
  { id: 'openai/gpt-6-luna', name: 'GPT-6 Luna', provider: 'openai', providerModel: 'gpt-6-luna', efforts: ['none', 'low', 'medium', 'high'], defaultEffort: 'low', context: 400_000, price: { input: 0.25, cachedInput: 0.025, cacheWrite: 0.25, output: 2 } },
  { id: 'anthropic/claude-opus-5-5', name: 'Claude Opus 5.5', provider: 'anthropic', providerModel: 'claude-opus-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, price: { input: 5, cachedInput: 0.5, cacheWrite: 6.25, output: 25 } },
  { id: 'anthropic/claude-sonnet-5-5', name: 'Claude Sonnet 5.5', provider: 'anthropic', providerModel: 'claude-sonnet-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, price: { input: 3, cachedInput: 0.3, cacheWrite: 3.75, output: 15 } },
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
  const micro = fresh * model.price.input + cacheRead * model.price.cachedInput + cacheWrite * model.price.cacheWrite + usage.output * model.price.output;
  const microUsd = Math.round(micro * 1000) / 1000;
  return { microUsd: Math.ceil(microUsd), credits: Math.ceil(microUsd / CREDIT_MICRO_USD) };
}
