/* Database reads behind the plan: the user's tier, their usage meter, share
   history and stored bytes. Decisions live in quotas.ts. */
import { and, count, eq, gte, isNull, lt, min, sql, sum } from 'drizzle-orm';
import type { Db } from '../db/client';
import { shares, subscriptions, usageEvents } from '../db/schema';
import { effectiveTier, entitlementsFor, type Entitlements, type ModelProvider, type Tier } from './plans';
import { usageDay, usagePeriod, WEEK, type ProviderSpend } from './quotas';

export interface Account {
  tier: Tier;
  ent: Entitlements;
  sub: typeof subscriptions.$inferSelect | null;
  windows: { period: { start: Date; end: Date }; day: { start: Date; end: Date } };
}

export async function loadAccount(db: Db, userId: string, now = new Date()): Promise<Account> {
  const [sub] = await db.select().from(subscriptions).where(eq(subscriptions.userId, userId)).limit(1);
  const tier = effectiveTier(sub, now);
  const billing = tier === 'free' ? null : { start: sub?.currentPeriodStart ?? null, end: sub?.currentPeriodEnd ?? null };
  return { tier, ent: entitlementsFor(tier), sub: sub ?? null, windows: { period: usagePeriod(now, billing), day: usageDay(now) } };
}

export async function providerSpend(db: Db, userId: string, provider: ModelProvider, windows: Account['windows']): Promise<ProviderSpend> {
  const window = (start: Date, end: Date) => db.select({ micros: sum(usageEvents.costMicros) }).from(usageEvents)
    .where(and(eq(usageEvents.userId, userId), eq(usageEvents.provider, provider), gte(usageEvents.createdAt, start), lt(usageEvents.createdAt, end)));
  const [[period], [day]] = await Promise.all([window(windows.period.start, windows.period.end), window(windows.day.start, windows.day.end)]);
  return { periodMicros: Number(period?.micros ?? 0), dayMicros: Number(day?.micros ?? 0) };
}

export async function shareHistory(db: Db, userId: string, now = new Date()) {
  const weekAgo = new Date(now.getTime() - WEEK);
  const [[all], [week]] = await Promise.all([
    db.select({ n: count() }).from(shares).where(eq(shares.userId, userId)),
    db.select({ n: count(), oldest: min(shares.createdAt) }).from(shares).where(and(eq(shares.userId, userId), gte(shares.createdAt, weekAgo))),
  ]);
  return { total: Number(all?.n ?? 0), inLastWeek: Number(week?.n ?? 0), oldestInLastWeek: week?.oldest ? new Date(week.oldest) : null, latestCreatedAt: null };
}

/** Bytes the user currently stores in Powermove Cloud. Live share pages are the only stored data today; add new stores here. */
export async function storedBytes(db: Db, userId: string): Promise<number> {
  const [row] = await db.select({ bytes: sql<string>`coalesce(sum(${shares.sizeBytes}), 0)` }).from(shares).where(and(eq(shares.userId, userId), isNull(shares.revokedAt)));
  return Number(row?.bytes ?? 0);
}

/** Serialize quota-sensitive writes for one user (share creation). Call inside a transaction. */
export async function lockUser(tx: Db, userId: string): Promise<void> {
  await tx.execute(sql`select id from "user" where id = ${userId} for update`);
}
