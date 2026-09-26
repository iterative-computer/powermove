import { eq } from 'drizzle-orm';
import { publishers as publishersTable, repos as reposTable, releases as releasesTable } from './db/schema';
import type { Db } from './db/client';
import type { ExtensionDetailDto, LineageDto, ListingDto, ReleaseDto } from '@powermove/registry/wire';
import type { InferSelectModel } from 'drizzle-orm';
import type { extensions, publishers, repos, releases } from './db/schema';
import type { ExtensionManifest } from '@powermove/registry/manifest';
type Repo = InferSelectModel<typeof repos>;
type Extension = InferSelectModel<typeof extensions>;
type Owner = InferSelectModel<typeof publishers>;
type Release = InferSelectModel<typeof releases>;
export function toListing(repo: Repo, extension: Extension, owner: Owner, latest: Release | null, lineage: LineageDto | null = null): ListingDto {
  return {
    repoId: repo.id, owner: { id: owner.id, handle: owner.handle, tombstoned: owner.tombstonedAt !== null }, slug: repo.slug,
    name: extension.name, tagline: extension.tagline, category: extension.category as ListingDto['category'],
    iconUrl: extension.iconKey ? `/v1/store/icons/${extension.iconKey}` : null, visibility: repo.visibility,
    latest: latest ? { id: latest.id, version: latest.version, publishedAt: latest.publishedAt.toISOString(), apiVersion: latest.apiVersion, yankedAt: latest.yankedAt?.toISOString() ?? null } : null,
    permissions: extension.permissions,
    installCount: extension.installCount, forkCount: extension.forkCount, licence: extension.licence, forkedFrom: lineage,
    createdAt: repo.createdAt.toISOString(), updatedAt: repo.updatedAt.toISOString()
  } satisfies ListingDto;
}
export function toRelease(row: Release): ReleaseDto {
  const manifest = row.manifest as ExtensionManifest;
  return {
    id: row.id, repoId: row.repoId, version: row.version, commitSha: row.commitSha, treeSha: row.treeSha, tarSha256: row.tarSha256,
    apiVersion: row.apiVersion, fileCount: row.fileCount, sizeBytes: row.sizeBytes, notes: row.notes,
    publishedAt: row.publishedAt.toISOString(), yankedAt: row.yankedAt?.toISOString() ?? null, basedOnReleaseId: row.basedOnReleaseId,
    manifest: { id: manifest.id, name: manifest.name, version: manifest.version, apiVersion: manifest.apiVersion,
      contributes: manifest.contributes ?? [], vars: manifest.vars ?? [], permissions: row.permissions, forkedFrom: manifest.forkedFrom ?? null, description: manifest.description ?? null }
  } satisfies ReleaseDto;
}
export function toDetail(repo: Repo, extension: Extension, owner: Owner, latest: Release | null, lineage: LineageDto | null, releaseRows: Release[]): ExtensionDetailDto {
  return { ...toListing(repo, extension, owner, latest, lineage), about: extension.about, releases: releaseRows.map(toRelease), moderation: repo.moderation } satisfies ExtensionDetailDto;
}

export async function lineageFor(db: Db, repo: Repo): Promise<LineageDto | null> {
  if (!repo.forkedFromRepoId || !repo.forkedFromReleaseId) return null;
  const [origin] = await db.select().from(reposTable).where(eq(reposTable.id, repo.forkedFromRepoId)).limit(1);
  const [release] = await db.select().from(releasesTable).where(eq(releasesTable.id, repo.forkedFromReleaseId)).limit(1);
  if (!origin || !release) return null;
  const [owner] = await db.select().from(publishersTable).where(eq(publishersTable.id, origin.ownerId)).limit(1);
  return owner ? { repoId: origin.id, handle: owner.handle, slug: origin.slug, releaseId: release.id, version: release.version } satisfies LineageDto : null;
}
