import { z } from 'zod';
import { Category, Handle, IsoDate, Moderation, Permission, Sha1, Sha256, Slug, Uuid, VarDecl, Version, Visibility } from './common';

export const PublisherDto = z.object({ id: Uuid, handle: Handle, tombstoned: z.boolean() });
export type PublisherDto = z.infer<typeof PublisherDto>;
export const LineageDto = z.object({ repoId: Uuid, handle: Handle, slug: Slug, releaseId: Uuid, version: Version });
export type LineageDto = z.infer<typeof LineageDto>;
export const ReleaseSummaryDto = z.object({ id: Uuid, version: Version, publishedAt: IsoDate, apiVersion: z.number().int().positive(), yankedAt: IsoDate.nullable() });
export type ReleaseSummaryDto = z.infer<typeof ReleaseSummaryDto>;
export const ListingDto = z.object({
  repoId: Uuid, owner: PublisherDto, slug: Slug, name: z.string(), tagline: z.string(), category: Category,
  iconUrl: z.string().nullable(), visibility: Visibility, latest: ReleaseSummaryDto.nullable(),
  permissions: z.array(Permission),
  installCount: z.number().int().nonnegative(), forkCount: z.number().int().nonnegative(), licence: z.string(),
  forkedFrom: LineageDto.nullable(), createdAt: IsoDate, updatedAt: IsoDate
});
export type ListingDto = z.infer<typeof ListingDto>;
export const ManifestSummaryDto = z.object({
  id: Slug, name: z.string(), version: Version, apiVersion: z.number().int().positive(),
  contributes: z.array(z.string()), vars: z.array(VarDecl), permissions: z.array(Permission), forkedFrom: z.string().nullable(), description: z.string().nullable()
});
export type ManifestSummaryDto = z.infer<typeof ManifestSummaryDto>;
export const ReleaseDto = z.object({
  id: Uuid, repoId: Uuid, version: Version, commitSha: Sha1, treeSha: Sha1, tarSha256: Sha256,
  apiVersion: z.number().int().positive(), fileCount: z.number().int().nonnegative(), sizeBytes: z.number().int().nonnegative(),
  notes: z.string().nullable(), publishedAt: IsoDate, yankedAt: IsoDate.nullable(), basedOnReleaseId: Uuid.nullable(), manifest: ManifestSummaryDto
});
export type ReleaseDto = z.infer<typeof ReleaseDto>;
export const ExtensionDetailDto = ListingDto.extend({ about: z.string().nullable(), releases: z.array(ReleaseDto), moderation: Moderation });
export type ExtensionDetailDto = z.infer<typeof ExtensionDetailDto>;
export const TreeFileDto = z.object({ path: z.string(), sha: Sha1, size: z.number().int().nonnegative() });
export type TreeFileDto = z.infer<typeof TreeFileDto>;
export const TreeDto = z.object({ treeSha: Sha1, files: z.array(TreeFileDto) });
export type TreeDto = z.infer<typeof TreeDto>;
export const CompareStatus = z.enum(['added', 'removed', 'modified']);
export type CompareStatus = z.infer<typeof CompareStatus>;
export const CompareDto = z.object({ baseReleaseId: Uuid, headReleaseId: Uuid, files: z.array(z.object({ path: z.string(), status: CompareStatus })), counts: z.object({ added: z.number().int().nonnegative(), removed: z.number().int().nonnegative(), modified: z.number().int().nonnegative() }) });
export type CompareDto = z.infer<typeof CompareDto>;
export const InstallDto = z.object({ repoId: Uuid, releaseId: Uuid, handle: Handle, slug: Slug, installedAt: IsoDate });
export type InstallDto = z.infer<typeof InstallDto>;
export const MeDto = z.object({ user: z.object({ id: z.string().min(1), name: z.string().nullable(), email: z.email(), image: z.string().nullable() }), publisher: PublisherDto.nullable(), settings: z.object({ rememberInstalls: z.boolean() }) });
export type MeDto = z.infer<typeof MeDto>;
export const SessionDto = z.object({ token: z.string(), expiresAt: IsoDate });
export type SessionDto = z.infer<typeof SessionDto>;
