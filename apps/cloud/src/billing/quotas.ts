/* Pure quota decisions. The database layer (usage.ts) gathers the numbers and
   these functions decide; tests exercise them without a database. */
import type { Entitlements, ModelProvider, ShareQuota } from './plans';

const DAY = 24 * 3600 * 1000;
export const WEEK = 7 * DAY;

/** The usage period a request falls in: the Stripe billing period when there is one, else the UTC calendar month. */
export function usagePeriod(now: Date, billing?: { start: Date | null; end: Date | null } | null): { start: Date; end: Date } {
  if (billing?.start && billing.end && billing.start <= now && now < billing.end) return { start: billing.start, end: billing.end };
  const start = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1));
  const end = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth() + 1, 1));
  return { start, end };
}
export function usageDay(now: Date): { start: Date; end: Date } {
  const start = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  return { start, end: new Date(start.getTime() + DAY) };
}

export interface ProviderSpend { periodMicros: number; dayMicros: number }
export type ModelDecision =
  | { ok: true; remainingMicros: number }
  | { ok: false; reason: 'no_hosted_models' | 'period_limit' | 'day_limit'; resetsAt: Date | null };

export function checkModelBudget(ent: Entitlements, provider: ModelProvider, spend: ProviderSpend, windows: { period: { end: Date }; day: { end: Date } }): ModelDecision {
  const allowance = ent.models?.[provider];
  if (!allowance) return { ok: false, reason: 'no_hosted_models', resetsAt: null };
  if (spend.periodMicros >= allowance.periodMicros) return { ok: false, reason: 'period_limit', resetsAt: windows.period.end };
  if (spend.dayMicros >= allowance.dayMicros) return { ok: false, reason: 'day_limit', resetsAt: windows.day.end };
  return { ok: true, remainingMicros: Math.min(allowance.periodMicros - spend.periodMicros, allowance.dayMicros - spend.dayMicros) };
}

/** Share history: every link the user ever created, revoked ones included, so deleting a link never refunds quota. */
export interface ShareHistory { total: number; latestCreatedAt: Date | null }
export type ShareDecision =
  | { ok: true; remaining: number | null }
  | { ok: false; reason: 'lifetime_used' | 'weekly_used'; nextAvailableAt: Date | null };

export function checkShareQuota(quota: ShareQuota, history: ShareHistory & { inLastWeek: number; oldestInLastWeek: Date | null }, now = new Date()): ShareDecision {
  switch (quota.kind) {
    case 'unlimited': return { ok: true, remaining: null };
    case 'lifetime':
      return history.total < quota.count ? { ok: true, remaining: quota.count - history.total } : { ok: false, reason: 'lifetime_used', nextAvailableAt: null };
    case 'weekly': {
      if (history.inLastWeek < quota.count) return { ok: true, remaining: quota.count - history.inLastWeek };
      const next = history.oldestInLastWeek ? new Date(history.oldestInLastWeek.getTime() + WEEK) : new Date(now.getTime() + WEEK);
      return { ok: false, reason: 'weekly_used', nextAvailableAt: next };
    }
  }
}

export type StorageDecision = { ok: true; remainingBytes: number } | { ok: false; reason: 'storage_full'; limitBytes: number; usedBytes: number };
/** The storage hook every upload path calls before writing. */
export function checkStorage(ent: Entitlements, usedBytes: number, addBytes: number): StorageDecision {
  if (usedBytes + addBytes > ent.storageBytes) return { ok: false, reason: 'storage_full', limitBytes: ent.storageBytes, usedBytes };
  return { ok: true, remainingBytes: ent.storageBytes - usedBytes - addBytes };
}
