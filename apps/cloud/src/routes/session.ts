import { ApiError } from '@powermove/registry/wire';
import type { Context } from 'hono';
import type { Session } from 'better-auth/types';
import type { Env } from '../env';
import { eq } from 'drizzle-orm';
import { publishers } from '../db/schema';
export function requireSession(c: Context<Env>): Session { const s = c.var.session; if (!s) throw new ApiError({ error: 'unauthorized' }); return s; }
export async function requirePublisher(c: Context<Env>) { const s = requireSession(c); const [publisher] = await c.var.data.db.select().from(publishers).where(eq(publishers.userId,s.userId)).limit(1); if (!publisher || publisher.tombstonedAt) throw new ApiError({ error: 'forbidden', detail: 'claim a handle first' }); return publisher; }
export async function rateAuth(c: Context<Env>): Promise<void> { const result = await c.env.RL_AUTH.limit({ key: c.req.header('cf-connecting-ip') ?? 'unknown' }); if (!result.success) throw new ApiError({ error: 'rate_limited' }); }
