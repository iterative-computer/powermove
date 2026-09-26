import { and, eq, inArray, sql } from 'drizzle-orm';
import type { Db } from '../db/client';
import { objects } from '../db/schema';
import { hasReleaseReference } from './references';

export interface PresenceContext { repoId?: string; originReleaseId?: string; basedOnReleaseId?: string }

export async function presentShas(db: Db, userId: string, shas: string[], ctx: PresenceContext, bucket: R2Bucket): Promise<Set<string>> {
  const unique = [...new Set(shas)];
  if (!unique.length) return new Set();
  const sha = sql`${objects.sha}`;
  const owned = ctx.repoId ? sql`(rel.repo_id = ${ctx.repoId} and exists (
    select 1 from publishers p where p.id = repo.owner_id and p.user_id = ${userId} and p.tombstoned_at is null
  ))` : sql`false`;
  const visibleIds = [ctx.originReleaseId, ctx.basedOnReleaseId].filter((x): x is string => !!x);
  const visible = visibleIds.length ? sql`(rel.id in (${sql.join(visibleIds.map(id => sql`${id}::uuid`), sql`, `)})
    and rel.yanked_at is null and repo.visibility in ('public','unlisted') and repo.moderation = 'none' and repo.tombstoned_at is null)` : sql`false`;
  const rows = await db.select({ sha: objects.sha, gcState: objects.gcState, authorized: sql<boolean>`(
    exists (select 1 from object_leases l where l.sha = ${sha} and l.user_id = ${userId} and l.expires_at > now())
    or ${hasReleaseReference(sha, sql`${owned} or ${visible}`)}
  )` }).from(objects).where(inArray(objects.sha, unique));
  const present = new Set<string>();
  const authorized = rows.filter(row => row.authorized && row.gcState !== 'deleting');
  let next = 0;
  await Promise.all(Array.from({length:Math.min(6,authorized.length)},async () => {
    while (next < authorized.length) {
      const row = authorized[next++]!;
      if (row.gcState === 'claimed') {
        const rescued = await db.update(objects).set({gcState:'live',lastSeenAt:new Date()}).where(and(eq(objects.sha,row.sha),eq(objects.gcState,'claimed'))).returning({sha:objects.sha});
        if (!rescued.length) continue;
      }
      if (await bucket.head(`objects/${row.sha}`)) present.add(row.sha);
    }
  }));
  return present;
}
