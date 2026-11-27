import { Hono, type Context } from 'hono';
import { and, desc, eq, ilike, isNotNull, or, sql, type SQL } from 'drizzle-orm';
import { alias } from 'drizzle-orm/pg-core';
import { Admin, ApiError, type AdminAccount, type AdminExtension, type AdminGrant, type AdminLogEntry, type AdminPublisher } from '@powermove/registry/wire';
import type { Env } from '../env';
import { admins, aeroBlocks, extensions, installs, moderationLog, publishers, releases, repos } from '../db/schema';
import { aeroOverview } from '../billing/analytics';
import { user, session, account as authAccount } from '../db/auth-schema';
import { listing } from './repo-management';
import { requireAdmin, requireSession } from './session';
import { enforce, rateLimited } from '../abuse';

type Publisher = typeof publishers.$inferSelect;
type Account = { id: string; email: string; name: string | null } | null;

/* Substring match; LIKE's own wildcards in the query are literal. */
function contains(q: string | undefined): string | null {
  const text = q?.trim();
  return text ? `%${text.replace(/[\\%_]/g, (m) => `\\${m}`)}%` : null;
}
function uniqueViolation(e: unknown): boolean {
  return ((e as { code?: string }).code ?? (e as { cause?: { code?: string } }).cause?.code) === '23505';
}
function toPublisher(p: Publisher, account: Account, extensionCount: number): AdminPublisher {
  return {
    publisher: { id: p.id, handle: p.handle, tombstoned: p.tombstonedAt !== null, verified: p.verifiedAt !== null },
    user: account, claimedAt: p.claimedAt.toISOString(), verifiedAt: p.verifiedAt?.toISOString() ?? null,
    tombstonedAt: p.tombstonedAt?.toISOString() ?? null, extensionCount,
  };
}
const extensionCount = sql<number>`(select count(*)::int from ${repos} where ${repos.ownerId} = ${publishers.id})`;
const account = { id: user.id, email: user.email, name: user.name };

async function publisherRows(c: Context<Env>, where: SQL | undefined, limit?: number): Promise<AdminPublisher[]> {
  const query = c.var.data.db.select({ publisher: publishers, user: account, extensionCount }).from(publishers)
    .leftJoin(user, eq(user.id, publishers.userId)).where(where).orderBy(desc(publishers.claimedAt), desc(publishers.id));
  const rows = limit ? await query.limit(limit) : await query;
  return rows.map((row) => toPublisher(row.publisher, row.user, row.extensionCount));
}
async function extensionRows(c: Context<Env>, where: SQL | undefined, limit?: number): Promise<AdminExtension[]> {
  const query = c.var.data.db.select({ repo: repos, extension: extensions, owner: publishers, latestVersion: releases.version })
    .from(repos).innerJoin(extensions, eq(extensions.repoId, repos.id)).innerJoin(publishers, eq(publishers.id, repos.ownerId))
    .leftJoin(releases, eq(releases.id, extensions.latestReleaseId)).where(where).orderBy(desc(repos.updatedAt), desc(repos.id));
  const rows = limit ? await query.limit(limit) : await query;
  return rows.map(({ repo, extension, owner, latestVersion }) => ({
    repoId: repo.id,
    owner: { id: owner.id, handle: owner.handle, tombstoned: owner.tombstonedAt !== null, verified: owner.verifiedAt !== null },
    slug: repo.slug, name: extension.name, tagline: extension.tagline, visibility: repo.visibility, moderation: repo.moderation,
    tombstoned: repo.tombstonedAt !== null, latestVersion, installCount: extension.installCount, updatedAt: repo.updatedAt.toISOString(),
  }));
}
async function publisherById(c: Context<Env>, id: string): Promise<AdminPublisher> {
  const [row] = await publisherRows(c, eq(publishers.id, id));
  if (!row) throw new ApiError({ error: 'not_found' });
  return row;
}
/* Only explicit account metadata leaves this boundary: never tokens or credentials. */
const userColumns = {
  user: { ...account, emailVerified: user.emailVerified, createdAt: user.createdAt, updatedAt: user.updatedAt },
  publisher: publishers, adminGrantedAt: admins.grantedAt, extensionCount,
  installCount: sql<number>`(select count(*)::int from ${installs} where ${installs.userId} = ${user.id} and ${installs.removedAt} is null)`,
  activeSessions: sql<number>`(select count(*)::int from ${session} where ${session.userId} = ${user.id} and ${session.expiresAt} > now())`,
  lastSessionAt: sql<string | null>`(select max(${session.createdAt})::text from ${session} where ${session.userId} = ${user.id})`,
};
async function userRows(c: Context<Env>, where?: SQL, limit = 50, offset = 0): Promise<AdminAccount[]> {
  const rows = await c.var.data.db.select(userColumns).from(user)
    .leftJoin(publishers, eq(publishers.userId, user.id)).leftJoin(admins, eq(admins.userId, user.id))
    .where(where).orderBy(desc(user.createdAt), desc(user.id)).limit(limit).offset(offset);
  return rows.map((r) => ({ ...r.user, createdAt: r.user.createdAt.toISOString(), updatedAt: r.user.updatedAt.toISOString(),
    publisher: r.publisher ? toPublisher(r.publisher, r.user, r.extensionCount).publisher : null,
    adminGrantedAt: r.adminGrantedAt?.toISOString() ?? null, extensionCount: r.extensionCount,
    installCount: r.installCount, activeSessions: r.activeSessions,
    lastSessionAt: r.lastSessionAt ? new Date(r.lastSessionAt).toISOString() : null,
  }));
}
const grantor = alias(user, 'grantor');
async function grantRows(c: Context<Env>, where?: SQL): Promise<AdminGrant[]> {
  const rows = await c.var.data.db.select({ grant: admins, user: account, grantor: { id: grantor.id, email: grantor.email, name: grantor.name } })
    .from(admins).innerJoin(user, eq(user.id, admins.userId)).leftJoin(grantor, eq(grantor.id, admins.grantedBy))
    .where(where).orderBy(admins.grantedAt, admins.userId);
  return rows.map((row) => ({ user: row.user, grantedAt: row.grant.grantedAt.toISOString(), grantedBy: row.grantor }));
}

export const adminRoutes = new Hono<Env>()
  /* Signed out → 401, before anything else; then a per-admin request budget,
     then the admins row → 403. Writes also draw on an hourly budget. */
  .use('*', async (c, next) => {
    const s = requireSession(c);
    if (!(await c.env.RL_ADMIN.limit({ key: s.userId })).success) rateLimited(c, 60);
    await requireAdmin(c);
    if (c.req.method !== 'GET') await enforce(c, 'admin_write_user', s.userId);
    await next();
    c.header('Cache-Control', 'no-store');
  })
  .get('/session', async (c) => {
    const s = requireSession(c);
    const [me] = await c.var.data.db.select(account).from(user).where(eq(user.id, s.userId)).limit(1);
    if (!me) throw new ApiError({ error: 'unauthorized' });
    return c.json({ user: me });
  })
  .get('/users', async (c) => {
    const { q, filter, limit, offset } = Admin.Users.Req.shape.query.parse(c.req.query());
    const pattern = contains(q);
    const where = and(pattern ? or(ilike(user.email, pattern), ilike(user.name, pattern), ilike(publishers.handle, pattern), ilike(user.id, pattern)) : undefined,
      filter === 'admins' ? isNotNull(admins.userId) : filter === 'publishers' ? isNotNull(publishers.id)
        : filter === 'verified' ? isNotNull(publishers.verifiedAt) : filter === 'unverified-email' ? eq(user.emailVerified, false) : undefined);
    const db = c.var.data.db;
    const [items, totals, counts] = await Promise.all([
      userRows(c, where, limit, offset),
      db.select({ total: sql<number>`count(*)::int` }).from(user).leftJoin(publishers, eq(publishers.userId, user.id)).leftJoin(admins, eq(admins.userId, user.id)).where(where),
      db.select({ users: sql<number>`count(*)::int`, admins: sql<number>`count(${admins.userId})::int`, publishers: sql<number>`count(${publishers.id})::int`, verified: sql<number>`count(${publishers.verifiedAt})::int` })
        .from(user).leftJoin(publishers, eq(publishers.userId, user.id)).leftJoin(admins, eq(admins.userId, user.id)),
    ]);
    return c.json({ items, total: totals[0]!.total, offset, limit, counts: counts[0]! });
  })
  .get('/users/:userId', async (c) => {
    const { userId } = Admin.User.Req.shape.params.parse(c.req.param());
    const [target] = await userRows(c, eq(user.id, userId), 1);
    if (!target) throw new ApiError({ error: 'not_found' });
    const db = c.var.data.db;
    const [providers, sessions, library, publisherDetails, owned] = await Promise.all([
      db.selectDistinct({ provider: authAccount.providerId }).from(authAccount).where(eq(authAccount.userId, userId)),
      db.select({ createdAt: session.createdAt, updatedAt: session.updatedAt, expiresAt: session.expiresAt, userAgent: session.userAgent })
        .from(session).where(and(eq(session.userId, userId), sql`${session.expiresAt} > now()`)).orderBy(desc(session.createdAt), desc(session.id)).limit(20),
      db.select({ repoId: repos.id, name: extensions.name, handle: publishers.handle, slug: repos.slug, version: releases.version, installedAt: installs.installedAt })
        .from(installs).innerJoin(repos, eq(repos.id, installs.repoId)).innerJoin(extensions, eq(extensions.repoId, repos.id))
        .innerJoin(publishers, eq(publishers.id, repos.ownerId)).innerJoin(releases, eq(releases.id, installs.releaseId))
        .where(and(eq(installs.userId, userId), sql`${installs.removedAt} is null`)).orderBy(desc(installs.installedAt), desc(installs.id)).limit(100),
      target.publisher ? publisherById(c, target.publisher.id) : Promise.resolve(null),
      target.publisher ? extensionRows(c, eq(repos.ownerId, target.publisher.id), 100) : Promise.resolve([]),
    ]);
    return c.json({ ...target, providers: providers.map((p) => p.provider), publisherDetails, extensions: owned,
      sessions: sessions.map((s) => ({ ...s, createdAt: s.createdAt.toISOString(), updatedAt: s.updatedAt.toISOString(), expiresAt: s.expiresAt.toISOString() })),
      library: library.map((i) => ({ ...i, installedAt: i.installedAt.toISOString() })),
    });
  })
  /* Reserved handles (src/handles.ts) can only be claimed here, by an admin,
     for an existing user: this is how `powermove` gets its publisher before
     the built-ins are published. */
  .post('/publishers', async (c) => {
    const body = Admin.SeedPublisher.Req.shape.body.parse(await c.req.json().catch(() => null));
    try {
      const publisher = await c.var.data.tx(async (tx) => {
        const [target] = await tx.select({ id: user.id }).from(user).where(eq(user.id, body.userId)).limit(1);
        if (!target) throw new ApiError({ error: 'not_found' });
        const [existing] = await tx.select().from(publishers).where(eq(publishers.userId, body.userId)).limit(1);
        if (existing) throw new ApiError({ error: 'handle_already_set' });
        const [p] = await tx.insert(publishers).values({ handle: body.handle, userId: body.userId }).returning();
        await tx.update(user).set({ username: body.handle, displayUsername: body.handle }).where(eq(user.id, body.userId));
        return p!;
      });
      console.log('admin seeded publisher', { handle: body.handle, userId: body.userId });
      return c.json({ publisher: { id: publisher.id, handle: publisher.handle, tombstoned: false, verified: publisher.verifiedAt !== null } }, 201);
    } catch (e) {
      if (e instanceof ApiError) throw e;
      if (uniqueViolation(e)) throw new ApiError({ error: 'handle_taken' });
      throw e;
    }
  })
  .get('/publishers', async (c) => {
    const { q } = Admin.Publishers.Req.shape.query.parse(c.req.query());
    const pattern = contains(q);
    return c.json({ items: await publisherRows(c, pattern ? or(ilike(publishers.handle, pattern), ilike(user.email, pattern)) : undefined, 50) });
  })
  .get('/publishers/:publisherId', async (c) => {
    const { publisherId } = Admin.Publisher.Req.shape.params.parse(c.req.param());
    const publisher = await publisherById(c, publisherId);
    return c.json({ ...publisher, extensions: await extensionRows(c, eq(repos.ownerId, publisherId)) });
  })
  .put('/publishers/:publisherId/verified', async (c) => {
    const s = requireSession(c);
    const { publisherId } = Admin.SetVerified.Req.shape.params.parse(c.req.param());
    const { verified } = Admin.SetVerified.Req.shape.body.parse(await c.req.json().catch(() => null));
    await c.var.data.tx(async (tx) => {
      const [p] = await tx.select().from(publishers).where(eq(publishers.id, publisherId)).for('update').limit(1);
      if (!p) throw new ApiError({ error: 'not_found' });
      if ((p.verifiedAt !== null) === verified) return;
      await tx.update(publishers).set({ verifiedAt: verified ? new Date() : null }).where(eq(publishers.id, publisherId));
      await tx.insert(moderationLog).values({ publisherId, action: verified ? 'verify' : 'unverify', reason: '', actor: s.userId });
    });
    return c.json(await publisherById(c, publisherId));
  })
  .get('/extensions', async (c) => {
    const { q } = Admin.Extensions.Req.shape.query.parse(c.req.query());
    const pattern = contains(q);
    return c.json({ items: await extensionRows(c, pattern ? or(ilike(extensions.name, pattern), ilike(repos.slug, pattern), ilike(publishers.handle, pattern)) : undefined, 50) });
  })
  .get('/extensions/:repoId', async (c) => {
    const { repoId } = Admin.Extension.Req.shape.params.parse(c.req.param());
    const [row] = await extensionRows(c, eq(repos.id, repoId));
    if (!row) throw new ApiError({ error: 'not_found' });
    return c.json(row);
  })
  .post('/repos/:repoId/moderation', async (c) => {
    const s = requireSession(c);
    const { repoId } = Admin.Moderate.Req.shape.params.parse(c.req.param());
    const body = Admin.Moderate.Req.shape.body.parse(await c.req.json().catch(() => null));
    const repo = await c.var.data.tx(async (tx) => {
      const [previous] = await tx.select().from(repos).where(eq(repos.id, repoId)).for('update').limit(1);
      if (!previous) throw new ApiError({ error: 'not_found' });
      if (body.action === 'unhide' && previous.moderation === 'removed') throw new ApiError({ error: 'bad_request' });
      const [updated] = await tx.update(repos).set({ moderation: body.action === 'unhide' ? 'none' : body.action === 'hide' ? 'hidden' : 'removed', updatedAt: new Date() }).where(eq(repos.id, repoId)).returning();
      if (!updated) throw new ApiError({ error: 'not_found' });
      await tx.insert(moderationLog).values({ repoId, action: body.action, reason: body.reason, actor: s.userId });
      return updated;
    });
    const [owner] = await c.var.data.db.select().from(publishers).where(eq(publishers.id, repo.ownerId)).limit(1);
    return c.json(await listing(c, repo, owner!));
  })
  .get('/admins', async (c) => c.json({ items: await grantRows(c) }))
  .post('/admins', async (c) => {
    const s = requireSession(c);
    const { email } = Admin.Grant.Req.shape.body.parse(await c.req.json().catch(() => null));
    const created = await c.var.data.tx(async (tx) => {
      const [target] = await tx.select({ id: user.id }).from(user).where(sql`lower(${user.email}) = ${email.trim().toLowerCase()}`).limit(1);
      if (!target) throw new ApiError({ error: 'not_found', detail: 'No Powermove account uses that email.' });
      const inserted = await tx.insert(admins).values({ userId: target.id, grantedBy: s.userId }).onConflictDoNothing().returning();
      if (inserted.length) await tx.insert(moderationLog).values({ userId: target.id, action: 'grant_admin', reason: '', actor: s.userId });
      return { userId: target.id, created: inserted.length > 0 };
    });
    const [grant] = await grantRows(c, eq(admins.userId, created.userId));
    return c.json(grant!, created.created ? 201 : 200);
  })
  .delete('/admins/:userId', async (c) => {
    const s = requireSession(c);
    const { userId } = Admin.Revoke.Req.shape.params.parse(c.req.param());
    await c.var.data.tx(async (tx) => {
      // Lock every grant so two admins revoking each other can't leave none.
      const all = await tx.select({ userId: admins.userId }).from(admins).for('update');
      if (!all.some((row) => row.userId === userId)) throw new ApiError({ error: 'not_found' });
      if (all.length <= 1) throw new ApiError({ error: 'bad_request', detail: "The last admin can't be removed." });
      await tx.delete(admins).where(eq(admins.userId, userId));
      await tx.insert(moderationLog).values({ userId, action: 'revoke_admin', reason: '', actor: s.userId });
    });
    return c.body(null, 204);
  })
  .get('/log', async (c) => {
    const { limit } = Admin.Log.Req.shape.query.parse(c.req.query());
    const actorUser = alias(user, 'actor_user');
    const actorPublisher = alias(publishers, 'actor_publisher');
    const targetPublisher = alias(publishers, 'target_publisher');
    const targetUser = alias(user, 'target_user');
    const rows = await c.var.data.db.select({
      entry: moderationLog, actorEmail: actorUser.email, actorHandle: actorPublisher.handle,
      repoHandle: extensions.handle, repoSlug: extensions.slug, publisherHandle: targetPublisher.handle, userEmail: targetUser.email,
    }).from(moderationLog)
      .leftJoin(actorUser, eq(actorUser.id, moderationLog.actor))
      .leftJoin(actorPublisher, eq(sql`${actorPublisher.id}::text`, moderationLog.actor))
      .leftJoin(extensions, eq(extensions.repoId, moderationLog.repoId))
      .leftJoin(targetPublisher, eq(targetPublisher.id, moderationLog.publisherId))
      .leftJoin(targetUser, eq(targetUser.id, moderationLog.userId))
      .orderBy(desc(moderationLog.createdAt), desc(moderationLog.id)).limit(limit ?? 100);
    return c.json({
      items: rows.map(({ entry, ...row }): AdminLogEntry => ({
        id: entry.id, action: entry.action, reason: entry.reason, createdAt: entry.createdAt.toISOString(),
        actor: { id: entry.actor, label: row.actorEmail ?? (row.actorHandle ? `@${row.actorHandle}` : null) },
        target: entry.repoId ? { kind: 'repo', id: entry.repoId, label: row.repoSlug ? `${row.repoHandle}/${row.repoSlug}` : null }
          : entry.publisherId ? { kind: 'publisher', id: entry.publisherId, label: row.publisherHandle ? `@${row.publisherHandle}` : null }
          : entry.userId ? { kind: 'user', id: entry.userId, label: row.userEmail }
          : null,
      })),
    });
  })
  .get('/aero/overview', async (c) => c.json(Admin.AeroOverview.Res.parse(await aeroOverview(c.var.data.db, c.env))))
  .delete('/aero/blocks/:userId', async (c) => {
    const s = requireSession(c);
    const userId = c.req.param('userId');
    await c.var.data.tx(async (tx) => {
      const removed = await tx.delete(aeroBlocks).where(eq(aeroBlocks.userId, userId)).returning({ userId: aeroBlocks.userId });
      if (!removed.length) throw new ApiError({ error: 'not_found' });
      await tx.insert(moderationLog).values({ userId, action: 'unblock_aero', reason: '', actor: s.userId });
    });
    return c.body(null, 204);
  });
