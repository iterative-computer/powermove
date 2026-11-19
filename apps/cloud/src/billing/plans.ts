/* Tiers and what each one is entitled to. This is the single place that knows
   what a plan includes: routes ask `entitlementsFor(tier)` and never compare
   tier names themselves, so adding Aero features later means editing this table.

   Money is in micro-dollars (1e-6 USD) so per-request costs stay integers. */

export const TIERS = ['free', 'aerolite', 'aero'] as const;
export type Tier = (typeof TIERS)[number];
export type ModelProvider = 'anthropic' | 'openai';
export const MODEL_PROVIDERS: readonly ModelProvider[] = ['anthropic', 'openai'];

/** Per 1M tokens, in USD. `cacheRead`/`cacheWrite` default to input × 0.1 / × 1.25. */
export interface ModelPrice { input: number; output: number; cacheRead?: number; cacheWrite?: number }
export interface HostedModel { id: string; provider: ModelProvider; label: string; price: ModelPrice }

/* Public list prices as of 2026-11. Anthropic from the Claude API model table;
   the OpenAI Sol price is the published GPT-6.1 Sol rate, assumed unchanged for
   6.2 until OpenAI's pricing page confirms it. Keep these current: the usage
   meter bills against them. */
export const MODELS: Record<string, HostedModel> = {
  'claude-sonnet-5-5': { id: 'claude-sonnet-5-5', provider: 'anthropic', label: 'Claude Sonnet 5.5', price: { input: 2, output: 10, cacheRead: 0.1, cacheWrite: 2.5 } },
  'claude-haiku-5-5': { id: 'claude-haiku-5-5', provider: 'anthropic', label: 'Claude Haiku 5.5', price: { input: 0.1, output: 0.5 } },
  'claude-opus-5-5': { id: 'claude-opus-5-5', provider: 'anthropic', label: 'Claude Opus 5.5', price: { input: 4, output: 20, cacheRead: 0.2, cacheWrite: 5 } },
  'gpt-6.2-sol': { id: 'gpt-6.2-sol', provider: 'openai', label: 'GPT-6.2 Sol', price: { input: 2, output: 10, cacheRead: 0.1, cacheWrite: 2 } },
  'gpt-6-luna': { id: 'gpt-6-luna', provider: 'openai', label: 'GPT-6 Luna', price: { input: 0.1, output: 0.5, cacheWrite: 0.1 } },
};

export interface ProviderAllowance {
  /** Spend ceiling per billing period, in micro-dollars of list-price inference. */
  periodMicros: number;
  /** Spend ceiling per rolling day, so one afternoon cannot drain the month. */
  dayMicros: number;
  models: string[];
}

export type ShareQuota =
  | { kind: 'lifetime'; count: number }
  | { kind: 'weekly'; count: number }
  | { kind: 'unlimited' };

export interface Entitlements {
  tier: Tier;
  label: string;
  priceCents: number;
  /** null = the tier has no hosted models; the user brings their own agent. */
  models: Record<ModelProvider, ProviderAllowance> | null;
  storageBytes: number;
  shares: ShareQuota;
  /** Interactive share pages (live player, inspect layers) vs a static read-only page. */
  interactiveShares: boolean;
  projectSync: boolean;
  webEditing: boolean;
  collaboration: boolean;
}

const GB = 1024 ** 3;
const usd = (dollars: number) => Math.round(dollars * 1_000_000);

/** Tunables that may change without a code review of the plan logic. */
export const CONFIG = {
  /** Aerolite storage. The spec marks it "??"; 1 GB until the team decides. */
  aeroliteStorageBytes: 1 * GB,
  aeroStorageBytes: 50 * GB,
  /** Free accounts store only their one share page (poster + snapshot). */
  freeStorageBytes: 25 * 1024 ** 2,
  /* Inference budgets at list price. See docs/aerolite.md for the margin math. */
  aerolite: { anthropic: { period: usd(2.5), day: usd(0.5) }, openai: { period: usd(1.5), day: usd(0.3) } },
  aero: { anthropic: { period: usd(7), day: usd(1.4) }, openai: { period: usd(4), day: usd(0.8) } },
} as const;

const AEROLITE_MODELS = { anthropic: ['claude-sonnet-5-5', 'claude-haiku-5-5'], openai: ['gpt-6.2-sol', 'gpt-6-luna'] };

export const PLANS: Record<Tier, Entitlements> = {
  free: {
    tier: 'free', label: 'Free', priceCents: 0, models: null, storageBytes: CONFIG.freeStorageBytes,
    shares: { kind: 'lifetime', count: 1 }, interactiveShares: false,
    projectSync: false, webEditing: false, collaboration: false,
  },
  aerolite: {
    tier: 'aerolite', label: 'Aerolite', priceCents: 900,
    models: {
      anthropic: { periodMicros: CONFIG.aerolite.anthropic.period, dayMicros: CONFIG.aerolite.anthropic.day, models: AEROLITE_MODELS.anthropic },
      openai: { periodMicros: CONFIG.aerolite.openai.period, dayMicros: CONFIG.aerolite.openai.day, models: AEROLITE_MODELS.openai },
    },
    storageBytes: CONFIG.aeroliteStorageBytes,
    shares: { kind: 'weekly', count: 1 }, interactiveShares: false,
    projectSync: false, webEditing: false, collaboration: false,
  },
  // Not sold yet. Present so entitlement checks and the UI already handle it.
  aero: {
    tier: 'aero', label: 'Aero', priceCents: 1900,
    models: {
      anthropic: { periodMicros: CONFIG.aero.anthropic.period, dayMicros: CONFIG.aero.anthropic.day, models: [...AEROLITE_MODELS.anthropic, 'claude-opus-5-5'] },
      openai: { periodMicros: CONFIG.aero.openai.period, dayMicros: CONFIG.aero.openai.day, models: AEROLITE_MODELS.openai },
    },
    storageBytes: CONFIG.aeroStorageBytes,
    shares: { kind: 'unlimited' }, interactiveShares: true,
    projectSync: true, webEditing: true, collaboration: true,
  },
};

/** Tiers a user can buy today. Aero joins this list when it launches. */
export const PURCHASABLE: readonly Tier[] = ['aerolite'];

export function isTier(value: unknown): value is Tier { return typeof value === 'string' && (TIERS as readonly string[]).includes(value); }
export function entitlementsFor(tier: Tier): Entitlements { return PLANS[tier]; }

/** The tier a subscription row grants right now. Lapsed or unpaid subscriptions fall back to free. */
export function effectiveTier(sub: { tier: string; status: string; currentPeriodEnd: Date | null } | null | undefined, now = new Date()): Tier {
  if (!sub || !isTier(sub.tier)) return 'free';
  // Stripe keeps a subscription 'past_due' while it retries the card; keep access during that grace.
  if (!['active', 'trialing', 'past_due'].includes(sub.status)) return 'free';
  if (sub.currentPeriodEnd && sub.currentPeriodEnd.getTime() + 24 * 3600 * 1000 < now.getTime()) return 'free';
  return sub.tier;
}

export interface TokenUsage { input: number; output: number; cacheRead?: number; cacheWrite?: number }

/** List-price cost of one request, in micro-dollars, rounded up so tiny calls are never free. */
export function costMicros(model: HostedModel, usage: TokenUsage): number {
  const p = model.price;
  const perToken = (perMillion: number) => perMillion; // $/1M tokens == micro-dollars per token
  const total = usage.input * perToken(p.input) + usage.output * perToken(p.output)
    + (usage.cacheRead ?? 0) * perToken(p.cacheRead ?? p.input * 0.1)
    + (usage.cacheWrite ?? 0) * perToken(p.cacheWrite ?? p.input * 1.25);
  return Math.ceil(total);
}

/** Which model a tier may call for a provider; returns the HostedModel or a reason. */
export function resolveModel(ent: Entitlements, provider: ModelProvider, modelId: unknown):
  { ok: true; model: HostedModel } | { ok: false; reason: 'no_hosted_models' | 'model_not_included' } {
  if (!ent.models) return { ok: false, reason: 'no_hosted_models' };
  const allowance = ent.models[provider];
  if (typeof modelId !== 'string' || !allowance.models.includes(modelId)) return { ok: false, reason: 'model_not_included' };
  const model = MODELS[modelId];
  if (!model || model.provider !== provider) return { ok: false, reason: 'model_not_included' };
  return { ok: true, model };
}
