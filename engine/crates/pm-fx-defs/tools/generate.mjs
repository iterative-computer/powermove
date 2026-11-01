// Development oracle only. Runtime consumes the committed JSON in Rust.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import {runInNewContext} from 'node:vm';
import {writeFileSync, readFileSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
const tag = 'v1.2.0';
const root = execFileSync('git', ['rev-parse', '--show-toplevel'], {encoding:'utf8'}).trim();
if(execFileSync('git',['rev-parse',tag+'^{commit}'],{cwd:root,encoding:'utf8'}).trim()!=='07ed5fa17ee640659cb075f9cad1af2095011d60')throw new Error('v1.2.0 reference commit changed');
const blobs = {};
function source(path) {
  blobs[path] = execFileSync('git', ['rev-parse', `${tag}:${path}`], {cwd:root,encoding:'utf8'}).trim();
  return execFileSync('git', ['show', `${tag}:${path}`], {cwd:root,encoding:'utf8'});
}
function evaluate(code, name, globals={}) {
  const clean = stripTypeScriptTypes(code.replace(/^import[\s\S]*?;\s*/gm, '').replace(/^export /gm,''));
  return runInNewContext(clean+`; JSON.stringify(${name})`, globals);
}
const effects = JSON.parse(evaluate(source('apps/desktop/src/extensions/effects-basic/effects.ts'),'EFFECTS'));
const transitions = JSON.parse(evaluate(source('apps/desktop/src/extensions/transitions-basic/transitions.ts'),'BUILTIN_TRANSITIONS'));
const space = source('apps/desktop/src/renderer/src/legacy/core/space-3d.ts');
const channels3d = JSON.parse(evaluate(space.slice(space.indexOf('export const CHANNELS_3D'),space.indexOf('const identity')),'CHANNELS_3D'));
const model = source('apps/desktop/src/renderer/src/legacy/core/model.ts');
const prefix = model.slice(0, model.indexOf('/* ── project factory')) + '\n}';
const PM = {uid:()=> 'unused'};
evaluate(prefix+'\ninstall(PM);', '{}', {PM, CHANNELS_3D:channels3d, presetStyle:()=>({}),normalizeCaptionsContent:x=>x});
const layerDefaults = {};
for (const type of Object.keys(PM.TYPE_META)) {
  const layer = PM.mkLayer(type, {}, {w:0,h:0,dur:0});
  const channels = {};
  for (const [k,p] of Object.entries(layer.p)) channels[k] = {kind:'literal',value:p.v};
  for (const [k,p] of Object.entries(layer.d)) {
    if (['code','cues','style','data','params','uniforms'].includes(k)) continue;
    channels['c.'+k] = {kind:'literal',value:p && typeof p === 'object' && 'v' in p ? p.v : p};
  }
  // Use factory evaluations, not a hand-maintained set of contextual defaults.
  const sized = PM.mkLayer(type, {}, {w:1976,h:1124,dur:0});
  for (const k of Object.keys(channels)) {
    const s = k.startsWith('c.') ? sized.d[k.slice(2)] : sized.p[k]?.v;
    if (s === 1976) channels[k] = {kind:'width'};
    else if (s === 1124) channels[k] = {kind:'height'};
    else if (s === 988) channels[k] = {kind:'halfWidth'};
    else if (s === 562) channels[k] = {kind:'halfHeight'};
  }
  layerDefaults[type] = channels;
}
// Reset has a declared transform table distinct from factory defaults.
// Keep that contract explicit rather than silently inferring it from mkLayer.
const inspector = source('apps/desktop/src/extensions/inspector/ChannelRow.svelte');
const resetTable = inspector.match(/const defaults: Record<string, number> = (\{[\s\S]*?\n    \});/)[1];
const resetDefaults={};
for(const type of Object.keys(layerDefaults)) {
 const zeros=runInNewContext('('+resetTable+')',{layer:{type},comp:{w:0,h:0}});
 const sized=runInNewContext('('+resetTable+')',{layer:{type},comp:{w:1976,h:1124}});
 resetDefaults[type]=Object.fromEntries(Object.entries(zeros).filter(([k])=>k in layerDefaults[type]).map(([k,value])=>[k,sized[k]===988?{kind:'halfWidth'}:sized[k]===562?{kind:'halfHeight'}:{kind:'literal',value}]));
}
const result = JSON.stringify({tag, commit:execFileSync('git',['rev-parse',tag+'^{commit}'],{cwd:root,encoding:'utf8'}).trim(),blobs,effects,transitions,channels:PM.CH,layerDefaults,resetDefaults},null,2)+'\n';
const target = fileURLToPath(new URL('../src/builtins.json',import.meta.url));
if (process.argv.includes('--check')) {
  if (readFileSync(target,'utf8') !== result) throw new Error('builtins.json differs from pinned registry');
} else writeFileSync(target,result);
console.log(`${effects.length} effects, ${transitions.length} transitions, ${Object.keys(layerDefaults).length} layer schemas`);
