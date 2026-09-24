import { Hono } from 'hono';
import { and, eq, sql } from 'drizzle-orm';
import { ApiError, Publish } from '@powermove/registry/wire';
import { decodeLoose, hashObject, parseCommit, parseTree, sha256Hex } from '@powermove/registry/git';
import { snapshot, SnapshotError, type SnapshotInput } from '@powermove/registry/snapshot';
import { REGISTRY_LIMITS } from '@powermove/registry/limits';
import { EXTENSION_API_VERSION, parseForkedFrom, parseManifest } from '@powermove/registry/manifest';
import { scanCapabilities, scanFiles } from '@powermove/registry/scan';
import { writeTarGz } from '@powermove/registry/tar';
import type { Env } from '../env';
import { extensions, objects, publishers, refs, releaseObjects, releases, repos } from '../db/schema';
import { enforce } from '../abuse';
import { presentShas } from '../objects/presence';
import { storeIcon } from '../objects/icon';
import { canReadFiles } from '../lifecycle';
import { lineageFor, toListing, toRelease } from '../dto';
import { requirePublisher, requireSession } from './session';
import { isReservedExtensionId } from '../handles';
const invalid = (detail?: string) => new ApiError({ error: 'tree_invalid', detail });
const missing = (shas: string[]) => new ApiError({ error: 'object_missing', shas });
const unique = (e: unknown) =>
  (e as {
      code?: string;
      cause?: {
        code?: string;
      };
    }).code === '23505' || (e as {
        cause?: {
          code?: string;
        };
      }).cause?.code === '23505';
const dec = new TextDecoder('utf-8', { fatal: true });
async function getObject(bucket: R2Bucket, sha: string, expected: 'commit' | 'tree' | 'blob') {
  const obj = await bucket.get(`objects/${sha}`);
  if (!obj) {
    throw expected === 'commit' ? new ApiError({ error: 'commit_missing' }) : missing([sha]);
  }
  try {
    const decoded = await decodeLoose(new Uint8Array(await obj.arrayBuffer()), REGISTRY_LIMITS.fileBytes + 65536);
    if (decoded.type !== expected) {
      throw expected === 'commit' ? new ApiError({ error: 'commit_missing' }) : invalid();
    }
    if (await hashObject(decoded.type, decoded.body) !== sha) {
      throw invalid();
    }
    return decoded.body;
  } catch (e) {
    if (e instanceof ApiError) {
      throw e;
    }
    throw expected === 'commit' ? new ApiError({ error: 'commit_missing' }) : invalid();
  }
}
async function walk(bucket: R2Bucket, treeSha: string, authorize: (shas: string[]) => Promise<void>): Promise<{
  files: SnapshotInput[];
  shas: string[];
}> {
  const files: SnapshotInput[] = [], shas = new Set<string>();
  const active = new Set<string>();
  let entries = 0;
  const blobEntries: {
    path: string;
    sha: string;
  }[] = [];
  async function visit(sha: string, prefix: string, depth: number): Promise<void> {
    if (depth > REGISTRY_LIMITS.files || active.has(sha)) {
      throw invalid();
    }
    await authorize([sha]);
    active.add(sha);
    shas.add(sha);
    let tree;
    try {
      tree = parseTree(await getObject(bucket, sha, 'tree'));
    } catch (e) {
      if (e instanceof ApiError) {
        throw e;
      }
      throw invalid();
    }
    for (const entry of tree) {
      if (++entries > REGISTRY_LIMITS.files * 2 + 1) {
        throw new ApiError({ error: 'limit_exceeded', code: 'too_many_files' });
      }
      const path = prefix + entry.name;
      if (entry.mode === '40000') {
        await visit(entry.sha, `${path}/`, depth + 1);
      } else {
        shas.add(entry.sha);
        blobEntries.push({ path, sha: entry.sha });
        if (blobEntries.length > REGISTRY_LIMITS.files) {
          throw new ApiError({ error: 'limit_exceeded', code: 'too_many_files' });
        }
      }
    }
    active.delete(sha);
  }
  await visit(treeSha, '', 0);
  await authorize([...new Set(blobEntries.map((entry) => entry.sha))]);
  const blobs = new Map<string, Uint8Array>();
  let totalBytes = 0;
  for (const entry of blobEntries) {
    let bytes = blobs.get(entry.sha);
    if (!bytes) {
      bytes = await getObject(bucket, entry.sha, 'blob');
      blobs.set(entry.sha, bytes);
    }
    totalBytes += bytes.length;
    if (totalBytes > REGISTRY_LIMITS.treeBytes) {
      throw new ApiError({ error: 'limit_exceeded', code: 'tree_too_large' });
    }
    files.push({ path: entry.path, bytes });
  }
  return { files, shas: [...shas] };
}
export interface PublishDeps {
  beforeCommit?: () => Promise<void>;
}
export function publishRoutes(deps: PublishDeps = {}) {
  return new Hono<Env>()
    .put('/:handle/:slug/releases', async (c) => {
      const session = requireSession(c), publisher = await requirePublisher(c);
      await enforce(c, 'publish_user_hour', session.userId);
      await enforce(c, 'publish_user_day', session.userId);
      const { handle, slug } = Publish.PutRelease.Req.shape.params.parse(c.req.param());
      if (handle !== publisher.handle) {
        throw new ApiError({ error: 'not_owner' });
      }
      const body = Publish.PutRelease.Req.shape.body.parse(await c.req.json().catch(() => null));
      const db = c.var.data.db;
      const [existing] = await db.select().from(repos).where(and(eq(repos.ownerId, publisher.id), eq(repos.slug, slug)))
        .limit(1);
      if (existing?.tombstonedAt) {
        throw new ApiError({ error: 'gone', reason: 'tombstoned' });
      }
      if (existing?.moderation === 'removed') {
        throw new ApiError({ error: 'gone', reason: 'removed' });
      }
      if (existing && body.originReleaseId) {
        throw new ApiError({ error: 'bad_request', detail: 'originReleaseId only applies to a new repo' });
      }
      if (!existing && body.basedOnReleaseId) {
        throw new ApiError({ error: 'bad_request', detail: 'basedOnReleaseId requires a fork' });
      }
      let origin: typeof releases.$inferSelect | undefined, originRepo: typeof repos.$inferSelect | undefined;
      if (body.originReleaseId) {
        [origin] = await db.select().from(releases).where(eq(releases.id, body.originReleaseId)).limit(1);
        if (!origin) {
          throw new ApiError({ error: 'not_found' });
        }
        [originRepo] = await db.select().from(repos).where(eq(repos.id, origin.repoId)).limit(1);
        if (!originRepo || canReadFiles(originRepo, origin, { publisherId: publisher.id, admin: false }) !== 'ok') {
          throw new ApiError({ error: 'not_found' });
        }
        if (originRepo.ownerId === publisher.id) {
          throw new ApiError({ error: 'self_origin' });
        }
      }
      if (body.basedOnReleaseId) {
        if (!existing?.forkedFromRepoId) {
          throw new ApiError({ error: 'bad_request', detail: 'not a fork' });
        }
        const [based] = await db.select().from(releases).where(eq(releases.id, body.basedOnReleaseId)).limit(1);
        const [basedRepo] = based ? await db.select().from(repos).where(eq(repos.id, based.repoId)).limit(1) : [];
        if (
          !based || !basedRepo || based.repoId !== existing.forkedFromRepoId ||
          canReadFiles(basedRepo, based, { publisherId: publisher.id, admin: false }) !== 'ok'
        ) {
          throw new ApiError({ error: 'bad_request', detail: 'basedOnReleaseId is not readable' });
        }
      }
      // All object IO and validation precedes the database transaction.
      const presenceContext = {
        repoId: existing?.id,
        originReleaseId: body.originReleaseId,
        basedOnReleaseId: body.basedOnReleaseId,
      };
      const authorize = async (shas: string[], commit = false) => {
        const present = await presentShas(db, session.userId, shas, presenceContext, c.env.OBJECTS);
        const absent = shas.filter((sha) => !present.has(sha));
        if (absent.length) {
          throw commit ? new ApiError({ error: 'commit_missing' }) : missing(absent);
        }
      };
      await authorize([body.commitSha], true);
      let commit;
      try {
        commit = parseCommit(await getObject(c.env.OBJECTS, body.commitSha, 'commit'));
      } catch (e) {
        if (e instanceof ApiError) {
          throw e;
        }
        throw new ApiError({ error: 'commit_missing' });
      }
      const identity = { name: handle, email: `${handle}@${c.env.HANDLE_MAIL_DOMAIN}` };
      if ([commit.author, commit.committer].some((x) => x.name !== identity.name || x.email !== identity.email)) {
        throw new ApiError({ error: 'author_mismatch' });
      }
      const walked = await walk(c.env.OBJECTS, commit.tree, authorize);
      let snap;
      try {
        snap = await snapshot(walked.files);
      } catch (e) {
        if (e instanceof SnapshotError) {
          if (['too_many_files', 'tree_too_large', 'file_too_large', 'path_too_long'].includes(e.code)) {
            throw new ApiError({
              error: 'limit_exceeded',
              code: e.code as 'too_many_files' | 'tree_too_large' | 'file_too_large' | 'path_too_long',
            });
          }
          throw invalid(e.message);
        }
        throw e;
      }
      if (snap.treeSha !== commit.tree) {
        throw invalid('snapshot tree differs from commit');
      }
      const manifestFile = walked.files.find((x) => x.path === 'manifest.json');
      if (!manifestFile) {
        throw new ApiError({ error: 'manifest_invalid', detail: 'manifest.json is missing' });
      }
      let raw: unknown;
      try {
        raw = JSON.parse(dec.decode(manifestFile.bytes));
      } catch {
        throw new ApiError({ error: 'manifest_invalid', detail: 'manifest.json is invalid JSON or UTF-8' });
      }
      const parsed = parseManifest(raw);
      if (!parsed.ok) {
        throw new ApiError({ error: 'manifest_invalid', detail: parsed.error });
      }
      const manifest = parsed.manifest;
      if (manifest.apiVersion > EXTENSION_API_VERSION) {
        throw new ApiError({ error: 'manifest_invalid', detail: `apiVersion ${manifest.apiVersion} is newer than this app (${EXTENSION_API_VERSION})` });
      }
      if (!origin && manifest.forkedFrom && parseForkedFrom(manifest.forkedFrom)?.kind === 'store' && !existing) {
        throw new ApiError({ error: 'manifest_invalid', detail: 'store fork requires originReleaseId' });
      }
      if (manifest.id !== slug) {
        throw new ApiError({ error: 'manifest_invalid', detail: 'manifest id must equal slug' });
      }
      // The reserved `powermove` publisher seeds the shipped built-ins. Other
      // publishers cannot claim their ids or any host event word.
      if (isReservedExtensionId(manifest.id) && publisher.handle !== 'powermove') {
        throw new ApiError({ error: 'manifest_invalid', detail: 'reserved id' });
      }
      if (manifest.version !== body.version) {
        throw new ApiError({ error: 'version_invalid' });
      }
      if (origin && originRepo) {
        if (snap.treeSha === origin.treeSha) {
          throw new ApiError({ error: 'same_as_origin' });
        }
        const [originOwner] = await db.select().from(publishers).where(eq(publishers.id, originRepo.ownerId)).limit(1);
        const forked = manifest.forkedFrom && parseForkedFrom(manifest.forkedFrom);
        if (
          !originOwner || !forked || forked.kind !== 'store' || forked.handle !== originOwner.handle ||
          forked.id !== originRepo.slug || forked.version !== origin.version
        ) {
          throw new ApiError({ error: 'manifest_invalid', detail: 'forkedFrom must name the origin release' });
        }
      }
      if (existing) {
        const forked = manifest.forkedFrom && parseForkedFrom(manifest.forkedFrom);
        if (existing.forkedFromReleaseId) {
          const [lineage] = await db.select({ release: releases, repo: repos, owner: publishers }).from(releases)
            .innerJoin(repos, eq(repos.id, releases.repoId)).innerJoin(publishers, eq(publishers.id, repos.ownerId))
            .where(eq(releases.id, existing.forkedFromReleaseId)).limit(1);
          if (
            !lineage || !forked || forked.kind !== 'store' || forked.handle !== lineage.owner.handle ||
            forked.id !== lineage.repo.slug || forked.version !== lineage.release.version
          ) {
            throw new ApiError({ error: 'manifest_invalid', detail: 'forkedFrom must match repo lineage' });
          }
        } else if (forked && forked.kind === 'store') {
          throw new ApiError({ error: 'manifest_invalid', detail: 'repo has no store lineage' });
        }
      }
      const wantedParent = existing
        ? (await db.select().from(refs).where(and(eq(refs.repoId, existing.id), eq(refs.name, 'main'))).limit(1))[0]
          ?.sha
        : origin?.commitSha;
      if (existing && !wantedParent) {
        throw invalid('main ref missing');
      }
      if (commit.parents.length !== (wantedParent ? 1 : 0) || (wantedParent && commit.parents[0] !== wantedParent)) {
        throw new ApiError({ error: 'head_moved', head: wantedParent ?? '0'.repeat(40) });
      }
      const textFiles = walked.files.flatMap((x) => {
        try {
          return [{ path: x.path, text: dec.decode(x.bytes) }];
        } catch {
          return [];
        }
      });
      const undeclared = scanCapabilities(textFiles).filter((finding) => !manifest.permissions?.includes(finding.capability));
      if (undeclared.length) throw new ApiError({ error: 'permission_undeclared', findings: undeclared });
      const scan = scanFiles(textFiles);
      const findings = [...scan.blocked, ...scan.waived];
      for (const waiver of body.waivers) {
        if (!findings.some((f) => !f.hard && f.path === waiver.path && f.line === waiver.line)) {
          throw new ApiError({ error: 'bad_request', detail: 'waiver does not match a non-hard finding' });
        }
      }
      const waived = findings.flatMap((f) => {
        const w = body.waivers.find((w) => w.path === f.path && w.line === f.line);
        return !f.hard && w ? [{ path: f.path, line: f.line, kind: f.kind, reason: w.reason }] : [];
      });
      const blocked = findings.filter((f) =>
        !waived.some((w) => w.path === f.path && w.line === f.line && w.kind === f.kind)
      );
      if (blocked.length) {
        throw new ApiError({
          error: 'scan_blocked',
          findings: blocked.map(({ path, line, kind }) => ({ path, line, kind })),
        });
      }
      const shas = [...new Set([body.commitSha, ...walked.shas])];
      await authorize(shas);
      const iconKey = await storeIcon(c.env.ICONS, body.iconPng);
      const releaseId = crypto.randomUUID(), tarKey = `tars/${releaseId}.tar.gz`;
      const tar = await writeTarGz(walked.files), tarSha256 = await sha256Hex(tar);
      const put = await c.env.TARS.put(tarKey, tar, { onlyIf: { etagDoesNotMatch: '*' } });
      if (!put) {
        throw new ApiError({ error: 'internal', detail: 'tar key collision' });
      }
      const result = await (async () => {
        try {
          const head = await c.env.TARS.head(tarKey), check = await c.env.TARS.get(tarKey);
          if (!head || !check || await sha256Hex(new Uint8Array(await check.arrayBuffer())) !== tarSha256) {
            throw new ApiError({ error: 'internal', detail: 'tar verification failed' });
          }
          await deps.beforeCommit?.();
          return await c.var.data.tx(async (tx) => {
            let repo = existing;
            if (repo) {
              await tx.execute(sql`select id from repos where id = ${repo.id}::uuid for update`);
              const [current] = await tx.select().from(repos).where(eq(repos.id, repo.id)).limit(1);
              if (!current || current.tombstonedAt) {
                throw new ApiError({ error: 'head_moved', head: wantedParent ?? '0'.repeat(40) });
              }
              repo = current;
            } else {
              try {
                [repo] = await tx.insert(repos).values({
                  ownerId: publisher.id,
                  slug,
                  visibility: body.visibility ?? 'public',
                  forkedFromRepoId: originRepo?.id,
                  forkedFromReleaseId: origin?.id,
                }).returning();
              } catch (e) {
                if (unique(e)) {
                  throw new ApiError({ error: 'head_moved', head: '0'.repeat(40) });
                }
                throw e;
              }
            }
            if (!repo) {
              throw new Error('repo insert failed');
            }
            const locked = await tx.update(objects).set({ gcState: 'live', lastSeenAt: new Date() }).where(
              and(
                sql`${objects.sha} in (${sql.join(shas.map((sha) => sql`${sha}`), sql`, `)})`,
                eq(objects.gcState, 'live'),
              ),
            ).returning({ sha: objects.sha });
            const lockedShas = new Set(locked.map((x) => x.sha));
            if (lockedShas.size !== shas.length) {
              throw missing(shas.filter((x) => !lockedShas.has(x)));
            }
            if (existing) {
              const changed = await tx.update(refs).set({ sha: body.commitSha }).where(
                and(eq(refs.repoId, repo.id), eq(refs.name, 'main'), eq(refs.sha, wantedParent!)),
              ).returning();
              if (!changed.length) {
                const [ref] = await tx.select().from(refs).where(and(eq(refs.repoId, repo.id), eq(refs.name, 'main')));
                throw new ApiError({ error: 'head_moved', head: ref?.sha ?? '0'.repeat(40) });
              }
            } else {
              await tx.insert(refs).values({ repoId: repo.id, name: 'main', sha: body.commitSha });
            }
            let release;
            try {
              [release] = await tx.insert(releases).values({
                id: releaseId,
                repoId: repo.id,
                version: body.version,
                commitSha: body.commitSha,
                treeSha: snap.treeSha,
                tarKey,
                tarSha256,
                manifest,
                permissions: manifest.permissions ?? [],
                files: snap.files,
                notes: body.notes ?? null,
                basedOnReleaseId: body.basedOnReleaseId ?? null,
                apiVersion: manifest.apiVersion,
                fileCount: snap.files.length,
                sizeBytes: snap.totalBytes,
                scanWaivers: waived,
              }).returning();
            } catch (e) {
              if (unique(e)) {
                throw new ApiError({ error: 'version_exists' });
              }
              throw e;
            }
            if (!release) {
              throw new Error('release insert failed');
            }
            await tx.insert(releaseObjects).values(shas.map((sha) => ({ releaseId, sha })));
            const listing = body.listing;
            await tx.insert(extensions).values({
              repoId: repo.id,
              handle,
              slug,
              name: listing.name,
              tagline: listing.tagline,
              about: listing.about ?? null,
              category: listing.category,
              licence: listing.licence ?? 'MIT',
              iconKey: iconKey ?? null,
              latestReleaseId: releaseId,
              permissions: manifest.permissions ?? [],
            }).onConflictDoUpdate({
              target: extensions.repoId,
              set: {
                name: listing.name,
                tagline: listing.tagline,
                about: listing.about ?? null,
                category: listing.category,
                licence: listing.licence ?? 'MIT',
                ...(iconKey ? { iconKey } : {}),
                latestReleaseId: releaseId,
                permissions: manifest.permissions ?? [],
              },
            });
            if (originRepo && !existing) {
              await tx.update(extensions).set({ forkCount: sql`${extensions.forkCount}+1` }).where(
                eq(extensions.repoId, originRepo.id),
              );
            }
            const [updatedRepo] = await tx.update(repos).set({ updatedAt: new Date() }).where(eq(repos.id, repo.id))
              .returning();
            const [ext] = await tx.select().from(extensions).where(eq(extensions.repoId, repo.id));
            return { repo: updatedRepo!, extension: ext!, release };
          });
        } catch (e) {
          try {
            await c.env.TARS.delete(tarKey);
          } catch (cleanup) {
            console.error('tar cleanup failed', cleanup);
          }
          if (e instanceof ApiError && e.body.error === 'head_moved' && e.body.head === '0'.repeat(40) && !existing) {
            const [winner] = await db.select({ sha: refs.sha }).from(repos).innerJoin(
              refs,
              and(eq(refs.repoId, repos.id), eq(refs.name, 'main')),
            ).where(and(eq(repos.ownerId, publisher.id), eq(repos.slug, slug))).limit(1);
            if (winner) {
              throw new ApiError({ error: 'head_moved', head: winner.sha });
            }
          }
          throw e;
        }
      })();
      return c.json({
        repo: toListing(result.repo, result.extension, publisher, result.release, await lineageFor(db, result.repo)),
        release: toRelease(result.release),
      }, 201);
    });
}
