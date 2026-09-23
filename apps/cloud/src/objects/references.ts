import { sql, type SQL } from 'drizzle-orm';

/** The single definition of release reachability used by presence and both GC checks. */
export function hasReleaseReference(sha: SQL, visibility?: SQL): SQL {
  return sql`exists (
    select 1 from release_objects ro
    join releases rel on rel.id = ro.release_id
    join repos repo on repo.id = rel.repo_id
    where ro.sha = ${sha} ${visibility ? sql`and (${visibility})` : sql``}
  )`;
}
