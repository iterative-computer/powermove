/*
 * Powermove plans as the desktop sees them (Aerolite, Aero). The cloud owns
 * entitlements; this file only describes the wire shapes so main can validate
 * responses and the renderer can type them.
 */
import { z } from 'zod';

export const PLAN_TIERS = ['free', 'aerolite', 'aero'] as const;
export type PlanTier = (typeof PLAN_TIERS)[number];

const ShareQuota = z.union([
  z.object({ kind: z.literal('lifetime'), count: z.number() }),
  z.object({ kind: z.literal('weekly'), count: z.number() }),
  z.object({ kind: z.literal('unlimited') })
]);
const PlanSummary = z.object({
  tier: z.enum(PLAN_TIERS), label: z.string(), priceCents: z.number(), storageBytes: z.number(),
  shares: ShareQuota, interactiveShares: z.boolean(),
  models: z.array(z.object({ id: z.string(), provider: z.enum(['anthropic', 'openai']), label: z.string() }))
});
const ProviderUsage = z.object({ period: z.number(), day: z.number(), periodResetsAt: z.string(), dayResetsAt: z.string() });
export const PlanDto = z.object({
  plan: PlanSummary,
  subscription: z.object({ status: z.string(), renewsAt: z.string().nullable(), cancelAtPeriodEnd: z.boolean() }).nullable(),
  usage: z.object({ anthropic: ProviderUsage, openai: ProviderUsage }).nullable(),
  shares: z.object({ used: z.number(), usedThisWeek: z.number(), canCreate: z.boolean(), nextAvailableAt: z.string().nullable() }),
  storage: z.object({ usedBytes: z.number(), limitBytes: z.number() }),
  upgrades: z.array(PlanSummary),
  billingConfigured: z.boolean()
});
export type PlanDto = z.infer<typeof PlanDto>;
export type PlanSummary = z.infer<typeof PlanSummary>;

export const ShareDto = z.object({
  id: z.string(), kind: z.string(), title: z.string(), url: z.string(), createdAt: z.string(),
  revokedAt: z.string().nullable(), viewCount: z.number(), sizeBytes: z.number()
});
export type ShareDto = z.infer<typeof ShareDto>;
export const SharesDto = z.object({ items: z.array(ShareDto) });
export const CreatedShareDto = z.object({ id: z.string(), url: z.string(), interactive: z.boolean() });
export type CreatedShareDto = z.infer<typeof CreatedShareDto>;

/** What the desktop uploads for a read-only share page. Mirrors the cloud's CreateShare schema. */
export interface ProjectShareSnapshot {
  kind: 'project'; title: string; description?: string;
  width: number; height: number; fps: number; durationSeconds: number;
  compositions: Array<{ name: string; durationSeconds: number; layerCount: number }>;
  layers: Array<{ name: string; type: string; inSeconds?: number; outSeconds?: number }>;
}
export interface ExtensionShareSnapshot {
  kind: 'extension'; title: string; description?: string; handle?: string; version?: string; tagline?: string;
  permissions: string[]; contributes: Array<{ kind: string; name: string }>; readme?: string;
}
export type ShareSnapshot = ProjectShareSnapshot | ExtensionShareSnapshot;
export interface CreateShareRequest { snapshot: ShareSnapshot; /** base64 PNG or JPEG, ≤ 2 MB */ poster?: string }

/** The hosted models' provider id in the agent picker. */
export const AEROLITE_PROVIDER = 'aerolite' as const;

/** "42%" style label for a usage fraction; never shows dollars. */
export function usagePercent(fraction: number): string {
  if (fraction <= 0) return '0%';
  if (fraction < 0.01) return '<1%';
  return `${Math.min(100, Math.round(fraction * 100))}%`;
}
export function formatBytes(bytes: number): string {
  if (bytes < 1024 ** 2) return `${Math.max(0, Math.round(bytes / 1024))} KB`;
  if (bytes < 1024 ** 3) return `${(bytes / 1024 ** 2).toFixed(bytes < 10 * 1024 ** 2 ? 1 : 0)} MB`;
  return `${+(bytes / 1024 ** 3).toFixed(1)} GB`;
}
