import { and, eq, sql } from 'drizzle-orm';
import type { Data } from './db/client';
import { abuseCounters, desktopAuth, objectLeases, objects } from './db/schema';
import { hasReleaseReference } from './objects/references';
import { neonData } from './db/client';

const noLease = (sha: ReturnType<typeof sql>) => sql`not exists (
  select 1 from object_leases l where l.sha = ${sha} and l.expires_at > now()
)`;

export async function claimGcCandidates(data: Data, limit = 500): Promise<string[]> {
  const sha = sql`candidate.sha`;
  const rows = await data.db.update(objects).set({gcState:'claimed'})
    .where(sql`${objects.sha} in (
      select candidate.sha from objects candidate
      where candidate.gc_state = 'live' and candidate.last_seen_at < now() - interval '24 hours'
        and not ${hasReleaseReference(sha)} and ${noLease(sha)}
      limit ${limit}
    )`).returning({sha:objects.sha});
  return rows.map(row => row.sha);
}

export async function sweepClaimed(data: Data, env: CloudflareBindings, shas: string[]): Promise<void> {
  for (const sha of shas) {
    const deleting = await data.tx(async tx => {
      const [row] = await tx.select({sha:objects.sha,gcState:objects.gcState,referenced:sql<boolean>`${hasReleaseReference(sql`${objects.sha}`)}`,
        leased:sql<boolean>`exists (select 1 from object_leases l where l.sha = ${objects.sha} and l.expires_at > now())`})
        .from(objects).where(eq(objects.sha,sha)).for('update').limit(1);
      if (!row) return false;
      if (row.gcState === 'deleting') return true;
      if (row.gcState !== 'claimed') return false;
      if (row.referenced || row.leased) {
        await tx.update(objects).set({gcState:'live',lastSeenAt:new Date()}).where(eq(objects.sha,sha));
        return false;
      }
      await tx.update(objects).set({gcState:'deleting'}).where(eq(objects.sha,sha));
      return true;
    });
    if (!deleting) continue;
    await env.OBJECTS.delete(`objects/${sha}`);
    await data.tx(async tx => {
      await tx.delete(objectLeases).where(and(eq(objectLeases.sha,sha),sql`${objectLeases.expiresAt} <= now()`));
      await tx.delete(objects).where(and(eq(objects.sha,sha),eq(objects.gcState,'deleting')));
    });
  }
}

export async function runGc(data: Data, env: CloudflareBindings): Promise<void> {
  await data.db.delete(objectLeases).where(sql`${objectLeases.expiresAt} < now()`);
  await data.db.delete(desktopAuth).where(sql`${desktopAuth.expiresAt} < now() - interval '1 hour'`);
  await data.db.delete(abuseCounters).where(sql`${abuseCounters.windowStart} < now() - interval '2 days'`);
  // Resume rows left claimed by an interrupted cron before claiming new work.
  const prior = await data.db.select({sha:objects.sha}).from(objects).where(sql`${objects.gcState} in ('claimed','deleting')`).limit(500);
  const claimed = [...prior.map(row => row.sha), ...await claimGcCandidates(data, 500-prior.length)];
  await sweepClaimed(data,env,claimed);
}

export async function gc(_event: ScheduledEvent, env: CloudflareBindings): Promise<void> {
  const data = neonData(env.DATABASE_URL);
  try { await runGc(data,env); } finally { await data.end(); }
}
