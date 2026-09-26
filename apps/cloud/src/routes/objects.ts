import { Hono } from 'hono';
import { and, eq, sql } from 'drizzle-orm';
import { ApiError, Objects } from '@powermove/registry/wire';
import { decodeLoose, GitCodecError, hashObject, isSha1Hex, parseCommit, parseTree, sha256Hex } from '@powermove/registry/git';
import { REGISTRY_LIMITS, UPLOAD_ENVELOPE } from '@powermove/registry/limits';
import type { Env } from '../env';
import { objectLeases, objects } from '../db/schema';
import { hasReleaseReference } from '../objects/references';
import { parseMultipart } from '../objects/multipart';
import { presentShas } from '../objects/presence';
import { requireSession } from './session';
import { enforce, rateLimited } from '../abuse';

const leaseExpiry = () => new Date(Date.now() + UPLOAD_ENVELOPE.leaseHours * 3_600_000);
const envelope = () => new ApiError({ error: 'too_large', limit: 'envelope' });
type Rejection = 'hash_mismatch' | 'bad_object' | 'too_large' | 'object_conflict';

export const objectRoutes = new Hono<Env>()
  .post('/missing', async c => {
    const userId = requireSession(c).userId;
    const limited = await c.env.RL_UPLOAD.limit({key:userId});
    if (!limited.success) rateLimited(c, 60);
    await enforce(c, 'missing_req_user', userId);
    const body = Objects.Missing.Req.shape.body.parse(await c.req.json().catch(() => null));
    const shas = [...new Set(body.shas)];
    const present = await presentShas(c.var.data.db, userId, shas, body, c.env.OBJECTS);
    if (present.size) await c.var.data.db.insert(objectLeases).values([...present].map(sha => ({userId,sha,expiresAt:leaseExpiry()})))
      .onConflictDoUpdate({target:[objectLeases.userId,objectLeases.sha],set:{expiresAt:leaseExpiry()}});
    return c.json({ missing: shas.filter(sha => !present.has(sha)) });
  })
  .post('/', async c => {
    const userId = requireSession(c).userId;
    const limited = await c.env.RL_UPLOAD.limit({ key: userId });
    if (!limited.success) rateLimited(c, 60);
    await enforce(c, 'upload_req_user', userId);
    const length = Number(c.req.header('content-length'));
    if (Number.isFinite(length) && length > UPLOAD_ENVELOPE.compressedBytesPerRequest) throw envelope();
    if (!c.req.raw.body) throw new ApiError({ error: 'bad_request' });
    const contentType = c.req.header('content-type') ?? '';
    const lease = async (sha: string, size: number) => c.var.data.tx(async tx => {
      await tx.execute(sql`select id from "user" where id = ${userId} for update`);
      const [existing] = await tx.select().from(objectLeases).where(and(eq(objectLeases.userId,userId),eq(objectLeases.sha,sha),sql`${objectLeases.expiresAt} > now()`)).limit(1);
      const [{used}] = await tx.select({used:sql<number>`coalesce(sum(${objects.size}),0)`}).from(objectLeases).innerJoin(objects,eq(objectLeases.sha,objects.sha))
        .where(and(eq(objectLeases.userId,userId),sql`${objectLeases.expiresAt} > now()`,sql`not ${hasReleaseReference(sql`${objects.sha}`)}`));
      if (!existing && Number(used) + size > UPLOAD_ENVELOPE.orphanQuotaBytes) throw new ApiError({error:'quota_exceeded'});
      await tx.insert(objectLeases).values({userId,sha,expiresAt:leaseExpiry()})
        .onConflictDoUpdate({target:[objectLeases.userId,objectLeases.sha],set:{expiresAt:leaseExpiry()}});
      return !existing;
    });
    let inflated = 0;
    const stored: string[] = [], present: string[] = [], rejected: {sha:string;code:Rejection}[] = [];
    for await (const part of parseMultipart(c.req.raw.body, contentType)) {
      const sha = part.name;
      if (!isSha1Hex(sha)) throw new ApiError({ error: 'bad_request', detail: 'invalid object name' });
      if (part.contentType !== 'application/x-git-loose-object') { rejected.push({ sha, code: 'bad_object' }); continue; }
      let decoded: Awaited<ReturnType<typeof decodeLoose>>;
      try {
        decoded = await decodeLoose(part.bytes, REGISTRY_LIMITS.fileBytes + 64);
        if (decoded.type === 'tree') parseTree(decoded.body);
        if (decoded.type === 'commit') parseCommit(decoded.body);
      } catch (error) {
        rejected.push({ sha, code: error instanceof GitCodecError && error.code === 'too_large' ? 'too_large' : 'bad_object' });
        continue;
      }
      inflated += decoded.body.length;
      if (inflated > UPLOAD_ENVELOPE.inflatedBytesPerRequest) throw envelope();
      if (await hashObject(decoded.type, decoded.body) !== sha) { rejected.push({sha,code:'hash_mismatch'}); continue; }
      const header = new TextEncoder().encode(`${decoded.type} ${decoded.body.length}\0`);
      const canonical = new Uint8Array(header.length + decoded.body.length);
      canonical.set(header);
      canonical.set(decoded.body,header.length);
      const sha256 = await sha256Hex(canonical), size = decoded.body.length;
      const alreadyPresent = (await presentShas(c.var.data.db,userId,[sha],{},c.env.OBJECTS)).has(sha);
      const [inserted] = await c.var.data.db.insert(objects).values({sha,type:decoded.type,size,sha256,gcState:'live',lastSeenAt:new Date()})
        .onConflictDoNothing().returning({sha:objects.sha});
      let existingState: 'live'|'claimed'|'deleting' = 'live';
      if (!inserted) {
        const [row] = await c.var.data.db.select().from(objects).where(eq(objects.sha,sha)).limit(1);
        if (!row) throw new ApiError({ error: 'internal' });
        if (row.type !== decoded.type || row.size !== size || row.sha256 !== sha256) {
          console.warn('object_conflict', {sha,existing:{type:row.type,size:row.size,sha256:row.sha256},incoming:{type:decoded.type,size,sha256}});
          rejected.push({sha,code:'object_conflict'}); continue;
        }
        existingState = row.gcState;
        if (row.gcState === 'deleting') { rejected.push({sha,code:'bad_object'}); continue; }
        if (row.gcState === 'claimed') {
          const rescued = await c.var.data.db.update(objects).set({gcState:'live',lastSeenAt:new Date()}).where(and(eq(objects.sha,sha),eq(objects.gcState,'claimed'))).returning({sha:objects.sha});
          if (!rescued.length) { rejected.push({sha,code:'bad_object'}); continue; }
        }
      }
      let newLease: boolean;
      try { newLease = await lease(sha,size); }
      catch (error) { if (inserted) await c.var.data.db.delete(objects).where(eq(objects.sha,sha)); throw error; }
      const cleanup = async () => {
        if (newLease) await c.var.data.db.delete(objectLeases).where(and(eq(objectLeases.userId,userId),eq(objectLeases.sha,sha)));
        if (inserted) await c.var.data.db.delete(objects).where(eq(objects.sha,sha));
      };
      const key = `objects/${sha}`;
      const current = inserted || existingState === 'claimed' ? null : await c.env.OBJECTS.head(key);
      if (!current) {
        const put = await c.env.OBJECTS.put(key, part.bytes, { onlyIf: {etagDoesNotMatch:'*'}, customMetadata: {type:decoded.type,size:String(size),sha256} });
        if (!put) {
          const head = await c.env.OBJECTS.head(key);
          if (!head || head.customMetadata?.type !== decoded.type || head.customMetadata?.size !== String(size) || head.customMetadata?.sha256 !== sha256) {
            console.warn('object_conflict', {sha,existing:head?.customMetadata,incoming:{type:decoded.type,size,sha256}});
            await cleanup();
            rejected.push({sha,code:'object_conflict'}); continue;
          }
        }
      } else if (current.customMetadata?.type !== decoded.type || current.customMetadata?.size !== String(size) || current.customMetadata?.sha256 !== sha256) {
        console.warn('object_conflict', {sha,existing:current.customMetadata,incoming:{type:decoded.type,size,sha256}});
        await cleanup();
        rejected.push({sha,code:'object_conflict'}); continue;
      }
      (alreadyPresent ? present : stored).push(sha);
    }
    return c.json({stored,present,rejected});
  });
