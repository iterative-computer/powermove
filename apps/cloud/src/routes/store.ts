import { Hono } from 'hono';
import { and, desc, eq, gt, ilike, inArray, isNotNull, lt, or, sql, asc } from 'drizzle-orm';
import { ApiError, Store, Category } from '@powermove/registry/wire';
import { decodeLoose, hashObject } from '@powermove/registry/git';
import { diffTrees } from '@powermove/registry/diff';
import { normalizePath, SnapshotError } from '@powermove/registry/snapshot';
import { REGISTRY_LIMITS } from '@powermove/registry/limits';
import type { Context } from 'hono';
import type { Env } from '../env';
import type { Db } from '../db/client';
import { extensions, featured, publishers, releases, repos } from '../db/schema';
import { lineageFor, toDetail, toListing, toRelease } from '../dto';
import { canBrowse, canReadFiles, canViewDetail, updateOffered, type Viewer } from '../lifecycle';

type Repo = typeof repos.$inferSelect;
type Release = typeof releases.$inferSelect;
type ListingRow = { repo: Repo; extension: typeof extensions.$inferSelect; owner: typeof publishers.$inferSelect; latest: Release | null };
const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const bad = () => new ApiError({ error: 'bad_request' });
const missing = () => new ApiError({ error: 'not_found' });
function decision(value: ReturnType<typeof canReadFiles>) {
  if (value === 'ok') return;
  if (value === 'not_found') throw missing();
  throw new ApiError({ error: 'gone', reason: value === 'gone_removed' ? 'removed' : value === 'gone_tombstoned' ? 'tombstoned' : 'yanked' });
}
async function viewer(c: Context<Env>): Promise<Viewer> {
  const session = c.var.session;
  if (!session) return { publisherId: null, admin: false };
  const [publisher] = await c.var.data.db.select({ id: publishers.id }).from(publishers).where(eq(publishers.userId, session.userId)).limit(1);
  return { publisherId: publisher?.id ?? null, admin: false };
}
function listingQuery(db: Db) {
  return db.select({ repo: repos, extension: extensions, owner: publishers, latest: releases })
    .from(repos).innerJoin(extensions, eq(extensions.repoId, repos.id))
    .innerJoin(publishers, eq(publishers.id, repos.ownerId))
    .leftJoin(releases, eq(releases.id, extensions.latestReleaseId));
}
async function listing(db: Db, row: ListingRow) {
  return toListing(row.repo, row.extension, row.owner, row.latest, await lineageFor(db, row.repo));
}
function browseFilter(v: Viewer) {
  return and(eq(repos.visibility, 'public'), eq(repos.moderation, 'none'), sql`${repos.tombstonedAt} is null`, isNotNull(extensions.latestReleaseId));
}
function encodeCursor(value: string | number, id: string) { return btoa(JSON.stringify([value, id])).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/g, ''); }
function decodeCursor(raw: string, sort: 'new' | 'installs' | 'name'): [string | number, string] {
  try {
    if (!/^[A-Za-z0-9_-]+$/.test(raw)) throw bad();
    const parsed: unknown = JSON.parse(atob(raw.replace(/-/g, '+').replace(/_/g, '/')));
    if (!Array.isArray(parsed) || parsed.length !== 2 || typeof parsed[1] !== 'string' || !uuid.test(parsed[1])) throw bad();
    if (sort === 'installs' ? !Number.isSafeInteger(parsed[0]) || parsed[0] < 0 : typeof parsed[0] !== 'string') throw bad();
    if (sort === 'new' && Number.isNaN(Date.parse(parsed[0] as string))) throw bad();
    return [parsed[0] as string | number, parsed[1]];
  } catch { throw bad(); }
}
async function coordinate(db: Db, handle: string, slug: string, v: Viewer) {
  const [row] = await db.select({ repo: repos, extension: extensions, owner: publishers, latest: releases })
    .from(repos).innerJoin(publishers, eq(publishers.id, repos.ownerId))
    .innerJoin(extensions, eq(extensions.repoId, repos.id))
    .leftJoin(releases, eq(releases.id, extensions.latestReleaseId))
    .where(and(eq(publishers.handle, handle), eq(repos.slug, slug))).limit(1);
  if (!row) throw missing();
  decision(canViewDetail(row.repo, v));
  return row;
}
async function releaseById(db: Db, id: string, v: Viewer) {
  const [row] = await db.select({ repo: repos, release: releases, owner: publishers })
    .from(releases).innerJoin(repos, eq(repos.id, releases.repoId))
    .innerJoin(publishers, eq(publishers.id, repos.ownerId)).where(eq(releases.id, id)).limit(1);
  if (!row) throw missing();
  decision(canReadFiles(row.repo, row.release, v));
  return row;
}
async function coordinateRelease(db: Db, handle: string, slug: string, version: string, v: Viewer) {
  const row = await coordinate(db, handle, slug, v);
  const [release] = await db.select().from(releases).where(and(eq(releases.repoId, row.repo.id), eq(releases.version, version))).limit(1);
  if (!release) throw missing();
  decision(canReadFiles(row.repo, release, v));
  return { ...row, release };
}
function tree(release: Release) { return { treeSha: release.treeSha, files: release.files }; }
async function tar(c: Context<Env>, release: Release, slug: string) {
  const object = await c.env.TARS.get(release.tarKey);
  if (!object) throw new ApiError({ error: 'internal' });
  return new Response(object.body, { headers: {
    'Content-Type': 'application/gzip', 'X-Tar-Sha256': release.tarSha256, 'X-Tree-Sha': release.treeSha,
    'Cache-Control': 'public, max-age=31536000, immutable',
    'Content-Disposition': `attachment; filename="${slug}-${release.version}.tar.gz"`
  } });
}
async function file(c: Context<Env>, release: Release) {
  let path: string;
  try { const raw = new URL(c.req.url).pathname.split('/file/').slice(1).join('/file/'); path = normalizePath(decodeURIComponent(raw)); }
  catch (e) { if (e instanceof SnapshotError || e instanceof URIError) throw bad(); throw e; }
  const entry = release.files.find(f => f.path === path);
  if (!entry) throw missing();
  if (entry.size > REGISTRY_LIMITS.fileBytes) throw new ApiError({ error: 'too_large', limit: 'file_too_large' });
  const object = await c.env.OBJECTS.get(`objects/${entry.sha}`);
  if (!object) throw new ApiError({ error: 'internal' });
  const decoded = await decodeLoose(new Uint8Array(await object.arrayBuffer()), REGISTRY_LIMITS.fileBytes);
  if (decoded.type !== 'blob' || decoded.body.length !== entry.size || await hashObject('blob', decoded.body) !== entry.sha) throw new ApiError({ error: 'internal' });
  return new Response(new Uint8Array(decoded.body).buffer, { headers: {
    'Content-Type': 'text/plain; charset=utf-8', 'X-Content-Type-Options': 'nosniff',
    'Content-Security-Policy': 'sandbox', 'Cache-Control': 'public, max-age=31536000, immutable'
  } });
}

export const storeRoutes = new Hono<Env>()
  .get('/browse', async c => {
    const db = c.var.data.db, v = await viewer(c);
    const base = await listingQuery(db).where(browseFilter(v));
    const available = new Map(base.filter(row => canBrowse(row.repo, v) && row.latest).map(row => [row.repo.id, row]));
    const pinned = await db.select().from(featured).where(inArray(featured.section, ['featured', 'picks'])).orderBy(asc(featured.position));
    const section = async (id: 'featured' | 'picks', title: string) => ({ id, title, items: await Promise.all(pinned.filter(p => p.section === id).flatMap(p => { const row = available.get(p.repoId); return row ? [row] : []; }).map(row => listing(db, row))) });
    const newest = [...available.values()].sort((a, b) => b.repo.updatedAt.getTime() - a.repo.updatedAt.getTime() || b.repo.id.localeCompare(a.repo.id));
    const sections: { id: 'featured' | 'picks' | 'new' | typeof Category.options[number]; title: string; items: Awaited<ReturnType<typeof listing>>[] }[] = [await section('featured', 'Featured'), await section('picks', 'Picks'), { id: 'new', title: 'New', items: await Promise.all(newest.slice(0, 12).map(row => listing(db, row))) }];
    for (const category of Category.options) {
      const rows = newest.filter(row => row.extension.category === category).slice(0, 12);
      if (rows.length) sections.push({ id: category, title: category[0]!.toUpperCase() + category.slice(1), items: await Promise.all(rows.map(row => listing(db, row))) });
    }
    return c.json({ sections });
  })
  .get('/extensions', async c => {
    const query = Store.Extensions.Req.shape.query.parse(c.req.query());
    const db = c.var.data.db, v = await viewer(c), sort = query.sort ?? 'new', limit = query.limit ?? 24;
    const cursor = query.cursor ? decodeCursor(query.cursor, sort) : null;
    // JS Dates have millisecond precision; sort and seek on that same precision.
    const newTime = sql<Date>`date_trunc('milliseconds', ${repos.updatedAt})`;
    const column = sort === 'new' ? newTime : sort === 'installs' ? extensions.installCount : extensions.name;
    const value = cursor ? sort === 'new' ? new Date(cursor[0] as string) : cursor[0] : null;
    const keyset = cursor ? sort === 'name'
      ? or(gt(extensions.name, value as string), and(eq(extensions.name, value as string), gt(repos.id, cursor[1])))
      : sort === 'installs'
        ? or(lt(extensions.installCount, value as number), and(eq(extensions.installCount, value as number), lt(repos.id, cursor[1])))
        : or(lt(newTime, value as Date), and(eq(newTime, value as Date), lt(repos.id, cursor[1]))) : undefined;
    const q = query.q?.trim();
    const escaped = q?.replace(/[\\%_]/g, '\\$&');
    const search = q ? or(sql`${extensions.search} @@ websearch_to_tsquery('simple', ${q})`, ilike(extensions.name, `${escaped}%`), ilike(extensions.slug, `${escaped}%`), ilike(extensions.handle, `${escaped}%`)) : undefined;
    const rows = await listingQuery(db).where(and(browseFilter(v), query.category ? eq(extensions.category, query.category) : undefined, search, keyset))
      .orderBy(sort === 'name' ? asc(extensions.name) : desc(column), sort === 'name' ? asc(repos.id) : desc(repos.id)).limit(limit + 1);
    const visible = rows.filter(row => canBrowse(row.repo, v) && row.latest);
    const page = visible.slice(0, limit);
    const last = page.at(-1);
    const sortValue = last && (sort === 'new' ? last.repo.updatedAt.toISOString() : sort === 'installs' ? last.extension.installCount : last.extension.name);
    return c.json({ items: await Promise.all(page.map(row => listing(db, row))), nextCursor: visible.length > limit && last ? encodeCursor(sortValue!, last.repo.id) : null });
  })
  .get('/x/:handle/:slug', async c => {
    const { handle, slug } = Store.Detail.Req.shape.params.parse(c.req.param());
    const db = c.var.data.db, row = await coordinate(db, handle, slug, await viewer(c));
    const releaseRows = await db.select().from(releases).where(eq(releases.repoId, row.repo.id)).orderBy(desc(releases.publishedAt), desc(releases.id));
    return c.json(toDetail(row.repo, row.extension, row.owner, row.latest, await lineageFor(db, row.repo), releaseRows));
  })
  .get('/x/:handle/:slug/r/:version', async c => { const p = Store.Release.Req.shape.params.parse(c.req.param()); return c.json(toRelease((await coordinateRelease(c.var.data.db, p.handle, p.slug, p.version, await viewer(c))).release)); })
  .get('/x/:handle/:slug/r/:version/tree', async c => { const p = Store.Tree.Req.shape.params.parse(c.req.param()); return c.json(tree((await coordinateRelease(c.var.data.db, p.handle, p.slug, p.version, await viewer(c))).release)); })
  .get('/x/:handle/:slug/r/:version/tar', async c => { const p = Store.Tar.Req.shape.params.parse(c.req.param()); return tar(c, (await coordinateRelease(c.var.data.db, p.handle, p.slug, p.version, await viewer(c))).release, p.slug); })
  .get('/x/:handle/:slug/r/:version/file/*', async c => { const raw = new URL(c.req.url).pathname.split('/file/').slice(1).join('/file/'); const p = Store.File.Req.shape.params.parse({ ...c.req.param(), path: raw }); return file(c, (await coordinateRelease(c.var.data.db, p.handle, p.slug, p.version, await viewer(c))).release); })
  .get('/releases/:releaseId', async c => { const { releaseId } = Store.ReleaseById.Req.shape.params.parse(c.req.param()); const row = await releaseById(c.var.data.db, releaseId, await viewer(c)); return c.json({ ...toRelease(row.release), handle: row.owner.handle, slug: row.repo.slug }); })
  .get('/releases/:releaseId/tree', async c => { const { releaseId } = Store.TreeById.Req.shape.params.parse(c.req.param()); return c.json(tree((await releaseById(c.var.data.db, releaseId, await viewer(c))).release)); })
  .get('/releases/:releaseId/tar', async c => { const { releaseId } = Store.TarById.Req.shape.params.parse(c.req.param()); const row = await releaseById(c.var.data.db, releaseId, await viewer(c)); return tar(c, row.release, row.repo.slug); })
  .get('/compare', async c => { const { base, head } = Store.Compare.Req.shape.query.parse(c.req.query()); const v = await viewer(c), db = c.var.data.db; const [a, b] = await Promise.all([releaseById(db, base, v), releaseById(db, head, v)]); return c.json({ baseReleaseId: base, headReleaseId: head, ...diffTrees(a.release.files, b.release.files) }); })
  .post('/versions', async c => {
    const { items } = Store.Versions.Req.shape.body.parse(await c.req.json().catch(() => null));
    if (!items.length) return c.json({ items: [] });
    const db = c.var.data.db, v = await viewer(c);
    const rows = await listingQuery(db).where(inArray(repos.id, [...new Set(items.map(item => item.repoId))]));
    const byRepo = new Map(rows.map(row => [row.repo.id, row]));
    const current = await db.select({ id: releases.id, repoId: releases.repoId, yankedAt: releases.yankedAt }).from(releases).where(inArray(releases.id, [...new Set(items.map(item => item.releaseId))]));
    const byRelease = new Map(current.map(row => [row.id, row]));
    return c.json({ items: items.map(item => {
      const row = byRepo.get(item.repoId), existing = byRelease.get(item.releaseId);
      if (!row) return { repoId: item.repoId, state: 'removed' as const, ownerPublisherId: null, current: { yanked: false }, latest: null, handle: null, slug: null };
      const detail = canViewDetail(row.repo, v);
      const state = detail === 'gone_removed' ? 'removed' as const : detail === 'gone_tombstoned' ? 'tombstoned' as const : row.repo.moderation === 'hidden' ? 'hidden' as const : 'ok' as const;
      const latest = row.latest && updateOffered(row.repo, row.latest) ? { releaseId: row.latest.id, version: row.latest.version, treeSha: row.latest.treeSha, tarSha256: row.latest.tarSha256, apiVersion: row.latest.apiVersion } : null;
      return { repoId: item.repoId, state, ownerPublisherId: row.owner.id, current: { yanked: existing?.repoId === item.repoId && existing.yankedAt !== null }, latest, handle: row.owner.handle, slug: row.repo.slug };
    }) });
  });
