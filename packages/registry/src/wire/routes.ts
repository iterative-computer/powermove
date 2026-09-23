import { z } from 'zod';
import { Category, Handle, Sha1, Slug, Uuid, Version, Visibility } from './common';
import { CompareDto, ExtensionDetailDto, InstallDto, ListingDto, MeDto, PublisherDto, ReleaseDto, SessionDto, TreeDto } from './dto';

export const CLIENT_HEADER = 'X-Powermove-Client';
export const MULTIPART_OBJECT_TYPE = 'application/x-git-loose-object';
const Empty = z.object({});
const Ok = z.object({ ok: z.literal(true) });
const NoContent = z.void();
const Coordinate = z.object({ handle: Handle, slug: Slug });
const ReleaseCoordinate = Coordinate.extend({ version: Version });
const ReleaseId = z.object({ releaseId: Uuid });
function Req<P extends z.ZodType = typeof Empty, Q extends z.ZodType = typeof Empty, B extends z.ZodType = typeof Empty>(params?: P, query?: Q, body?: B) {
  return z.object({ params: (params ?? Empty) as P, query: (query ?? Empty) as Q, body: (body ?? Empty) as B });
}
const repoList = z.object({ items: z.array(ListingDto) });
const releaseWithCoordinate = ReleaseDto.extend({ handle: Handle, slug: Slug });
const listingInput = z.object({ name: z.string().min(1).max(80), tagline: z.string().max(160), about: z.string().max(4000).optional(), category: Category, licence: z.string().max(40).optional() });
const iconPng = z.string().regex(/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/).refine((value) =>
  value.length * 3 / 4 - (value.endsWith('==') ? 2 : value.endsWith('=') ? 1 : 0) <= 256 * 1024
).optional();

export const Auth = {
  DesktopStart: { Req: Req(Empty, z.object({ provider: z.enum(['google', 'github']), state: z.string().regex(/^[a-fA-F0-9]{16,64}$/), challenge: z.string().regex(/^[A-Za-z0-9_-]+$/) })), Res: z.string() },
  DesktopDone: { Req: Req(Empty, z.object({ state: z.string() })), Res: z.string() },
  DesktopExchange: { Req: Req(Empty, Empty, z.object({ state: z.string(), token: z.string(), verifier: z.string() })), Res: SessionDto },
  EmailSend: { Req: Req(Empty, Empty, z.object({ email: z.email() })), Res: Ok },
  EmailVerify: { Req: Req(Empty, Empty, z.object({ email: z.email(), otp: z.string() })), Res: SessionDto },
  SignOut: { Req: Req(), Res: Ok }
} as const;
export const Me = {
  Get: { Req: Req(), Res: MeDto },
  SetHandle: { Req: Req(Empty, Empty, z.object({ handle: Handle })), Res: z.object({ publisher: PublisherDto }) },
  Settings: { Req: Req(Empty, Empty, z.object({ rememberInstalls: z.boolean().optional() })), Res: z.object({ settings: z.object({ rememberInstalls: z.boolean() }) }) },
  Delete: { Req: Req(), Res: NoContent },
  Repos: { Req: Req(), Res: repoList }
} as const;
export const Objects = {
  Missing: { Req: Req(Empty, Empty, z.object({ shas: z.array(Sha1).max(1000), repoId: Uuid.optional(), originReleaseId: Uuid.optional(), basedOnReleaseId: Uuid.optional() })), Res: z.object({ missing: z.array(Sha1) }) },
  Upload: { Req: Req(Empty, Empty, z.instanceof(FormData)), Res: z.object({ stored: z.array(Sha1), present: z.array(Sha1), rejected: z.array(z.object({ sha: Sha1, code: z.enum(['hash_mismatch', 'bad_object', 'too_large', 'object_conflict']) })) }) }
} as const;
export const Store = {
  Browse: { Req: Req(), Res: z.object({ sections: z.array(z.object({ id: z.union([z.enum(['featured', 'picks', 'new']), Category]), title: z.string(), items: z.array(ListingDto) })) }) },
  Extensions: { Req: Req(Empty, z.object({ category: Category.optional(), q: z.string().optional(), sort: z.enum(['new', 'installs', 'name']).optional(), cursor: z.string().optional(), limit: z.coerce.number().int().min(1).max(50).optional() })), Res: z.object({ items: z.array(ListingDto), nextCursor: z.string().nullable() }) },
  Detail: { Req: Req(Coordinate), Res: ExtensionDetailDto },
  Release: { Req: Req(ReleaseCoordinate), Res: ReleaseDto },
  Tree: { Req: Req(ReleaseCoordinate), Res: TreeDto },
  Tar: { Req: Req(ReleaseCoordinate), Res: z.instanceof(Uint8Array) },
  File: { Req: Req(ReleaseCoordinate.extend({ path: z.string() })), Res: z.string() },
  ReleaseById: { Req: Req(ReleaseId), Res: releaseWithCoordinate },
  TreeById: { Req: Req(ReleaseId), Res: TreeDto },
  TarById: { Req: Req(ReleaseId), Res: z.instanceof(Uint8Array) },
  Compare: { Req: Req(Empty, z.object({ base: Uuid, head: Uuid })), Res: CompareDto },
  Versions: { Req: Req(Empty, Empty, z.object({ items: z.array(z.object({ repoId: Uuid, releaseId: Uuid })).max(200) })), Res: z.object({ items: z.array(z.object({ repoId: Uuid, state: z.enum(['ok', 'hidden', 'removed', 'tombstoned']), ownerPublisherId: Uuid.nullable(), current: z.object({ yanked: z.boolean() }), latest: z.object({ releaseId: Uuid, version: Version, treeSha: Sha1, tarSha256: z.string().regex(/^[a-f0-9]{64}$/), apiVersion: z.number().int().positive() }).nullable(), handle: Handle.nullable(), slug: Slug.nullable() })) }) },
  Icon: { Req: Req(z.object({ key: z.string() })), Res: z.instanceof(Uint8Array) },
  Report: { Req: Req(Coordinate, Empty, z.object({ reason: z.string().max(1000) })), Res: NoContent }
} as const;
export const Installs = {
  List: { Req: Req(), Res: z.object({ items: z.array(InstallDto) }) },
  Add: { Req: Req(Empty, Empty, z.object({ repoId: Uuid, releaseId: Uuid })), Res: NoContent },
  Delete: { Req: Req(z.object({ repoId: Uuid })), Res: NoContent }
} as const;
export const Publish = {
  PutRelease: { Req: Req(Coordinate, Empty, z.object({ version: Version, commitSha: Sha1, notes: z.string().max(4000).optional(), listing: listingInput, visibility: Visibility.optional(), iconPng, originReleaseId: Uuid.optional(), basedOnReleaseId: Uuid.optional(), waivers: z.array(z.object({ path: z.string(), line: z.number().int().positive(), reason: z.string().min(3).max(200) })) })), Res: z.object({ repo: ListingDto, release: ReleaseDto }) },
  Yank: { Req: Req(ReleaseCoordinate), Res: z.object({ repo: ListingDto, release: ReleaseDto }) },
  PatchRepo: { Req: Req(Coordinate, Empty, z.object({ visibility: Visibility.optional(), listing: listingInput.partial().optional(), iconPng })), Res: ListingDto },
  DeleteRepo: { Req: Req(Coordinate), Res: NoContent }
} as const;
export const Admin = {
  /** Seeds a reserved handle (e.g. `powermove`) for an existing user; bypasses the reserved list. */
  SeedPublisher: { Req: Req(Empty, Empty, z.object({ handle: Handle, userId: z.string().min(1) })), Res: z.object({ publisher: PublisherDto }) },
  Moderate: { Req: Req(z.object({ repoId: Uuid }), Empty, z.object({ action: z.enum(['hide', 'unhide', 'remove']), reason: z.string().max(1000) })), Res: ListingDto }
} as const;
