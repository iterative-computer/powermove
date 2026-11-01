// Execute the actual tagged atomic writer body and gzip decoder. No Electron,
// profile, network or copied production content. Run only over synthetic temp data.
import {execFileSync} from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import {stripTypeScriptTypes} from 'node:module';
const dir=process.argv[2];
const tag=(rev,p)=>execFileSync('git',['show',`${rev}:apps/desktop/src/${p}`],{encoding:'utf8'});
const registry=tag('v1.2.0','shared/store-keys.ts');
const expected=JSON.parse(await fs.readFile(path.join(dir,'keys.json'),'utf8'));
for(const [index,name] of ['STATIC_KEYS','DYNAMIC_PREFIXES'].entries()){
 const source=registry.split(`export const ${name} = [`)[1].split('] as const')[0].replace(/\/\/[^\n]*/g,'');
 assert.deepEqual([...source.matchAll(/'([^']+)'/g)].map(m=>m[1]),expected[index]);
}
const storage=tag('v1.2.0','main/storage.ts');
assert.match(storage,/const WRITE_DELAY_MS = 150/);
assert.match(storage,/const value: unknown = JSON.parse\(serialized\)/);
const marker='  private async persist(key: string, operation: PendingOperation): Promise<void> {';
const body=storage.split(marker)[1].split('\n  private async removeStaleTemporaryFiles')[0].trimEnd().slice(0,-1);
const persist=new Function('path','isErrorCode',`return async function(key,operation) {${body}}`)(path,(e,c)=>e?.code===c);
const calls=[];
const instrumented={...fs,writeFile:async(...args)=>{calls.push('write');return fs.writeFile(...args);},open:async(...args)=>{calls.push('open');const f=await fs.open(...args);return {sync:async()=>{calls.push('sync');await f.sync();},close:async()=>{calls.push('close');await f.close();}};},rename:async(...args)=>{calls.push('rename');return fs.rename(...args);}};
const context={directory:dir,fs:instrumented,fileNameFor:key=>`electron-${key}.json`,randomSuffix:()=> 'synthetic',emitError:()=>assert.fail('unexpected write error')};
const plain=await fs.readFile(path.join(dir,'plain.json'),'utf8');
await persist.call(context,'theme',{kind:'set',serialized:plain});
assert.deepEqual(calls,['write','open','sync','close','rename']);
assert.equal(await fs.readFile(path.join(dir,'electron-theme.json'),'utf8'),plain);
// Node strips only types; the shipped decoder implementation runs unchanged.
const codec=stripTypeScriptTypes(tag('v1.3.0','main/storage-codec.ts'));
const module=await import(`data:text/javascript;base64,${Buffer.from(codec).toString('base64')}`);
const compressed=await fs.readFile(path.join(dir,'compressed.json'),'utf8');
assert.equal(module.decodeStorageJSON(compressed,1<<20).value,JSON.parse(plain));
assert.equal(module.decodeStorageJSON(compressed,1<<20).compressed,true);
assert.equal(JSON.parse(plain),module.decodeStorageJSON(plain,1<<20).value);
// The v1.2 reader cannot decode gzip: explicit format selection is required.
assert.equal(JSON.parse(compressed).__powermoveDiskValue,1);
const projects=tag('v1.2.0','renderer/src/legacy/core/projects.ts');
assert.match(projects,/PM\.store\.set\(R\.SLOT \+ proj\.id, \{ v: PM\.version \|\| '1\.0\.0', proj \}\)/);
assert.match(projects,/projectHistory\.\$\{id\}/);assert.match(projects,/projectMeta\.\$\{id\}/);
// Run the tag's real slot/session readers and history dictionary decoder.
const historySource=stripTypeScriptTypes(tag('v1.2.0','shared/history-memory.ts'));
const history=await import(`data:text/javascript;base64,${Buffer.from(historySource).toString('base64')}`);
const slice=(start,end)=>projects.slice(projects.indexOf(start),projects.indexOf(end,projects.indexOf(start)));
const cloneBody=projects.split("\n").find(line=>line.startsWith("const clone ="));
const readerBody = `${cloneBody}
const R={SLOT:'project.',JOURNAL:'projectJournal.',STATE:'projectState.'};
${slice('function applyPatches(', '\nfunction journal(')}
${slice('R.unwrap =', '/** Upsert registry')}
${slice('R.getAsync =', '\nR.getStateAsync =')}
${slice('R.getStateAsync =', '\nR.getState =')}
return R;`;
const pm={store:{getAsync:async(key,fallback)=>{
 try {const text=await fs.readFile(path.join(dir,'store',`${key}.json`),'utf8');return history.unpackStoredHistory(key,JSON.parse(text));}
 catch(e){if(e.code==='ENOENT')return fallback;throw e;}
}}};
const reader=new Function('PM','parseAsync','yieldTask',`return (${stripTypeScriptTypes(`function build(){${readerBody}}`)} )();`)(pm,JSON.parse,async()=>{});
const project=await reader.getAsync('P');
assert.equal(project.id,'P');assert.equal(project.name,'Fixture');assert.equal(project.layers[0].p.opacity.v,45);assert.equal(project.future.kept,true);
const session=await reader.getStateAsync('P');
assert.equal(session.history.version,1);assert.equal(session.history.entries.length,1);assert.equal(session.history.index,0);
const compsSource=tag('v1.2.0','renderer/src/legacy/core/compositions.ts');
const compsModule=stripTypeScriptTypes(compsSource.slice(compsSource.indexOf('export const COMP_FIELDS'),compsSource.indexOf('export function install(')));
const comps=await import(`data:text/javascript;base64,${Buffer.from(compsModule).toString('base64')}`);
let rollbackProject=structuredClone(project);
// The shipped undo stack stores composition-relative patches. The tag's
// applyPatches restores its original opacity (undo after native editing).
const applyBody=slice('function applyPatches(', '\nfunction journal(');
const apply=new Function(`return (${stripTypeScriptTypes(`function build(){${cloneBody}
${applyBody};return applyPatches;}`)})();`)();
for(const entry of [...session.history.entries].reverse()){
 rollbackProject=apply(rollbackProject,comps.realizePatches(entry.backward,rollbackProject));
}
const rollbackLayer=rollbackProject.comps?.[rollbackProject.compId]?.layers?.[0]??rollbackProject.layers[0];
assert.equal(rollbackLayer.p.opacity.v,100);
console.log('v1.2 keys, debounce, write/fsync/rename and plain JSON parity; v1.3 gzip decoder: passed');
