import { and, desc, eq, gte, lt, notInArray, sql } from 'drizzle-orm';
import type { Db } from '../db/client';
import { creditLedger, subscriptions } from '../db/schema';
import { PLANS, isPlanId, planById } from './plans';

export type Status = 'active' | 'past_due' | 'canceled';
/** Stripe's subscription statuses as the app sees them. `incomplete` (Checkout not paid yet) is no plan. */
export function publicStatus(stripeStatus: string): Status | null {
  if (stripeStatus === 'active' || stripeStatus === 'trialing') return 'active';
  if (stripeStatus === 'past_due' || stripeStatus === 'unpaid' || stripeStatus === 'paused') return 'past_due';
  if (stripeStatus === 'canceled' || stripeStatus === 'incomplete_expired') return 'canceled';
  return null;
}

/** The subscription that decides the account's plan: the newest live one, else the newest ended one. */
export async function currentSubscription(db: Db, userId: string) {
  const [live] = await db.select().from(subscriptions)
    .where(and(eq(subscriptions.userId, userId), notInArray(subscriptions.status, ['canceled', 'incomplete_expired', 'incomplete'])))
    .orderBy(desc(subscriptions.currentPeriodStart)).limit(1);
  if (live) return live;
  const [ended] = await db.select().from(subscriptions).where(eq(subscriptions.userId, userId))
    .orderBy(desc(subscriptions.currentPeriodEnd)).limit(1);
  return ended ?? null;
}

export interface Credits { included: number; used: number; remaining: number }
export interface Period { start: Date; end: Date }

/** Credits of the current period: its grants minus the spends made inside it. Nothing carries over. */
export async function periodCredits(db: Db, userId: string, period: Period): Promise<Credits> {
  const [row] = await db.select({
    included: sql<string>`coalesce(sum(${creditLedger.credits}) filter (where ${creditLedger.kind} = 'grant' and ${creditLedger.periodStart} = ${period.start.toISOString()}::timestamptz), 0)`,
    used: sql<string>`coalesce(sum(${creditLedger.credits}) filter (where ${creditLedger.kind} = 'spend' and ${creditLedger.createdAt} >= ${period.start.toISOString()}::timestamptz and ${creditLedger.createdAt} < ${period.end.toISOString()}::timestamptz), 0)`,
  }).from(creditLedger).where(eq(creditLedger.userId, userId));
  const included = Number(row?.included ?? 0), used = Number(row?.used ?? 0);
  return { included, used, remaining: included - used };
}

export interface BillingState {
  plan: { id: string; name: string; priceCents: number; interval: 'month' } | null;
  status: Status | null;
  cancelAtPeriodEnd: boolean;
  credits: Credits;
  period: { start: string; end: string } | null;
  daily: Array<{ date: string; credits: number }>;
  plans: Array<{ id: string; name: string; priceCents: number; credits: number }>;
}

const day = (date: Date) => date.toISOString().slice(0, 10);

export async function billingState(db: Db, userId: string, now = new Date()): Promise<BillingState> {
  const plans = PLANS.map(({ id, name, priceCents, credits }) => ({ id, name, priceCents, credits }));
  const sub = await currentSubscription(db, userId);
  const status = sub ? publicStatus(sub.status) : null;
  if (!sub || !status || status === 'canceled' || !isPlanId(sub.plan)) {
    return { plan: null, status: status === 'canceled' ? 'canceled' : null, cancelAtPeriodEnd: false, credits: { included: 0, used: 0, remaining: 0 }, period: null, daily: [], plans };
  }
  const plan = planById(sub.plan);
  const period = { start: sub.currentPeriodStart, end: sub.currentPeriodEnd };
  const credits = await periodCredits(db, userId, period);
  const rows = await db.select({
    date: sql<string>`to_char(${creditLedger.createdAt} at time zone 'UTC', 'YYYY-MM-DD')`,
    credits: sql<string>`sum(${creditLedger.credits})`,
  }).from(creditLedger).where(and(eq(creditLedger.userId, userId), eq(creditLedger.kind, 'spend'),
    gte(creditLedger.createdAt, period.start), lt(creditLedger.createdAt, period.end)))
    .groupBy(sql`1`);
  const byDay = new Map(rows.map(row => [row.date, Number(row.credits)]));
  const daily: BillingState['daily'] = [];
  const last = day(now < period.end ? now : new Date(period.end.getTime() - 1));
  for (let cursor = new Date(`${day(period.start)}T00:00:00Z`); day(cursor) <= last; cursor = new Date(cursor.getTime() + 86_400_000)) {
    daily.push({ date: day(cursor), credits: byDay.get(day(cursor)) ?? 0 });
  }
  return {
    plan: { id: plan.id, name: plan.name, priceCents: plan.priceCents, interval: 'month' },
    status,
    cancelAtPeriodEnd: sub.cancelAtPeriodEnd,
    credits: { ...credits, remaining: Math.max(0, credits.remaining) },
    period: { start: period.start.toISOString(), end: period.end.toISOString() },
    daily,
    plans,
  };
}

/** What an Aero request may spend now: the live plan's period credits, or nothing. */
export async function spendable(db: Db, userId: string): Promise<{ remaining: number; subscriptionId: string | null }> {
  const sub = await currentSubscription(db, userId);
  const status = sub ? publicStatus(sub.status) : null;
  if (!sub || (status !== 'active' && status !== 'past_due')) return { remaining: 0, subscriptionId: null };
  const credits = await periodCredits(db, userId, { start: sub.currentPeriodStart, end: sub.currentPeriodEnd });
  return { remaining: credits.remaining, subscriptionId: sub.id };
}

export interface Spend {
  userId: string;
  requestId: string;
  subscriptionId: string | null;
  model: string;
  inputTokens: number;
  cachedInputTokens: number;
  cacheWriteTokens: number;
  outputTokens: number;
  costMicroUsd: number;
  credits: number;
  estimated: boolean;
}

/** One ledger row per request id; a replay of the same id writes nothing. Returns whether this call wrote it. */
export async function recordSpend(db: Db, spend: Spend): Promise<boolean> {
  const rows = await db.insert(creditLedger).values({
    userId: spend.userId, kind: 'spend', idempotencyKey: `request:${spend.requestId}`, credits: spend.credits,
    subscriptionId: spend.subscriptionId, model: spend.model, inputTokens: spend.inputTokens,
    cachedInputTokens: spend.cachedInputTokens, cacheWriteTokens: spend.cacheWriteTokens, outputTokens: spend.outputTokens,
    costMicroUsd: spend.costMicroUsd, estimated: spend.estimated,
  }).onConflictDoNothing({ target: creditLedger.idempotencyKey }).returning({ id: creditLedger.id });
  return rows.length === 1;
}

/** Grants a period's credits, once per key (the invoice id). */
export async function recordGrant(db: Db, grant: { userId: string; key: string; credits: number; subscriptionId: string; period: Period }): Promise<boolean> {
  const rows = await db.insert(creditLedger).values({
    userId: grant.userId, kind: 'grant', idempotencyKey: grant.key, credits: grant.credits, subscriptionId: grant.subscriptionId,
    periodStart: grant.period.start, periodEnd: grant.period.end,
  }).onConflictDoNothing({ target: creditLedger.idempotencyKey }).returning({ id: creditLedger.id });
  return rows.length === 1;
}

/** Credits already granted for the period that starts at `start`. */
export async function grantedFor(db: Db, userId: string, start: Date): Promise<number> {
  const [row] = await db.select({ total: sql<string>`coalesce(sum(${creditLedger.credits}), 0)` }).from(creditLedger)
    .where(and(eq(creditLedger.userId, userId), eq(creditLedger.kind, 'grant'), eq(creditLedger.periodStart, start)));
  return Number(row?.total ?? 0);
}
