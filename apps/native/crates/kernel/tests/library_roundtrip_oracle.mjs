// Execute tagged readers on generated data; never part of the native app.
import {execFileSync} from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import * as nodeModule from 'node:module';
const strip=nodeModule.stripTypeScriptTypes ?? (source => new Bun.Transpiler({loader:'ts', target:'node'}).transformSync(source));
const [dir,tag]=process.argv.slice(2);
const source=p=>execFileSync('git',['show',`${tag}:apps/desktop/src/${p}`],{encoding:'utf8'});
const load=async text=>import(`data:text/javascript;base64,${Buffer.from(strip(text)).toString('base64')}`);
const history=await load(source('shared/history-memory.ts'));
const codec=tag==='v1.3.0' ? await load(source('main/storage-codec.ts')) : null;
const projects=source('renderer/src/legacy/core/projects.ts');
const slice=(start,end)=>{
 assert(projects.includes(start),`missing tagged start ${start}`);
 const from=projects.indexOf(start), to=projects.indexOf(end,from);
 assert(to>from,`missing tagged end ${end}`);
 return projects.slice(from,to);
};
const clone=projects.split('\n').find(line=>line.startsWith('const clone ='));
assert(clone);
const pm={store:{getAsync:async(key,fallback)=>{
 try {
  const text=await fs.readFile(path.join(dir,'store',`${key}.json`),'utf8');
  return history.unpackStoredHistory(key,codec ? codec.decodeStorageJSON(text,1<<28).value : JSON.parse(text));
 }catch(e){if(e.code==='ENOENT')return fallback;throw e;}
}}};
const body=`${clone}
${projects.split("\n").find(line=>line.startsWith("const sequences =")) ?? ""}
const R={SLOT:'project.',JOURNAL:'projectJournal.',STATE:'projectState.'};
${slice('function applyPatches(', '\nfunction journal(')}
${slice('R.unwrap =', '/** Upsert registry')}
${slice('R.getAsync =', '\nR.getStateAsync =')}
${slice('R.getStateAsync =', '\nR.getState =')}
return {R,applyPatches};`;
const {R,applyPatches}=new Function('PM','parseAsync','yieldTask',`return (${strip(`function build(){${body}}`)})();`)(pm,JSON.parse,async()=>{});
const expected=JSON.parse(await fs.readFile(path.join(dir,'expected.json'),'utf8'));
const project=await R.getAsync('P');
assert.deepEqual(project,expected.project);
const session=await R.getStateAsync('P');
assert.deepEqual(session.history,history.unpackStoredHistory('projectHistory.P',expected.history));
assert.equal(project.layers[0].from,0.1234567890123456);
assert.equal(project.layers[0].p['position.x'].v,0.1234567890123456);
assert.equal(project.layers[0].d.text,'Unicode: 🧭 é');
if(session.history.entries.length){
 const compsSource=source('renderer/src/legacy/core/compositions.ts');
 const comps=await load(compsSource.slice(compsSource.indexOf('export const COMP_FIELDS'),compsSource.indexOf('export function install(')));
 const entry=session.history.entries[Math.max(0,session.history.index)];
 if(session.history.index < 0) {
  assert.equal(project.fps,23.976);
  const redone=applyPatches(structuredClone(project),comps.realizePatches(entry.forward,project));
  assert.equal(redone.comps?.[redone.compId]?.fps??redone.fps,29.97);
 } else {
 assert.equal(project.fps,29.97);
 const undone=applyPatches(structuredClone(project),comps.realizePatches(entry.backward,project));
 assert.equal(undone.comps?.[undone.compId]?.fps??undone.fps,23.976);
 const redone=applyPatches(undone,comps.realizePatches(entry.forward,undone));
 assert.equal(redone.comps?.[redone.compId]?.fps??redone.fps,29.97);
 }
}
console.log(`${tag} full library project/history round trip passed`);
