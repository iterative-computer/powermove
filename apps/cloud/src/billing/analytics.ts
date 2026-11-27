/* Aero and billing analytics for the admin panel, from the ledger tables. Money is the ledger's: a request's cost is
   its spend (exact when the provider reported usage, the whole hold when it was estimated), or its whole hold
   while it is unsettled past its expiry; live holds are not spend yet. */
import { and, desc, eq, gte, inArray, sql } from 'drizzle-orm';
import type { AeroOverview } from '@powermove/registry/wire';
import type { Db } from '../db/client';
import { user } from '../db/auth-schema';
import { aeroBlocks, billingReversals, creditLedger, subscriptions } from '../db/schema';
import { pricingOf } from '../aero/pricing';
import { displayCredits, periodBalance, spendingPeriod } from './ledger';
import { planById } from './plans';

const DAY = 86_400_000;
const usd = (nano: number) => nano / 1e9;
/** A request row's spend. */
const spent = sql`case when ${creditLedger.kind} = 'spend' then ${creditLedger.costNanoUsd} when ${creditLedger.holdExpiresAt} <= now() then ${creditLedger.holdNanoUsd} else 0 end`;
const requests = inArray(creditLedger.kind, ['spend', 'hold']);
const usageColumns = {
  nano: sql<string>`coalesce(sum(${spent}), 0)`,
  calls: sql<number>`count(*)::int`,
  estimatedCalls: sql<number>`(count(*) filter (where ${creditLedger.estimated}))::int`,
  estimatedNano: sql<string>`coalesce(sum(${spent}) filter (where ${creditLedger.estimated}), 0)`,
  input: sql<string>`coalesce(sum(${creditLedger.inputTokens}), 0)`,
  cached: sql<string>`coalesce(sum(${creditLedger.cachedInputTokens}), 0)`,
  cacheWrite: sql<string>`coalesce(sum(${creditLedger.cacheWriteTokens}), 0)`,
  output: sql<string>`coalesce(sum(${creditLedger.outputTokens}), 0)`,
};
type UsageRow = { nano: string; calls: number; estimatedCalls: number; estimatedNano: string; input: string; cached: string; cacheWrite: string; output: string };
function usage(row: UsageRow | undefined, unit: number): AeroOverview['today'] {
  const input = Number(row?.input ?? 0), cached = Number(row?.cached ?? 0), nano = Number(row?.nano ?? 0);
  return {
    costUsd: usd(nano), credits: nano / unit, calls: row?.calls ?? 0, estimatedCalls: row?.estimatedCalls ?? 0, estimatedUsd: usd(Number(row?.estimatedNano ?? 0)),
    inputTokens: input, cachedTokens: cached, cacheWriteTokens: Number(row?.cacheWrite ?? 0), outputTokens: Number(row?.output ?? 0),
    cacheHitRatio: input > 0 ? cached / input : 0,
  };
}

export async function aeroOverview(db: Db, env: CloudflareBindings, now = new Date()): Promise<AeroOverview> {
  // Costs are the ledger's nano-USD; credits use the pricing's unit.
  const unit = pricingOf(env).creditNanoUsd;
  const credits = (nano: number) => nano / unit;
  const today = new Date(Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()));
  const since = new Date(today.getTime() - 29 * DAY);
  const [todayRow] = await db.select(usageColumns).from(creditLedger).where(and(requests, gte(creditLedger.createdAt, today)));
  const [last30Row] = await db.select(usageColumns).from(creditLedger).where(and(requests, gte(creditLedger.createdAt, since)));
  const models = await db.select({ model: creditLedger.model, ...usageColumns }).from(creditLedger)
    .where(and(requests, gte(creditLedger.createdAt, since))).groupBy(creditLedger.model).orderBy(desc(sql`sum(${spent})`));
  const days = await db.select({ date: sql<string>`to_char(${creditLedger.createdAt} at time zone 'UTC', 'YYYY-MM-DD')`, nano: usageColumns.nano, calls: usageColumns.calls })
    .from(creditLedger).where(and(requests, gte(creditLedger.createdAt, since))).groupBy(sql`1`);
  const byDay = new Map(days.map(day => [day.date, day]));
  const daily = Array.from({ length: 30 }, (_, i) => {
    const date = new Date(since.getTime() + i * DAY).toISOString().slice(0, 10);
    const day = byDay.get(date);
    const nano = Number(day?.nano ?? 0);
    return { date, costUsd: usd(nano), credits: credits(nano), calls: day?.calls ?? 0 };
  });

  const [subs] = await db.select({
    active: sql<number>`(count(*) filter (where ${subscriptions.status} in ('active', 'trialing')))::int`,
    new30: sql<number>`(count(*) filter (where ${subscriptions.createdAt} >= ${since.toISOString()}::timestamptz and ${subscriptions.status} not in ('incomplete', 'incomplete_expired')))::int`,
    cancelling: sql<number>`(count(*) filter (where ${subscriptions.status} in ('active', 'trialing') and ${subscriptions.cancelAtPeriodEnd}))::int`,
    pastDue: sql<number>`(count(*) filter (where ${subscriptions.status} in ('past_due', 'unpaid')))::int`,
  }).from(subscriptions);

  const top = await db.select({ id: creditLedger.userId, email: user.email, nano: usageColumns.nano, calls: usageColumns.calls }).from(creditLedger)
    .leftJoin(user, eq(user.id, creditLedger.userId)).where(and(requests, gte(creditLedger.createdAt, since)))
    .groupBy(creditLedger.userId, user.email).orderBy(desc(sql`sum(${spent})`)).limit(10);
  const topAccounts = await Promise.all(top.map(async (row) => {
    const current = await spendingPeriod(db, row.id);
    const left = current ? displayCredits(await periodBalance(db, row.id, current.period, unit), unit).remaining : null;
    return { user: { id: row.id, email: row.email }, costUsd: usd(Number(row.nano)), calls: row.calls, creditsLeft: left };
  }));

  const estimated = await db.select({ id: creditLedger.userId, email: user.email, model: creditLedger.model, nano: spent, source: creditLedger.usageSource, createdAt: creditLedger.createdAt })
    .from(creditLedger).leftJoin(user, eq(user.id, creditLedger.userId))
    .where(and(requests, eq(creditLedger.estimated, true), gte(creditLedger.createdAt, since))).orderBy(desc(creditLedger.createdAt)).limit(50);
  const blocked = await db.select({ id: aeroBlocks.userId, email: user.email, reason: aeroBlocks.reason, createdAt: aeroBlocks.createdAt })
    .from(aeroBlocks).leftJoin(user, eq(user.id, aeroBlocks.userId)).orderBy(desc(aeroBlocks.createdAt));
  const reversals = await db.select({ id: billingReversals.userId, email: user.email, invoiceId: billingReversals.invoiceId, reason: billingReversals.reason, createdAt: billingReversals.createdAt })
    .from(billingReversals).leftJoin(user, eq(user.id, billingReversals.userId)).orderBy(desc(billingReversals.createdAt)).limit(50);

  // Today against the global ceiling, the way admission counts it: spends and holds created today.
  const [ceilingRow] = await db.select({ nano: sql<string>`coalesce(sum(case when ${creditLedger.kind} = 'spend' then ${creditLedger.costNanoUsd} else ${creditLedger.holdNanoUsd} end), 0)` })
    .from(creditLedger).where(and(requests, gte(creditLedger.createdAt, today)));
  const ceilingUsd = Number(env.AERO_GLOBAL_DAILY_USD ?? 50);

  return {
    generatedAt: now.toISOString(),
    today: usage(todayRow as UsageRow, unit), last30: usage(last30Row as UsageRow, unit),
    models: models.map(row => ({ model: row.model ?? 'unknown', ...usage(row as UsageRow, unit) })),
    daily,
    subscriptions: { active: subs?.active ?? 0, mrrCents: (subs?.active ?? 0) * planById('aerolite').priceCents, new30: subs?.new30 ?? 0, cancelling: subs?.cancelling ?? 0, pastDue: subs?.pastDue ?? 0 },
    topAccounts,
    estimated: estimated.map(row => ({ user: { id: row.id, email: row.email }, model: row.model, costUsd: usd(Number(row.nano)), source: row.source, createdAt: row.createdAt.toISOString() })),
    blocked: blocked.map(row => ({ user: { id: row.id, email: row.email }, reason: row.reason, createdAt: row.createdAt.toISOString() })),
    ceiling: { todayUsd: usd(Number(ceilingRow?.nano ?? 0)), ceilingUsd: Number.isFinite(ceilingUsd) ? ceilingUsd : 50 },
    reversals: reversals.map(row => ({ user: { id: row.id, email: row.email }, invoiceId: row.invoiceId, reason: row.reason, createdAt: row.createdAt.toISOString() })),
  };
}
