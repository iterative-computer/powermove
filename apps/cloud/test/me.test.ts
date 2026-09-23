import { test,expect } from 'bun:test';
import { createApp } from '../src/app';
import { withData } from './db';
import { makeEnv } from './env';
import { seedSession } from './helpers';
import { publishers, repos, releases, installs, objects, objectLeases } from '../src/db/schema';
import { eq } from 'drizzle-orm';
const app=createApp({data:()=>{throw new Error('replaced')}});
function api(data:Parameters<typeof makeEnv>[0]) { return createApp({data:()=>data}); }
test('me, handle claim, settings and account deletion', async()=>withData(async data=>{
 const env=makeEnv(data), app=api(data), s=await seedSession(data,env); const request=(path:string,init?:RequestInit)=>app.request(path,{...init,headers:{...s.headers,...init?.headers}},env);
 let r=await request('/v1/me'); expect(r.status).toBe(200);expect((await r.json() as any).publisher).toBeNull();
 r=await request('/v1/me/repos');expect(r.status).toBe(200);expect(await r.json() as any).toEqual({items:[]});
 for(const [handle,error] of [['X','handle_invalid'],['admin','handle_reserved']]) { r=await request('/v1/me/handle',{method:'POST',body:JSON.stringify({handle}),headers:{'Content-Type':'application/json'}});expect(r.status).toBe(400);expect((await r.json() as any).error).toBe(error); }
 r=await request('/v1/me/handle',{method:'POST',body:JSON.stringify({handle:'jude-kim'}),headers:{'Content-Type':'application/json'}});expect(r.status).toBe(200);expect((await r.json() as any).publisher.handle).toBe('jude-kim');
 r=await request('/v1/me/handle',{method:'POST',body:JSON.stringify({handle:'other'}),headers:{'Content-Type':'application/json'}});expect(r.status).toBe(409);expect((await r.json() as any).error).toBe('handle_already_set');
 r=await request('/v1/me/settings',{method:'PATCH',body:JSON.stringify({rememberInstalls:false}),headers:{'Content-Type':'application/json'}});expect(r.status).toBe(200);expect((await r.json() as any).settings.rememberInstalls).toBe(false);
 r=await request('/v1/me');expect((await r.json() as any).settings.rememberInstalls).toBe(false);
 const [owner]=await data.db.select().from(publishers).where(eq(publishers.handle,'jude-kim'));
 const [repo]=await data.db.insert(repos).values({ownerId:owner.id,slug:'sample'}).returning();
 const [release]=await data.db.insert(releases).values({repoId:repo.id,version:'1.0.0',commitSha:'a'.repeat(40),treeSha:'b'.repeat(40),tarKey:'tar',tarSha256:'c'.repeat(64),manifest:{},apiVersion:2,fileCount:1,sizeBytes:10,scanWaivers:[]}).returning();
 await data.db.insert(installs).values({userId:s.id,repoId:repo.id,releaseId:release.id});
 await data.db.insert(objects).values({sha:'d'.repeat(40),type:'blob',size:1,sha256:'e'.repeat(64)});
 await data.db.insert(objectLeases).values({userId:s.id,sha:'d'.repeat(40),expiresAt:new Date(Date.now()+10000)});
 r=await request('/v1/me',{method:'DELETE'});expect(r.status).toBe(204);
 expect(await data.db.select().from(installs).where(eq(installs.userId,s.id))).toHaveLength(0);
 expect(await data.db.select().from(objectLeases).where(eq(objectLeases.userId,s.id))).toHaveLength(0);
 expect(await data.db.select().from(repos).where(eq(repos.id,repo.id))).toHaveLength(1);
 const [p]=await data.db.select().from(publishers).where(eq(publishers.handle,'jude-kim'));expect(p.userId).toBeNull();expect(p.tombstonedAt).not.toBeNull();
 const s2=await seedSession(data,env,'second@example.com');r=await app.request('/v1/me/handle',{method:'POST',body:JSON.stringify({handle:'jude-kim'}),headers:{Authorization:`Bearer ${s2.token}`,'Content-Type':'application/json'}},env);expect(r.status).toBe(409);
}));
