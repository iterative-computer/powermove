import { and, desc, eq, gte, lt, notInArray, sql } from 'drizzle-orm';
import type { Db } from '../db/client';
import { creditLedger, subscriptions } from '../db/schema';
import { PLANS, isPlanId, planById } from './plans';
import { CREDIT_NANO_USD } from '../aero/catalogue';

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
/** A period's money in exact nano-USD: what its grants include, what its requests have spent, and what running
    requests hold. */
export interface Balance { includedNano: number; usedNano: number; heldNano: number; remainingNano: number }

/** The period's requests: the holds and spends admitted in it, wherever they settle. */
const inPeriod = (period: Period) => sql`${creditLedger.kind} in ('spend', 'hold') and ${creditLedger.periodStart} = ${period.start.toISOString()}::timestamptz`;
/** A request's share of the balance. A spend is its cost. A hold counts in full until it settles: a live one as
    reserved, one older than its expiry (its call never settled) as spent, the safe side. */
const live = sql`${creditLedger.kind} = 'hold' and ${creditLedger.holdExpiresAt} > now()`;
const spent = sql`case when ${creditLedger.kind} = 'spend' then ${creditLedger.costNanoUsd} when ${live} then 0 else ${creditLedger.holdNanoUsd} end`;
const reserved = sql`case when ${live} then ${creditLedger.holdNanoUsd} else 0 end`;

/** The period's balance: its grants minus the requests made inside it. Nothing carries over. */
export async function periodBalance(db: Db, userId: string, period: Period): Promise<Balance> {
  const [row] = await db.select({
    included: sql<string>`coalesce(sum(${creditLedger.credits}) filter (where ${creditLedger.kind} = 'grant' and ${creditLedger.periodStart} = ${period.start.toISOString()}::timestamptz), 0)`,
    used: sql<string>`coalesce(sum(${spent}) filter (where ${inPeriod(period)}), 0)`,
    held: sql<string>`coalesce(sum(${reserved}) filter (where ${inPeriod(period)}), 0)`,
  }).from(creditLedger).where(eq(creditLedger.userId, userId));
  const includedNano = Number(row?.included ?? 0) * CREDIT_NANO_USD, usedNano = Number(row?.used ?? 0), heldNano = Number(row?.held ?? 0);
  return { includedNano, usedNano, heldNano, remainingNano: includedNano - usedNano - heldNano };
}

/** Whole credits to show, derived from the exact balance (rounding is display only, never charged): `used` rounds
    up and `remaining` down, so the two add up to `included` whenever nothing is held, and the account never shows a
    credit it doesn't have. */
export function displayCredits(balance: Balance): Credits {
  const included = balance.includedNano / CREDIT_NANO_USD;
  return { included, used: Math.ceil(balance.usedNano / CREDIT_NANO_USD), remaining: Math.max(0, Math.floor(balance.remainingNano / CREDIT_NANO_USD)) };
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
  const plans = PLANS.filter(plan => plan.forSale).map(({ id, name, priceCents, credits }) => ({ id, name, priceCents, credits }));
  const sub = await currentSubscription(db, userId);
  const status = sub ? publicStatus(sub.status) : null;
  if (!sub || !status || status === 'canceled' || !isPlanId(sub.plan)) {
    return { plan: null, status: status === 'canceled' ? 'canceled' : null, cancelAtPeriodEnd: false, credits: { included: 0, used: 0, remaining: 0 }, period: null, daily: [], plans };
  }
  const plan = planById(sub.plan);
  const period = { start: sub.currentPeriodStart, end: sub.currentPeriodEnd };
  const balance = await periodBalance(db, userId, period);
  const rows = await db.select({
    date: sql<string>`to_char(${creditLedger.createdAt} at time zone 'UTC', 'YYYY-MM-DD')`,
    nano: sql<string>`sum(${spent})`,
  }).from(creditLedger).where(and(eq(creditLedger.userId, userId), inPeriod(period)))
    .groupBy(sql`1`);
  const byDay = new Map(rows.map(row => [row.date, Number(row.nano)]));
  // Each day shows the whole credits its spending crossed (the running total's rounded-up credits, day over day),
  // so the days add up to `used` exactly.
  const daily: BillingState['daily'] = [];
  const last = day(now < period.end ? now : new Date(period.end.getTime() - 1));
  let total = 0, shown = 0;
  for (let cursor = new Date(`${day(period.start)}T00:00:00Z`); day(cursor) <= last; cursor = new Date(cursor.getTime() + 86_400_000)) {
    total += byDay.get(day(cursor)) ?? 0;
    const upTo = Math.ceil(total / CREDIT_NANO_USD);
    daily.push({ date: day(cursor), credits: upTo - shown });
    shown = upTo;
  }
  return {
    plan: { id: plan.id, name: plan.name, priceCents: plan.priceCents, interval: 'month' },
    status,
    cancelAtPeriodEnd: sub.cancelAtPeriodEnd,
    credits: displayCredits(balance),
    period: { start: period.start.toISOString(), end: period.end.toISOString() },
    daily,
    plans,
  };
}

/** Serialises everything that changes what an account may spend (admitting a request, a subscription's period
    changing) for the rest of the transaction. */
export async function lockBalance(tx: Db, userId: string) {
  await tx.execute(sql`select pg_advisory_xact_lock(hashtext(${`aero-balance:${userId}`}))`);
}

/** The subscription and period an Aero request spends against now, by the database's clock (sampled with the
    subscription, after any lock the caller holds): a live plan whose period hasn't ended. Past the period's end,
    before the renewal arrives, there is nothing to spend against. */
export async function spendingPeriod(db: Db, userId: string): Promise<{ subscriptionId: string; period: Period } | null> {
  const [sub] = await db.select({
    id: subscriptions.id, status: subscriptions.status, start: subscriptions.currentPeriodStart, end: subscriptions.currentPeriodEnd,
    live: sql<boolean>`${subscriptions.currentPeriodStart} <= clock_timestamp() and clock_timestamp() < ${subscriptions.currentPeriodEnd}`,
  }).from(subscriptions)
    .where(and(eq(subscriptions.userId, userId), notInArray(subscriptions.status, ['canceled', 'incomplete_expired', 'incomplete'])))
    .orderBy(desc(subscriptions.currentPeriodStart)).limit(1);
  const status = sub ? publicStatus(sub.status) : null;
  if (!sub || (status !== 'active' && status !== 'past_due') || !sub.live) return null;
  return { subscriptionId: sub.id, period: { start: sub.start, end: sub.end } };
}

/** What an Aero request may spend now, in nano-USD: the current period's balance less running holds, or nothing. */
export async function spendable(db: Db, userId: string): Promise<{ remainingNano: number; subscriptionId: string | null; period: Period | null }> {
  const current = await spendingPeriod(db, userId);
  if (!current) return { remainingNano: 0, subscriptionId: null, period: null };
  return { remainingNano: (await periodBalance(db, userId, current.period)).remainingNano, ...current };
}

/** Holds past their expiry become spends of the whole hold, marked estimated: their calls never settled (an
    object killed before its alarm was armed, a database down past the hold's life). Conditional, so a hold that
    settles meanwhile keeps its own spend. Returns how many it converted. */
export async function sweepHolds(db: Db): Promise<number> {
  const swept = await db.update(creditLedger).set({ kind: 'spend', costNanoUsd: sql`${creditLedger.holdNanoUsd}`, estimated: true, usageSource: 'hold' })
    .where(and(eq(creditLedger.kind, 'hold'), sql`${creditLedger.holdExpiresAt} < now()`)).returning({ id: creditLedger.id });
  return swept.length;
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
