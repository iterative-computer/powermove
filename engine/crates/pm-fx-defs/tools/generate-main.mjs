// Development-only oracle. Runtime reads JSON in Rust; no JS in the renderer.
// Node >=24, invoked with --experimental-vm-modules.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import {SourceTextModule, createContext, runInContext} from 'node:vm';
import {readFileSync, writeFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
const commit = 'c7f46a05641b23944d3692c9d80f9677c7e1ef1d';
const root = execFileSync('git',['rev-parse','--show-toplevel'],{encoding:'utf8'}).trim();
const context = createContext({});
const modules = new Map(), blobs = {};
async function load(path) {
  if (modules.has(path)) return modules.get(path);
  blobs[path] = execFileSync('git',['rev-parse',`${commit}:${path}`],{cwd:root,encoding:'utf8'}).trim();
  const code = execFileSync('git',['show',`${commit}:${path}`],{cwd:root,encoding:'utf8'});
  const module = new SourceTextModule(stripTypeScriptTypes(code),{context,identifier:path});
  modules.set(path,module);
  await module.link((specifier,parent) => {
    if (!specifier.startsWith('.')) throw new Error(`Unexpected runtime dependency ${specifier}`);
    const target = new URL(specifier+'.ts','file:///'+parent.identifier).pathname.slice(1);
    if (!target.startsWith('apps/desktop/src/')) throw new Error('Source outside pinned desktop');
    return load(target);
  });
  return module;
}
const module = await load('apps/desktop/src/extensions/effects-basic/effects.ts');
await module.evaluate();
const effects = JSON.parse(JSON.stringify(module.namespace.EFFECTS));
const glsl = await namespace('apps/desktop/src/renderer/src/kernel/glsl.ts');
const shaderPath = 'apps/desktop/src/renderer/src/legacy/gl/shaders.ts';
blobs[shaderPath] = execFileSync('git',['rev-parse',`${commit}:${shaderPath}`],{cwd:root,encoding:'utf8'}).trim();
const shaderSource = execFileSync('git',['show',`${commit}:${shaderPath}`],{cwd:root,encoding:'utf8'});
// Read only the pure PRE constant; install() also connects a live kernel.
const match = shaderSource.match(/const PRE = (`[\s\S]*?`);/);
if (!match || match[1].includes('${')) throw new Error('Expected a literal GLSL preamble');
const preamble = runInContext(match[1],context);
const shaders = effects.flatMap(def=>[def,...(def.legacy??[])]).map(def=>({
  id:def.id,version:def.version??1,source:glsl.toLegacyFx(def,preamble).frag,
}));
const output = {commit,blobs:Object.fromEntries(Object.entries(blobs).sort()),effects,shaders};
const target = fileURLToPath(new URL('../src/main-effects.json',import.meta.url));
const result = JSON.stringify(output,null,2)+'\n';
if (process.argv.includes('--check')) {
  if (readFileSync(target,'utf8')!==result) throw new Error('main-effects.json differs from pinned main');
} else writeFileSync(target,result);
console.log(`${effects.length} current, ${effects.reduce((sum,e)=>sum+(e.legacy?.length??0),0)} historical definitions`);

// Values are executed by the immutable Electron source, not by Rust helpers.
async function namespace(path) { const m = await load(path); if (m.status !== 'evaluated') await m.evaluate(); return m.namespace; }
const sampling = await namespace('apps/desktop/src/renderer/src/legacy/gl/effect-sampling.ts');
const inputs = await namespace('apps/desktop/src/renderer/src/core/effect-inputs.ts');
const defaults = await namespace('apps/desktop/src/renderer/src/kernel/effect-defaults.ts');
const versions = await namespace('apps/desktop/src/renderer/src/kernel/effect-versions.ts');
const colors = await namespace('apps/desktop/src/extensions/effects-basic/color-math.ts');
const video = await namespace('apps/desktop/src/shared/video-color.ts');
const samples = await namespace('apps/desktop/src/renderer/src/core/render-samples.ts');
const fixtures = {commit, sampling:[], grids:[], mips:[], temporal:[], inputs:[], defaults:[], versions:[], profiles:[], transfers:[], video:[], motionBlur:[]};
const finite = [0,.01,1,24,64,400,10000,-400];
for (const radius of finite) for (const scale of [[1,1],[.25,.5],[-2,3]]) for (const [width,height] of [[1,1],[37,19],[1920,1080]]) {
 const args=[radius,1.5,32,scale,width,height]; fixtures.sampling.push({args,value:sampling.sampleLod(...args)});
}
for (const columns of [-100,0,1,1.49,1.5,2048,1e9]) for (const rows of [1,10.5,7000]) {
 const args=[columns,rows,2048];fixtures.grids.push({args,value:sampling.gridSize(...args)});
}
for (const depth of [8,16,32]) for (const [width,height] of [[1,1],[37,19],[1920,1080]]) for (const levels of [0,1,4,10]) {
 const args=[width,height,depth,levels]; fixtures.mips.push({args,value:sampling.mipBytes(...args)});
}
for (const count of [0,.49,.5,1,4.5,32,32.5,-1]) for (const step of [-3600,-1/30,0,1/60,3600,3601]) {
 const args=[count,step,3.25,32];try{fixtures.temporal.push({args,value:inputs.temporalTimes({temporal:{count:'count',step:'step',maxSamples:32}},key=>key==='count'?count:step,3.25)});}catch{fixtures.temporal.push({args,error:true});}
}
const referenceCases=[{}, {map:{layerId:null,stage:'source'}},{map:{layerId:'hidden',stage:'effects',space:'source'}},{path:{kind:'path',layerId:'L',pathId:'M'}},{lut:{kind:'asset',assetId:null}},
 {map:{stage:'source'}},{map:{layerId:'',stage:'source'}},{map:{layerId:'\ufeff',stage:'source'}},{map:{layerId:'\u0085',stage:'source'}},{map:{layerId:'x'.repeat(257),stage:'masks'}},{map:{layerId:'😀'.repeat(128),stage:'masks'}},{map:{layerId:'😀'.repeat(129),stage:'masks'}},
 {map:{layerId:'L',stage:'bad'}},{map:{layerId:'L',stage:'source',space:null}},{map:{layerId:'L',stage:'source',extra:true}}, {path:{kind:'path',layerId:null,pathId:'M'}},{path:{kind:'path',layerId:'L',pathId:null}},{path:{kind:'path',layerId:null,pathId:null}},{lut:{kind:'asset'}},{lut:{kind:'asset',assetId:1}},{constructor:{layerId:null,stage:'source'}},{Bad:{layerId:null,stage:'source'}},
 Object.fromEntries('abcde'.split('').map(k=>[k,{layerId:null,stage:'source'}])),[],null];
for(const args of referenceCases){try{fixtures.inputs.push({args,value:inputs.validateEffectInputs(args)});}catch{fixtures.inputs.push({args,error:true});}}
for(const def of effects)for(const param of def.params) for(const size of [{width:1,height:1},{width:4096,height:2160},{width:0,height:-1}]) fixtures.defaults.push({id:def.id,key:param.k,size,value:defaults.effectParameterDefault(param,size)});
for(const args of [null,0,1,1.0,2,1.5,-1,'1',false,{},[],Number.MAX_SAFE_INTEGER,Number.MAX_SAFE_INTEGER+1])fixtures.versions.push({args,value:versions.effectVersion(args)??null});
for(let input=0;input<3;input++)for(let output=0;output<3;output++)for(const linearInput of [false,true])for(const linearOutput of [false,true])for(const rgb of [[0,0,0],[1,1,1],[-.25,2,8],[.01,.04,.5],[1,0,0],[0,1,0]]) {
 const args=[rgb,input,output,linearInput,linearOutput];fixtures.profiles.push({args,value:colors.convertProfile(...args)});
}
for(const x of [-8,-1,-.04045,-.0031308,0,.0031308,.04045,.5,1,8])for(const profile of [0,1,2])fixtures.transfers.push({args:[x,profile],decode:colors.decodeTransfer(x,profile),encode:colors.encodeTransfer(x,profile)});
for (const args of [null,{}, {transfer:'smpte2084'}, {transfer:'arib-std-b67'}, {transfer:'bt709',primaries:'bt2020'}, {matrix:'bt2020c'}, {matrix:'bt2020nc'}, {primaries:'bt2020',transfer:'smpte2084'}, {primaries:'smpte432'}, {transfer:'log100'}]) fixtures.video.push({args,value:video.videoColorTreatment(args)});
for(const time of [0,1.234,-2])for(const fps of [0,24,29.97,60])for(const options of [{},{mbSamples:1,shutter:0},{mbSamples:4,shutter:-.5},{mbSamples:100,shutter:.25}])fixtures.motionBlur.push({args:[time,fps,options],value:samples.motionBlurTimes(time,fps,options)});
fixtures.blobs=Object.fromEntries(Object.entries(blobs).sort());
const fixtureTarget=fileURLToPath(new URL('../tests/main-values.json',import.meta.url));
const fixtureText='{\n'+Object.entries(fixtures).map(([key,value])=>
 Array.isArray(value) ? `  "${key}": [\n${value.map(row=>'    '+JSON.stringify(row)).join(',\n')}\n  ]` : `  "${key}": ${JSON.stringify(value)}`
).join(',\n')+'\n}\n';
if(process.argv.includes('--check')){if(readFileSync(fixtureTarget,'utf8')!==fixtureText)throw new Error('main-values.json differs from source oracle');}else writeFileSync(fixtureTarget,fixtureText);
console.log(Object.entries(fixtures).filter(([,v])=>Array.isArray(v)).map(([k,v])=>`${k}: ${v.length}`).join(', '));
