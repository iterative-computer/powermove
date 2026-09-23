import { Hono } from 'hono';
import { and, desc, eq, isNull, sql } from 'drizzle-orm';
import { ApiError, Admin, Publish, Store } from '@powermove/registry/wire';
import { sha256Hex } from '@powermove/registry/git';
import type { Env } from '../env';
import { extensions, moderationLog, publishers, releases, repos, reports } from '../db/schema';
import { lineageFor, toListing, toRelease } from '../dto';
import { requirePublisher, rateAuth } from './session';
import { canViewDetail } from '../lifecycle';

async function owned(c: import('hono').Context<Env>, handle: string, slug: string) {
  const owner=await requirePublisher(c);
  if (handle !== owner.handle) throw new ApiError({error:'not_owner'});
  const [repo]=await c.var.data.db.select().from(repos).where(and(eq(repos.ownerId,owner.id),eq(repos.slug,slug))).limit(1);
  if (!repo) throw new ApiError({error:'not_found'});
  if (repo.tombstonedAt) throw new ApiError({error:'gone',reason:'tombstoned'});
  return {owner,repo};
}
async function listing(c: import('hono').Context<Env>, repo: typeof repos.$inferSelect, owner: typeof publishers.$inferSelect) {
  const [extension]=await c.var.data.db.select().from(extensions).where(eq(extensions.repoId,repo.id)).limit(1);
  if (!extension) throw new ApiError({error:'internal'});
  const [latest]=extension.latestReleaseId ? await c.var.data.db.select().from(releases).where(eq(releases.id,extension.latestReleaseId)).limit(1) : [];
  return toListing(repo,extension,owner,latest ?? null,await lineageFor(c.var.data.db,repo));
}
async function icon(bucket:R2Bucket, raw?:string) {
  if (raw === undefined) return undefined;
  let bytes:Uint8Array;
  try { bytes=Uint8Array.from(atob(raw),x=>x.charCodeAt(0)); } catch { throw new ApiError({error:'bad_request',detail:'invalid PNG'}); }
  if (bytes.length > 256*1024 || bytes.length<8 || ![137,80,78,71,13,10,26,10].every((v,i)=>bytes[i]===v)) throw new ApiError({error:'bad_request',detail:'invalid PNG'});
  const key=`${await sha256Hex(bytes)}.png`;
  await bucket.put(`icons/${key}`,bytes,{onlyIf:{etagDoesNotMatch:'*'},httpMetadata:{contentType:'image/png'}});
  return key;
}
export const repoManagement = new Hono<Env>()
 .post('/:handle/:slug/releases/:version/yank',async c=>{
   const {handle,slug,version}=Publish.Yank.Req.shape.params.parse(c.req.param());
   const {owner,repo}=await owned(c,handle,slug);
   const result=await c.var.data.tx(async tx=>{
     await tx.execute(sql`select id from repos where id = ${repo.id}::uuid for update`);
     const [release]=await tx.select().from(releases).where(and(eq(releases.repoId,repo.id),eq(releases.version,version))).limit(1);
     if (!release) throw new ApiError({error:'not_found'});
     const [yanked]=await tx.update(releases).set({yankedAt:release.yankedAt ?? new Date()}).where(eq(releases.id,release.id)).returning();
     const [latest]=await tx.select().from(releases).where(and(eq(releases.repoId,repo.id),isNull(releases.yankedAt))).orderBy(desc(releases.publishedAt),desc(releases.id)).limit(1);
     await tx.update(extensions).set({latestReleaseId:latest?.id ?? null}).where(eq(extensions.repoId,repo.id));
     await tx.insert(moderationLog).values({repoId:repo.id,releaseId:release.id,action:'yank',reason:'',actor:owner.id});
     return yanked!;
   });
   return c.json({repo:await listing(c,repo,owner),release:toRelease(result)});
 })
 .patch('/:handle/:slug',async c=>{
   const {handle,slug}=Publish.PatchRepo.Req.shape.params.parse(c.req.param());
   const body=Publish.PatchRepo.Req.shape.body.parse(await c.req.json().catch(()=>null));
   const {owner,repo}=await owned(c,handle,slug);
   const iconKey=await icon(c.env.ICONS,body.iconPng);
   const [updated]=await c.var.data.db.update(repos).set({...(body.visibility?{visibility:body.visibility}:{}),updatedAt:new Date()}).where(eq(repos.id,repo.id)).returning();
   if (body.listing || iconKey) await c.var.data.db.update(extensions).set({...body.listing,...(iconKey?{iconKey}:{})}).where(eq(extensions.repoId,repo.id));
   return c.json(await listing(c,updated!,owner));
 })
 .delete('/:handle/:slug',async c=>{
   const {handle,slug}=Publish.DeleteRepo.Req.shape.params.parse(c.req.param());
   const {owner,repo}=await owned(c,handle,slug);
   await c.var.data.tx(async tx=>{
     await tx.update(repos).set({tombstonedAt:new Date(),updatedAt:new Date()}).where(eq(repos.id,repo.id));
     await tx.insert(moderationLog).values({repoId:repo.id,action:'tombstone',reason:'',actor:owner.id});
   });
   return c.body(null,204);
 });

function constantTimeEqual(a:string,b:string) {
  const x=new TextEncoder().encode(a),y=new TextEncoder().encode(b);
  let diff=x.length^y.length;
  for(let i=0;i<Math.max(x.length,y.length);i++) diff |= (x[i]??0)^(y[i]??0);
  return diff===0;
}
export const adminRoutes = new Hono<Env>().post('/repos/:repoId/moderation',async c=>{
  if (!c.env.ADMIN_TOKEN || !constantTimeEqual(c.req.header('X-Admin-Token')??'',c.env.ADMIN_TOKEN)) throw new ApiError({error:'unauthorized'});
  const {repoId}=Admin.Moderate.Req.shape.params.parse(c.req.param());
  const body=Admin.Moderate.Req.shape.body.parse(await c.req.json().catch(()=>null));
  const repo=await c.var.data.tx(async tx=>{
    const [updated]=await tx.update(repos).set({moderation:body.action==='unhide'?'none':body.action==='hide'?'hidden':'removed',updatedAt:new Date()}).where(eq(repos.id,repoId)).returning();
    if (!updated) throw new ApiError({error:'not_found'});
    await tx.insert(moderationLog).values({repoId,action:body.action,reason:body.reason,actor:'admin'});
    return updated;
  });
  const [owner]=await c.var.data.db.select().from(publishers).where(eq(publishers.id,repo.ownerId)).limit(1);
  return c.json(await listing(c,repo,owner!));
});
export const storeUtility = new Hono<Env>()
 .get('/icons/:key',async c=>{
   const {key}=Store.Icon.Req.shape.params.parse(c.req.param());
   if (!/^[a-f0-9]{64}\.png$/.test(key)) throw new ApiError({error:'not_found'});
   const obj=await c.env.ICONS.get(`icons/${key}`);
   if (!obj) throw new ApiError({error:'not_found'});
   return new Response(obj.body,{headers:{'Content-Type':'image/png','Cache-Control':'public, max-age=31536000, immutable'}});
 })
 .post('/x/:handle/:slug/report',async c=>{
   await rateAuth(c);
   const {handle,slug}=Store.Report.Req.shape.params.parse(c.req.param());
   const body=Store.Report.Req.shape.body.parse(await c.req.json().catch(()=>null));
   const [row]=await c.var.data.db.select({repo:repos,owner:publishers}).from(repos).innerJoin(publishers,eq(repos.ownerId,publishers.id)).where(and(eq(publishers.handle,handle),eq(repos.slug,slug))).limit(1);
   if (!row) throw new ApiError({error:'not_found'});
   const decision=canViewDetail(row.repo,{publisherId:null,admin:false});
   if (decision==='not_found') throw new ApiError({error:'not_found'});
   if (decision==='gone_removed') throw new ApiError({error:'gone',reason:'removed'});
   if (decision==='gone_tombstoned') throw new ApiError({error:'gone',reason:'tombstoned'});
   await c.var.data.db.insert(reports).values({repoId:row.repo.id,reporterId:c.var.session?.userId ?? null,reason:body.reason});
   return c.body(null,204);
 });
