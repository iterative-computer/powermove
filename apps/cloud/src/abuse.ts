import { sql } from 'drizzle-orm';
import type { Context } from 'hono';
import { ApiError } from '@powermove/registry/wire';
import type { Db } from './db/client';
import { abuseCounters } from './db/schema';
import type { Env } from './env';

export type Scope = 'email_send_addr' | 'email_send_ip' | 'email_send_global' | 'email_verify_ip' | 'email_verify_addr'
  | 'auth_start_ip' | 'exchange_ip' | 'handle_claim_user' | 'publish_user_hour' | 'publish_user_day'
  | 'human_verify_ip' | 'upload_req_user' | 'missing_req_user' | 'report_ip' | 'report_repo_ip' | 'install_user' | 'admin_ip';
export interface Rule { scope: Scope; limit: number; windowSeconds: number }
export const RULES: Record<Scope, Omit<Rule, 'scope'>> = {
  human_verify_ip: { limit: 30, windowSeconds: 3600 },
  email_send_addr: { limit: 3, windowSeconds: 900 },
  email_send_ip: { limit: 20, windowSeconds: 3600 },
  email_send_global: { limit: 5000, windowSeconds: 86400 },
  email_verify_addr: { limit: 5, windowSeconds: 900 },
  email_verify_ip: { limit: 30, windowSeconds: 3600 },
  auth_start_ip: { limit: 30, windowSeconds: 3600 },
  exchange_ip: { limit: 30, windowSeconds: 3600 },
  handle_claim_user: { limit: 5, windowSeconds: 86400 },
  publish_user_hour: { limit: 20, windowSeconds: 3600 },
  publish_user_day: { limit: 100, windowSeconds: 86400 },
  upload_req_user: { limit: 600, windowSeconds: 3600 },
  missing_req_user: { limit: 600, windowSeconds: 3600 },
  report_ip: { limit: 10, windowSeconds: 3600 },
  report_repo_ip: { limit: 1, windowSeconds: 86400 },
  install_user: { limit: 600, windowSeconds: 3600 },
  admin_ip: { limit: 60, windowSeconds: 3600 },
};

export function clientIp(c: Context<Env>): string { return c.req.header('cf-connecting-ip') ?? 'unknown'; }
export function rateLimited(c: Context<Env>, retryAfterSeconds: number): never {
  c.header('Retry-After', String(retryAfterSeconds));
  throw new ApiError({ error: 'rate_limited' });
}

export async function consume(db: Db, scope: Scope, key: string, now = new Date()): Promise<
  { ok: true; remaining: number } | { ok: false; retryAfterSeconds: number }
> {
  const { limit, windowSeconds } = RULES[scope];
  const hash = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(key))))
    .map(byte => byte.toString(16).padStart(2, '0')).join('').slice(0, 32);
  const windowStart = sql<Date>`to_timestamp(floor(extract(epoch from ${now.toISOString()}::timestamptz) / ${windowSeconds}) * ${windowSeconds})`;
  const [row] = await db.insert(abuseCounters).values({ scope, key: hash, windowStart, count: 1 })
    .onConflictDoUpdate({ target: [abuseCounters.scope, abuseCounters.key, abuseCounters.windowStart],
      set: { count: sql`${abuseCounters.count} + 1` } })
    .returning({ count: abuseCounters.count });
  const count = row!.count;
  if (count <= limit) return { ok: true, remaining: limit - count };
  return { ok: false, retryAfterSeconds: Math.max(1, Math.ceil((windowSeconds * 1000 - (now.getTime() % (windowSeconds * 1000))) / 1000)) };
}

export async function enforce(c: Context<Env>, scope: Scope, key: string): Promise<void> {
  const result = await consume(c.var.data.db, scope, key);
  if (!result.ok) {
    if (scope === 'email_send_global') console.error('email ceiling reached');
    rateLimited(c, result.retryAfterSeconds);
  }
}
