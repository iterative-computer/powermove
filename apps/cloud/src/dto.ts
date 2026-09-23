import type { ListingDto } from '@powermove/registry/wire';
import type { InferSelectModel } from 'drizzle-orm';
import type { extensions, publishers, repos, releases } from './db/schema';
type Row = { repo: InferSelectModel<typeof repos>; extension: InferSelectModel<typeof extensions>; owner: InferSelectModel<typeof publishers>; latest: InferSelectModel<typeof releases> | null };
export function toListing(row: Row): ListingDto {
  const { repo, extension, owner, latest } = row;
  return {
    repoId: repo.id, owner: { id: owner.id, handle: owner.handle, tombstoned: owner.tombstonedAt !== null }, slug: repo.slug,
    name: extension.name, tagline: extension.tagline, category: extension.category as ListingDto['category'],
    iconUrl: extension.iconKey ? `/v1/store/icons/${extension.iconKey}` : null, visibility: repo.visibility,
    latest: latest ? { id: latest.id, version: latest.version, publishedAt: latest.publishedAt.toISOString(), apiVersion: latest.apiVersion, yankedAt: latest.yankedAt?.toISOString() ?? null } : null,
    installCount: extension.installCount, forkCount: extension.forkCount, licence: extension.licence, forkedFrom: null,
    createdAt: repo.createdAt.toISOString(), updatedAt: repo.updatedAt.toISOString()
  } satisfies ListingDto;
}
