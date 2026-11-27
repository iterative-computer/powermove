import { z } from 'zod';
import { Category, Handle, IsoDate, Sha1, Slug, Uuid, Version, Visibility } from './common';
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
  DesktopStart: { Req: Req(Empty, z.object({ provider: z.enum(['google']), state: z.string().regex(/^[a-fA-F0-9]{16,64}$/), challenge: z.string().regex(/^[A-Za-z0-9_-]+$/) })), Res: z.string() },
  DesktopDone: { Req: Req(Empty, z.object({ state: z.string() })), Res: z.string() },
  DesktopExchange: { Req: Req(Empty, Empty, z.object({ state: z.string(), token: z.string(), verifier: z.string() })), Res: SessionDto },
  /** The admin panel's Google sign-in: same PKCE hand-off, token returned to ADMIN_ORIGIN/auth/callback. */
  WebStart: { Req: Req(Empty, z.object({ provider: z.enum(['google']), state: z.string().regex(/^[a-fA-F0-9]{16,64}$/), challenge: z.string().regex(/^[A-Za-z0-9_-]+$/) })), Res: z.string() },
  WebDone: { Req: Req(Empty, z.object({ state: z.string() })), Res: z.string() },
  WebExchange: { Req: Req(Empty, Empty, z.object({ state: z.string(), token: z.string(), verifier: z.string() })), Res: SessionDto },
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
const AdminUser = z.object({ id: z.string().min(1), email: z.email(), name: z.string().nullable() });
const AdminPublisher = z.object({ publisher: PublisherDto, user: AdminUser.nullable(), claimedAt: IsoDate, verifiedAt: IsoDate.nullable(), tombstonedAt: IsoDate.nullable(), extensionCount: z.number().int().nonnegative() });
const AdminExtension = z.object({ repoId: Uuid, owner: PublisherDto, slug: Slug, name: z.string(), tagline: z.string(), visibility: Visibility, moderation: z.enum(['none', 'hidden', 'removed']), tombstoned: z.boolean(), latestVersion: Version.nullable(), installCount: z.number().int().nonnegative(), updatedAt: IsoDate });
const AdminAccount = AdminUser.extend({
  emailVerified: z.boolean(), createdAt: IsoDate, updatedAt: IsoDate,
  publisher: PublisherDto.nullable(), adminGrantedAt: IsoDate.nullable(),
  extensionCount: z.number().int().nonnegative(), installCount: z.number().int().nonnegative(),
  activeSessions: z.number().int().nonnegative(), lastSessionAt: IsoDate.nullable(),
});
const AdminGrant = z.object({ user: AdminUser, grantedAt: IsoDate, grantedBy: AdminUser.nullable() });
const LogTarget = z.object({ kind: z.enum(['repo', 'publisher', 'user']), id: z.string(), label: z.string().nullable() });
const LogEntry = z.object({ id: Uuid, action: z.string(), reason: z.string(), createdAt: IsoDate, actor: z.object({ id: z.string(), label: z.string().nullable() }), target: LogTarget.nullable() });
const PublisherId = z.object({ publisherId: Uuid });
const search = z.object({ q: z.string().max(200).optional() });
export type AdminAccount = z.infer<typeof AdminAccount>;
export type AdminUser = z.infer<typeof AdminUser>;
export type AdminPublisher = z.infer<typeof AdminPublisher>;
export type AdminExtension = z.infer<typeof AdminExtension>;
export type AdminGrant = z.infer<typeof AdminGrant>;
export type AdminLogEntry = z.infer<typeof LogEntry>;

/** Every route needs a session whose user has an `admins` row: 401 signed out, 403 otherwise. */
export const Admin = {
  /** The signed-in admin; 403 tells a signed-in user they are not one. */
  Users: {
    Req: Req(Empty, search.extend({ filter: z.enum(['all', 'admins', 'publishers', 'verified', 'unverified-email']).default('all'), offset: z.coerce.number().int().min(0).max(1000000).default(0), limit: z.coerce.number().int().min(1).max(100).default(50) })),
    Res: z.object({ items: z.array(AdminAccount), total: z.number().int(), offset: z.number().int(), limit: z.number().int(), counts: z.object({ users: z.number().int(), admins: z.number().int(), publishers: z.number().int(), verified: z.number().int() }) }),
  },
  User: {
    Req: Req(z.object({ userId: z.string().min(1).max(200) })),
    Res: AdminAccount.extend({
      providers: z.array(z.string()), publisherDetails: AdminPublisher.nullable(), extensions: z.array(AdminExtension),
      sessions: z.array(z.object({ createdAt: IsoDate, updatedAt: IsoDate, expiresAt: IsoDate, userAgent: z.string().nullable() })),
      library: z.array(z.object({ repoId: Uuid, name: z.string(), handle: z.string(), slug: z.string(), version: z.string(), installedAt: IsoDate })),
    }),
  },
  Session: { Req: Req(), Res: z.object({ user: AdminUser }) },
  /** Seeds a reserved handle (e.g. `powermove`) for an existing user; bypasses the reserved list. */
  SeedPublisher: { Req: Req(Empty, Empty, z.object({ handle: Handle, userId: z.string().min(1) })), Res: z.object({ publisher: PublisherDto }) },
  /** Handle or account email, substring match; newest claims first. */
  Publishers: { Req: Req(Empty, search), Res: z.object({ items: z.array(AdminPublisher) }) },
  Publisher: { Req: Req(PublisherId), Res: AdminPublisher.extend({ extensions: z.array(AdminExtension) }) },
  /** Sets or clears `verified_at`; the blue check on every PublisherDto. */
  SetVerified: { Req: Req(PublisherId, Empty, z.object({ verified: z.boolean() })), Res: AdminPublisher },
  /** Name, slug or handle, substring match, in every moderation and tombstone state. */
  Extensions: { Req: Req(Empty, search), Res: z.object({ items: z.array(AdminExtension) }) },
  Extension: { Req: Req(z.object({ repoId: Uuid })), Res: AdminExtension },
  Moderate: { Req: Req(z.object({ repoId: Uuid }), Empty, z.object({ action: z.enum(['hide', 'unhide', 'remove']), reason: z.string().max(1000) })), Res: ListingDto },
  Admins: { Req: Req(), Res: z.object({ items: z.array(AdminGrant) }) },
  /** Grants admin to an existing account by email. Idempotent. */
  Grant: { Req: Req(Empty, Empty, z.object({ email: z.email() })), Res: AdminGrant },
  /** Refused for the last admin. */
  Revoke: { Req: Req(z.object({ userId: z.string().min(1) })), Res: NoContent },
  Log: { Req: Req(Empty, z.object({ limit: z.coerce.number().int().min(1).max(200).optional() })), Res: z.object({ items: z.array(LogEntry) }) },
  /** Aero and billing at a glance: usage, money, subscriptions, the accounts that matter. Read-only. */
  AeroOverview: { Req: Req(), Res: z.lazy(() => AeroOverview) },
  /** Lets an account blocked for review (a spend over its hold) use Aero again. Audit-logged. */
  AeroUnblock: { Req: Req(z.object({ userId: z.string().min(1) })), Res: NoContent },
} as const;

const Usage = z.object({
  costUsd: z.number(), credits: z.number(), calls: z.number().int(), estimatedCalls: z.number().int(), estimatedUsd: z.number(),
  inputTokens: z.number().int(), cachedTokens: z.number().int(), cacheWriteTokens: z.number().int(), outputTokens: z.number().int(),
  /** Cache reads over all input tokens (0 with no input). */
  cacheHitRatio: z.number(),
});
const Who = z.object({ id: z.string(), email: z.string().nullable() });
export const AeroOverview = z.object({
  generatedAt: IsoDate,
  today: Usage,
  last30: Usage,
  models: z.array(Usage.extend({ model: z.string() })),
  daily: z.array(z.object({ date: z.string(), costUsd: z.number(), credits: z.number(), calls: z.number().int() })),
  subscriptions: z.object({ active: z.number().int(), mrrCents: z.number().int(), new30: z.number().int(), cancelling: z.number().int(), pastDue: z.number().int() }),
  topAccounts: z.array(z.object({ user: Who, costUsd: z.number(), calls: z.number().int(), creditsLeft: z.number().nullable() })),
  estimated: z.array(z.object({ user: Who, model: z.string().nullable(), costUsd: z.number(), source: z.string().nullable(), createdAt: IsoDate })),
  blocked: z.array(z.object({ user: Who, reason: z.string(), createdAt: IsoDate })),
  ceiling: z.object({ todayUsd: z.number(), ceilingUsd: z.number() }),
  reversals: z.array(z.object({ user: Who, invoiceId: z.string(), reason: z.string(), createdAt: IsoDate })),
});
export type AeroOverview = z.infer<typeof AeroOverview>;
