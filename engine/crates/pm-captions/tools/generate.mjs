// Development-only oracle: captures public synthetic fixtures from the tag.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';
import {writeFileSync,readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
const root=execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
if(execFileSync('git',['rev-parse','v1.2.0^{commit}'],{cwd:root,encoding:'utf8'}).trim()!=='07ed5fa17ee640659cb075f9cad1af2095011d60')throw new Error('v1.2.0 reference commit changed');
const base='apps/desktop/src/renderer/src/captions/';
const source=p=>execFileSync('git',['show',`v1.2.0:${p}`],{cwd:root,encoding:'utf8'});
const clean=code=>stripTypeScriptTypes(code.replace(/^import[\s\S]*?;\s*/gm,'').replace(/^export type .*;\s*/gm,'').replace(/^export /gm,''));
const types=source('apps/desktop/src/renderer/src/core/types/captions.ts');
const model=source(base+'model.ts'),formats=source(base+'formats.ts');
const names=[...model.matchAll(/export function (\w+)/g)].map(m=>m[1]);
const formatNames=[...formats.matchAll(/export function (\w+)/g)].map(m=>m[1]);
const api=runInNewContext(clean(types)+'\n'+clean(model)+'\n'+clean(formats)+`\n({${[...names,...formatNames,'CAPTION_PRESETS','DEFAULT_CAPTION_STYLE'].join(',')}})`, {Date:class extends Date {static now(){return 0}},Math:Object.assign(Object.create(Math),{random:()=>0.5})});
const cases=[];
const isCue=x=>x && typeof x==='object' && typeof x.id==='string' && 'start' in x && 'end' in x && 'text' in x;
const sanitize=x=>{
 if(typeof x==='function') return '@ids';
 if(Array.isArray(x))return x.map(sanitize);
 if(x && typeof x==='object')return Object.fromEntries(Object.entries(x).map(([k,v])=>[k,sanitize(v)]));
 return x;
};
const eraseIds=x=>{
 if(Array.isArray(x))return x.map(eraseIds);
 if(x && typeof x==='object')return Object.fromEntries(Object.entries(x).filter(([k])=>!(k==='id' && isCue(x))).map(([k,v])=>[k,eraseIds(v)]));
 return x;
};
const globals={};
for(const [name,fn] of Object.entries(api)) {
 globals[name]=typeof fn==='function' ? (...args)=>{
   const result=fn(...args);
   // Record public inputs/outputs, preserving meaningful cue IDs. Allocated
   // IDs are compared separately from cue semantics by the Rust replay.
   cases.push({op:name,args:sanitize(args),expected:sanitize(result),withoutIds:eraseIds(result)});
   return result;
 }:fn;
}
const noop=()=>{};
const assertion=new Proxy(noop,{get:()=>assertion,apply:()=>assertion});
const it=(label,fn)=>fn();it.each=arr=>(label,fn)=>arr.forEach(x=>Array.isArray(x)?fn(...x):fn(x));
Object.assign(globals,{describe:(label,fn)=>fn(),it,expect:()=>assertion});
for(const file of ['model.test.ts','formats.test.ts'])runInNewContext('(()=>{'+clean(source(base+file))+'})()',globals);
// Execute pure CJK fixtures with their original pure layout/segmentation
// helpers; capture only the model functions this crate ports.
globals.MIN_CUE_DURATION=0.04;
for (const [file,name] of [['layout.ts','layoutCaption'],['segment.ts','segmentWords']]) {
 globals[name]=runInNewContext(clean(source(base+file))+`\n${name}`,{...api,MIN_CUE_DURATION:0.04});
}
runInNewContext('(()=>{'+clean(source(base+'cjk.test.ts'))+'})()',globals);
const timed=[{id:'a',start:1,end:2,text:'A'},{id:'b',start:3,end:5,text:'B',words:[{text:'B',start:3,end:5}]}];
for(const delta of [4,-1,0])globals.rebaseCuesForStart(timed,delta);
for(const end of [4,0,10])globals.clipCuesToEnd(timed,end);
globals.shiftAllCues(timed,0.333333);
// Every standalone subtitle fixture in the repository (none at v1.2.0), plus
// inline subtitle files across the caption tests/e2e specs, are found below.
const paths=execFileSync('git',['ls-tree','-r','--name-only','v1.2.0'],{cwd:root,encoding:'utf8'}).trim().split('\n');
const standalone=[];
for(const path of paths.filter(p=>/\.(srt|vtt)$/i.test(p))) {
 const text=source(path);standalone.push(path);const result=api.parseCaptions(text,'auto',path);
 cases.push({op:'parseCaptions',args:[text,'auto',path],expected:sanitize(result),withoutIds:eraseIds(result)});
}
// Extract string/array subtitle constants without executing browser tests.
const inline=[];
const inlineSources=execFileSync('git',['grep','-l','-E','[0-9]{2}:[0-9]{2}.*-->','v1.2.0','--','apps/desktop'],{cwd:root,encoding:'utf8'}).trim().split('\n').map(p=>p.replace(/^v1\.2\.0:/,''));
for(const path of inlineSources) {
 const code=source(path);
 const literals=[...code.matchAll(/'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|`(?:[^`\\]|\\.)*`/g)].map(m=>m[0]);
 for(const expr of literals) {
  if(expr.includes('${') || !/[0-9]{2}:[0-9]{2}.*-->/.test(expr))continue;
  const text=runInNewContext(expr);if(typeof text!=='string')continue;
  const result=api.parseCaptions(text);
  inline.push(path);
  cases.push({op:'parseCaptions',source:path,args:[text],withoutIds:eraseIds(result)});
 }
 const constants=[...code.matchAll(/\bconst\s+(?:SRT|VTT|SUBTITLES|srt|vtt)\s*=\s*((?:'(?:[^'\\]|\\.)*'|"(?:[^"\\]|\\.)*"|`[^`]*`)|\[[\s\S]*?\]\.join\([^;]*?\))\s*;/g)];
 for(const [_,expr] of constants) {
  if(expr.includes('${'))continue;
  const text=runInNewContext(expr);
  if(typeof text!=='string' || !text.includes('-->'))continue;
  inline.push(path);
  const result=api.parseCaptions(text);
  cases.push({op:'parseCaptions',source:path,args:[text],expected:sanitize(result),withoutIds:eraseIds(result)});
 }
}
for(const path of inlineSources)if(!inline.includes(path))throw new Error('Uncaptured inline caption fixture: '+path);
const style={default:api.DEFAULT_CAPTION_STYLE,presets:api.CAPTION_PRESETS};
function write(relative,value){const target=fileURLToPath(new URL(relative,import.meta.url));const contents=JSON.stringify(value,null,2)+'\n';if(process.argv.includes('--check')) {if(readFileSync(target,'utf8')!==contents)throw new Error(relative+' drift');}else writeFileSync(target,contents);}
write('../src/styles.json',style);
// Clock/random IDs are not goldens. Replace dynamically allocated IDs with
// stable labels everywhere, while retaining authored ids in input/output.
const shared={};const sharedKeys=new Map();
function share(value){
 if(Array.isArray(value)){
  if(value.length>100){const key=JSON.stringify(value);let id=sharedKeys.get(key);if(!id){id='array'+sharedKeys.size;sharedKeys.set(key,id);shared[id]=value;}return {$fixture:id};}
  return value.map(share);
 }
 return value;
}
const corpus={shared,tag:'v1.2.0',standalone,inline:[...new Set(inline)],cases:cases.filter(c=>c.op!=='cueId').map(c=>({...c,args:share(c.args),expected:undefined}))};
write('../tests/reference.json',corpus);
console.log(`${cases.length} reference operations, ${standalone.length} standalone and ${inline.length} inline subtitle strings from ${new Set(inline).size} files`);
