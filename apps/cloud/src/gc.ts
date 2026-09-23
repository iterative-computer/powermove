import { and, eq, sql } from 'drizzle-orm';
import type { Data } from './db/client';
import { desktopAuth, objectLeases, objects } from './db/schema';
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
    const [row] = await data.db.select({sha:objects.sha, referenced: sql<boolean>`${hasReleaseReference(sql`${objects.sha}`)}`,
      leased: sql<boolean>`exists (select 1 from object_leases l where l.sha = ${objects.sha} and l.expires_at > now())`})
      .from(objects).where(and(eq(objects.sha,sha),eq(objects.gcState,'claimed'))).limit(1);
    if (!row) continue;
    if (row.referenced || row.leased) {
      await data.db.update(objects).set({gcState:'live',lastSeenAt:new Date()}).where(and(eq(objects.sha,sha),eq(objects.gcState,'claimed')));
      continue;
    }
    await env.OBJECTS.delete(`objects/${sha}`);
    await data.db.delete(objects).where(and(eq(objects.sha,sha),eq(objects.gcState,'claimed'),
      sql`not ${hasReleaseReference(sql`${objects.sha}`)}`, noLease(sql`${objects.sha}`)));
  }
}

export async function runGc(data: Data, env: CloudflareBindings): Promise<void> {
  await data.db.delete(objectLeases).where(sql`${objectLeases.expiresAt} < now()`);
  await data.db.delete(desktopAuth).where(sql`${desktopAuth.expiresAt} < now() - interval '1 hour'`);
  // Resume rows left claimed by an interrupted cron before claiming new work.
  const prior = await data.db.select({sha:objects.sha}).from(objects).where(eq(objects.gcState,'claimed')).limit(500);
  const claimed = [...prior.map(row => row.sha), ...await claimGcCandidates(data, 500-prior.length)];
  await sweepClaimed(data,env,claimed);
}

export async function gc(_event: ScheduledEvent, env: CloudflareBindings): Promise<void> {
  const data = neonData(env.DATABASE_URL);
  try { await runGc(data,env); } finally { await data.end(); }
}
