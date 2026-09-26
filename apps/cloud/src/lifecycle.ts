export type RepoState = { visibility: 'public' | 'unlisted'; moderation: 'none' | 'hidden' | 'removed'; tombstonedAt: Date | null; ownerId: string };
export type Viewer = { publisherId: string | null; admin: boolean };
type DetailDecision = 'ok' | 'gone_removed' | 'gone_tombstoned' | 'not_found';
export function canBrowse(repo: RepoState, _viewer: Viewer): boolean {
  return repo.visibility === 'public' && repo.moderation === 'none' && repo.tombstonedAt === null;
}
export function canViewDetail(repo: RepoState, viewer: Viewer): DetailDecision {
  if (repo.moderation === 'removed') return 'gone_removed';
  if (repo.tombstonedAt) return 'gone_tombstoned';
  if (repo.moderation === 'hidden' && !viewer.admin && viewer.publisherId !== repo.ownerId) return 'not_found';
  return 'ok';
}
export function canReadFiles(repo: RepoState, release: { yankedAt: Date | null }, viewer: Viewer): DetailDecision | 'gone_yanked' {
  const detail = canViewDetail(repo, viewer);
  return detail === 'ok' && release.yankedAt ? 'gone_yanked' : detail;
}
export function updateOffered(repo: RepoState, release: { yankedAt: Date | null }): boolean {
  return repo.moderation === 'none' && repo.tombstonedAt === null && release.yankedAt === null;
}
