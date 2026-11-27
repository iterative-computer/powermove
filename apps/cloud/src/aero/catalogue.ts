/* The curated Aero catalogue: the public facts of each model (ids, names, efforts, the default, context and output
   sizes). What a model costs is not here: it comes from the AERO_PRICING secret (pricing.ts). */
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
  /** The most output (reasoning included) one request can produce. */
  maxOutput: number;
}
export const MODELS: readonly AeroModel[] = [
  { id: 'openai/gpt-6.2-sol', name: 'GPT-6.2 Sol', provider: 'openai', providerModel: 'gpt-6.2-sol', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', default: true, context: 1_050_000, maxOutput: 128_000 },
  { id: 'openai/gpt-6.2-astra', name: 'GPT-6.2 Astra', provider: 'openai', providerModel: 'gpt-6.2-astra', efforts: ['none', 'low', 'medium', 'high', 'xhigh'], defaultEffort: 'medium', context: 1_050_000, maxOutput: 128_000 },
  { id: 'openai/gpt-6-luna', name: 'GPT-6 Luna', provider: 'openai', providerModel: 'gpt-6-luna', efforts: ['none', 'low', 'medium', 'high'], defaultEffort: 'low', context: 1_050_000, maxOutput: 128_000 },
  { id: 'anthropic/claude-opus-5-5', name: 'Claude Opus 5.5', provider: 'anthropic', providerModel: 'claude-opus-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, maxOutput: 128_000 },
  { id: 'anthropic/claude-sonnet-5-5', name: 'Claude Sonnet 5.5', provider: 'anthropic', providerModel: 'claude-sonnet-5-5', efforts: ['low', 'medium', 'high', 'xhigh', 'max'], defaultEffort: 'high', context: 1_000_000, maxOutput: 128_000 },
];
export const modelById = (id: string): AeroModel | undefined => MODELS.find(model => model.id === id);

/** Token counts as the provider reported them. `input` is the total, cache reads and writes included; `cacheWrite`
    is every cache write, `cacheWriteLong` the part of it written at the long TTL; `output` includes reasoning. */
export interface Metered { input: number; cacheRead: number; cacheWrite: number; cacheWriteLong: number; output: number }
