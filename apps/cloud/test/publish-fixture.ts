import { encodeCommit, encodeLoose, hashObject } from '@powermove/registry/git';
import { snapshot, type SnapshotInput } from '@powermove/registry/snapshot';
import type { Data } from '../src/db/client';
import { publishers } from '../src/db/schema';
import { createApp } from '../src/app';
import { seedSession } from './helpers';
import type { ExtensionPermission } from '@powermove/registry/manifest';
const enc=new TextEncoder();
export function files(slug='demo',version='1.0.0',extra:SnapshotInput[]=[],permissions?:ExtensionPermission[]):SnapshotInput[]{return [{path:'manifest.json',bytes:enc.encode(JSON.stringify({id:slug,name:'Demo',version,apiVersion:3,description:'A test extension',...(permissions?{permissions}:{})}))},{path:'index.ts',bytes:enc.encode('export default 1\n')},{path:'nested/readme.md',bytes:enc.encode('nested file\n')},...extra];}
export async function publisher(data:Data,env:CloudflareBindings,handle:string){const session=await seedSession(data,env,`${handle}@example.com`);const [row]=await data.db.insert(publishers).values({handle,userId:session.id}).returning();return {...session,publisher:row!};}
export async function uploadTree(data:Data,env:CloudflareBindings,actor:Awaited<ReturnType<typeof publisher>>,input:SnapshotInput[],options:{parents?:string[];treeOverride?:string;author?:string}={}) {
  const snap=await snapshot(input);
  const identity={name:options.author??actor.publisher.handle,email:`${options.author??actor.publisher.handle}@${env.HANDLE_MAIL_DOMAIN}`,time:1,tz:'+0000'};
  const body=encodeCommit({tree:options.treeOverride??snap.treeSha,parents:options.parents??[],author:identity,committer:identity,message:'release'});
  const commitSha=await hashObject('commit',body);
  const parts=await Promise.all([...snap.objects,{sha:commitSha,type:'commit' as const,body}].map(async x=>({sha:x.sha,bytes:await encodeLoose(x.type,x.body)})));
  const form=new FormData();for(const p of parts) form.append(p.sha,new Blob([new Uint8Array(p.bytes)],{type:'application/x-git-loose-object'}),'object');
  const response=await createApp({data:()=>data}).request('/v1/objects',{method:'POST',headers:actor.headers,body:form},env);
  if(response.status!==200) throw new Error(`upload failed ${response.status}: ${await response.text()}`);
  return {snap,commitSha,parts};
}
export function publishRequest(data:Data,env:CloudflareBindings,actor:Awaited<ReturnType<typeof publisher>>,slug:string,commitSha:string,options:Record<string,unknown>={},beforeCommit?:()=>Promise<void>){return createApp({data:()=>data,beforeCommit}).request(`/v1/repos/${actor.publisher.handle}/${slug}/releases`,{method:'PUT',headers:{...actor.headers,'Content-Type':'application/json'},body:JSON.stringify({version:'1.0.0',commitSha,listing:{name:'Demo',tagline:'Example',category:'tools'},waivers:[],...options})},env);}
