import { Hono } from 'hono';
import { and, desc, eq, isNull, sql } from 'drizzle-orm';
import { ApiError, Installs } from '@powermove/registry/wire';
import type { Env } from '../env';
import { extensions, installs, publishers, releases, repos, userSettings } from '../db/schema';
import { canReadFiles } from '../lifecycle';
import { requireSession } from './session';

export const installRoutes = new Hono<Env>()
  .get('/', async c => {
    const session = requireSession(c);
    const rows = await c.var.data.db.select({ install: installs, repo: repos, owner: publishers })
      .from(installs).innerJoin(repos, eq(repos.id, installs.repoId)).innerJoin(publishers, eq(publishers.id, repos.ownerId))
      .where(and(eq(installs.userId, session.userId), isNull(installs.removedAt)))
      .orderBy(desc(installs.installedAt), desc(installs.id));
    return c.json({ items: rows.map(({ install, repo, owner }) => ({ repoId: install.repoId, releaseId: install.releaseId, handle: owner.handle, slug: repo.slug, installedAt: install.installedAt.toISOString() })) });
  })
  .post('/', async c => {
    const session = requireSession(c);
    const { repoId, releaseId } = Installs.Add.Req.shape.body.parse(await c.req.json().catch(() => null));
    await c.var.data.tx(async tx => {
      await tx.execute(sql`select id from "user" where id = ${session.userId} for update`);
      const [settings] = await tx.select().from(userSettings).where(eq(userSettings.userId, session.userId)).limit(1);
      if (settings?.rememberInstalls === false) return;
      const [viewer] = await tx.select({ id: publishers.id }).from(publishers).where(eq(publishers.userId, session.userId)).limit(1);
      const [row] = await tx.select({ repo: repos, release: releases, publisherId: publishers.id })
        .from(releases).innerJoin(repos, eq(repos.id, releases.repoId)).innerJoin(publishers, eq(publishers.id, repos.ownerId))
        .where(and(eq(releases.id, releaseId), eq(releases.repoId, repoId))).limit(1);
      if (!row) throw new ApiError({ error: 'not_found' });
      const access = canReadFiles(row.repo, row.release, { publisherId: viewer?.id ?? null, admin: false });
      if (access !== 'ok') throw access === 'not_found' ? new ApiError({ error: 'not_found' }) : new ApiError({ error: 'gone', reason: access === 'gone_removed' ? 'removed' : access === 'gone_tombstoned' ? 'tombstoned' : 'yanked' });
      const inserted = await tx.insert(installs).values({ userId: session.userId, repoId, releaseId })
        .onConflictDoNothing({ target: [installs.userId, installs.repoId], where: isNull(installs.removedAt) }).returning({ id: installs.id });
      if (inserted.length) await tx.update(extensions).set({ installCount: sql`${extensions.installCount}+1` }).where(eq(extensions.repoId, repoId));
      else await tx.update(installs).set({ releaseId, installedAt: new Date() }).where(and(eq(installs.userId, session.userId), eq(installs.repoId, repoId), isNull(installs.removedAt)));
    });
    return c.body(null, 204);
  })
  .delete('/:repoId', async c => {
    const session = requireSession(c);
    const { repoId } = Installs.Delete.Req.shape.params.parse(c.req.param());
    await c.var.data.tx(async tx => {
      const removed = await tx.update(installs).set({ removedAt: new Date() })
        .where(and(eq(installs.userId, session.userId), eq(installs.repoId, repoId), isNull(installs.removedAt))).returning({ id: installs.id });
      if (removed.length) await tx.update(extensions).set({ installCount: sql`${extensions.installCount}-1` }).where(eq(extensions.repoId, repoId));
    });
    return c.body(null, 204);
  });
