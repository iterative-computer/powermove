/* Share links. The desktop uploads a snapshot (facts about a project or an
   extension, plus an optional poster frame) and gets back a URL whose page is a
   static, read-only view. Quotas come from the plan: free 1 ever, Aerolite 1
   per rolling week, Aero unlimited. */
import { Hono } from 'hono';
import { z } from 'zod';
import { and, desc, eq, isNull, sql } from 'drizzle-orm';
import { ApiError } from '@powermove/registry/wire';
import type { Env } from '../env';
import { shares } from '../db/schema';
import { requireSession } from './session';
import { loadAccount, lockUser, shareHistory, storedBytes } from '../billing/account';
import { checkShareQuota, checkStorage } from '../billing/quotas';
import { renderSharePage } from '../share-page';

const MAX_POSTER_BYTES = 2 * 1024 * 1024;
const text = (max: number) => z.string().trim().max(max);
export const ProjectSnapshot = z.object({
  kind: z.literal('project'),
  title: text(120).min(1),
  description: text(2000).optional(),
  width: z.number().int().positive().max(16384), height: z.number().int().positive().max(16384),
  fps: z.number().positive().max(240), durationSeconds: z.number().nonnegative().max(24 * 3600),
  compositions: z.array(z.object({ name: text(120), durationSeconds: z.number().nonnegative().max(24 * 3600), layerCount: z.number().int().nonnegative() })).max(50).default([]),
  layers: z.array(z.object({ name: text(120), type: text(40), inSeconds: z.number().optional(), outSeconds: z.number().optional() })).max(300).default([]),
});
export const ExtensionSnapshot = z.object({
  kind: z.literal('extension'),
  title: text(120).min(1),
  description: text(2000).optional(),
  handle: text(64).optional(), version: text(40).optional(), tagline: text(200).optional(),
  permissions: z.array(text(40)).max(20).default([]),
  contributes: z.array(z.object({ kind: text(40), name: text(120) })).max(100).default([]),
  readme: text(20000).optional(),
});
export const CreateShare = z.object({
  snapshot: z.discriminatedUnion('kind', [ProjectSnapshot, ExtensionSnapshot]),
  /** base64 PNG or JPEG */
  poster: z.string().max(Math.ceil(MAX_POSTER_BYTES * 4 / 3) + 4).optional(),
});
export type ShareSnapshot = z.infer<typeof CreateShare>['snapshot'];

function decodePoster(base64: string): { bytes: Uint8Array; type: 'image/png' | 'image/jpeg' } {
  let bytes: Uint8Array;
  try { bytes = Uint8Array.from(atob(base64), ch => ch.charCodeAt(0)); } catch { throw new ApiError({ error: 'bad_request', detail: 'invalid poster' }); }
  if (bytes.length > MAX_POSTER_BYTES) throw new ApiError({ error: 'bad_request', detail: 'poster is larger than 2 MB' });
  if ([137, 80, 78, 71].every((v, i) => bytes[i] === v)) return { bytes, type: 'image/png' };
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return { bytes, type: 'image/jpeg' };
  throw new ApiError({ error: 'bad_request', detail: 'poster must be PNG or JPEG' });
}
const ALPHABET = 'abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789';
function shareId(): string { return Array.from(crypto.getRandomValues(new Uint8Array(10)), b => ALPHABET[b % ALPHABET.length]).join(''); }
const shareUrl = (env: CloudflareBindings, id: string) => `${env.APP_ORIGIN}/s/${id}`;

export const shareRoutes = new Hono<Env>()
  .get('/', async (c) => {
    const s = requireSession(c);
    const rows = await c.var.data.db.select({ id: shares.id, kind: shares.kind, title: shares.title, createdAt: shares.createdAt, revokedAt: shares.revokedAt, viewCount: shares.viewCount, sizeBytes: shares.sizeBytes })
      .from(shares).where(eq(shares.userId, s.userId)).orderBy(desc(shares.createdAt)).limit(100);
    return c.json({ items: rows.map(r => ({ ...r, url: shareUrl(c.env, r.id), createdAt: r.createdAt.toISOString(), revokedAt: r.revokedAt?.toISOString() ?? null })) });
  })
  .post('/', async (c) => {
    const s = requireSession(c);
    const body = CreateShare.parse(await c.req.json().catch(() => null));
    const poster = body.poster ? decodePoster(body.poster) : null;
    const snapshotJson = JSON.stringify(body.snapshot);
    const sizeBytes = new TextEncoder().encode(snapshotJson).length + (poster?.bytes.length ?? 0);
    const id = shareId();
    const posterKey = poster ? `shares/${id}/poster` : null;
    const created = await c.var.data.tx(async (tx) => {
      await lockUser(tx, s.userId);
      const now = new Date();
      const account = await loadAccount(tx, s.userId, now);
      const quota = checkShareQuota(account.ent.shares, await shareHistory(tx, s.userId, now), now);
      if (!quota.ok) {
        throw new ApiError({ error: 'quota_exceeded', detail: quota.reason === 'lifetime_used'
          ? 'Free accounts include one share link. Upgrade to Aerolite for a new link every week.'
          : `You've shared this week's link. The next one is available ${quota.nextAvailableAt?.toISOString().slice(0, 10)}.` });
      }
      const storage = checkStorage(account.ent, await storedBytes(tx, s.userId), sizeBytes);
      if (!storage.ok) throw new ApiError({ error: 'quota_exceeded', detail: 'Your Powermove Cloud storage is full.' });
      const [row] = await tx.insert(shares).values({ id, userId: s.userId, kind: body.snapshot.kind, title: body.snapshot.title, snapshot: body.snapshot, posterKey, sizeBytes, interactive: account.ent.interactiveShares }).returning();
      return row!;
    });
    if (poster && posterKey) await c.env.SHARES.put(posterKey, poster.bytes, { httpMetadata: { contentType: poster.type } });
    return c.json({ id: created.id, url: shareUrl(c.env, created.id), interactive: created.interactive }, 201);
  })
  .delete('/:id', async (c) => {
    const s = requireSession(c);
    const [row] = await c.var.data.db.update(shares).set({ revokedAt: new Date() })
      .where(and(eq(shares.id, c.req.param('id')), eq(shares.userId, s.userId), isNull(shares.revokedAt))).returning();
    if (!row) throw new ApiError({ error: 'not_found' });
    if (row.posterKey) await c.env.SHARES.delete(row.posterKey);
    return c.json({ ok: true });
  });

/** Public pages, mounted at /s. */
export const sharePages = new Hono<Env>()
  .get('/:id', async (c) => {
    const [row] = await c.var.data.db.select().from(shares).where(eq(shares.id, c.req.param('id'))).limit(1);
    const nonce = crypto.randomUUID();
    c.header('Content-Security-Policy', `default-src 'none'; style-src 'nonce-${nonce}'; img-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'`);
    c.header('X-Robots-Tag', 'noindex');
    if (!row || row.revokedAt) return c.html(renderSharePage(null, nonce), 404);
    await c.var.data.db.update(shares).set({ viewCount: sql`${shares.viewCount} + 1` }).where(eq(shares.id, row.id));
    return c.html(renderSharePage({ id: row.id, snapshot: row.snapshot as ShareSnapshot, hasPoster: Boolean(row.posterKey), createdAt: row.createdAt }, nonce));
  })
  .get('/:id/poster', async (c) => {
    const [row] = await c.var.data.db.select({ posterKey: shares.posterKey, revokedAt: shares.revokedAt }).from(shares).where(eq(shares.id, c.req.param('id'))).limit(1);
    if (!row?.posterKey || row.revokedAt) return c.json({ error: 'not_found' }, 404);
    const object = await c.env.SHARES.get(row.posterKey);
    if (!object) return c.json({ error: 'not_found' }, 404);
    return new Response(object.body, { headers: { 'Content-Type': object.httpMetadata?.contentType ?? 'image/png', 'Cache-Control': 'public, max-age=300', 'X-Content-Type-Options': 'nosniff' } });
  });
