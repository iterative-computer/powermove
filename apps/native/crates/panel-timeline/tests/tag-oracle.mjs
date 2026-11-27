// Test-only Node. Execute immutable tag sources; expected results never use Rust.
import {execFileSync} from 'node:child_process';
import {stripTypeScriptTypes} from 'node:module';
import {writeFileSync} from 'node:fs';
const tag='v1.2.0',base='apps/desktop/src/extensions/timeline/';
async function load(name) {
  const source=execFileSync('git',['show',`${tag}:${base}${name}.ts`],{encoding:'utf8'});
  return import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(source)).toString('base64')}`);
}
const {layoutTracks,planTrackMove}=await load('track-layout');
const {createDirectionalSnapper}=await load('directional-snap');
const timeline=execFileSync('git',['show',`${tag}:${base}timeline.ts`],{encoding:'utf8'});
const helpers=timeline.slice(timeline.indexOf('export function resolveKeyframeGroupSnap('),timeline.indexOf('export interface EdgeSnapProbe'));
const {resolveKeyframeGroupSnap,playheadSnapTargets}=await import(`data:text/javascript;base64,${Buffer.from(stripTypeScriptTypes(helpers)).toString('base64')}`);
let seed=20261101;const random=()=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed/2**32;};
const tracks=[];
for(let n=0;n<40;n++) {
  const layers=Array.from({length:16},(_,i)=>({id:`L${i}`,type:i%9===0?'audio':i%11===0?'captions':'shape',from:Math.floor(random()*30)/3-2,dur:Math.floor(random()*12+1)/3,track:Math.floor(random()*3)}));
  if(n%2===0) {layers[0].type='group';for(let i=1;i<6;i++)layers[i].group='L0';}
  if(n%4===0) {layers[1].type='group';layers[2].group='L1';layers[3].group='L1';}
  const collapsed=n%3===0?['L0']:[];
  const result=layoutTracks(layers,{duration:20,collapsed:id=>collapsed.includes(id)});
  const move={ids:['L0','L8'],dt:.125,lanes:{video:n%3-1,audio:n%2,captions:1}};
  const moved=planTrackMove(layers,{duration:20,collapsed:id=>collapsed.includes(id)},move);
  tracks.push({layers,collapsed,move,moved:moved.layers.map(({id,from,track})=>({id,from,track})),expected:{video:result.video,audio:result.audio,captions:result.captions,items:result.items,bands:result.bands}});
}
const snapping=[];
for(let n=0;n<24;n++) {
  const anchors=Array.from({length:n%5+1},(_,i)=>({time:i/3,edge:i%2?'out':'in'}));
  const targets=Array.from({length:20},(_,i)=>({time:i/4,weight:i%4===0?1.5:1,...(i%3===0?{edge:'out'}:{})}));
  const snap=createDirectionalSnapper();const steps=[];let raw=0;
  for(let i=0;i<40;i++) {raw+=random()*0.8-0.3;const enabled=i%9!==0;const result=snap.resolve(anchors,raw,targets,.15,enabled);steps.push({raw,enabled,delta:result.delta,lock:result.lock?{...result.lock}:null});}
  snapping.push({anchors,targets,steps});
}
const groupSnapping=[];
for(let n=0;n<20;n++) {
  const anchors=[.125,1.25,3.5],targets=[0,.5,2,3,4.125];let lock=null;const steps=[];
  for(let i=0;i<30;i++) {const raw=random()*5-1;const result=resolveKeyframeGroupSnap(anchors,raw,targets,.1,lock,.15);lock=result.lock;steps.push({raw,...result});}
  groupSnapping.push({anchors,targets,steps});
}
const playhead=[];
for(let n=0;n<20;n++) {const input={fps:n%2?30:24,dur:10,work:[1,8],markers:[{t:2.01}],layers:[{from:-.125,dur:2.3,keys:[.125,.3]},{from:4.001,dur:1.53,keys:[.125,1.5]}]};playhead.push({input,expected:playheadSnapTargets(input)});}
const revision=execFileSync('git',['rev-parse',`${tag}^{commit}`],{encoding:'utf8'}).trim();
const output={tag,revision,source:base,tracks,snapping,groupSnapping,playhead};
writeFileSync(new URL('./fixtures/tag-oracle.json',import.meta.url),JSON.stringify(output,null,0)+'\n');
console.log(`Tagged oracle: ${tracks.length} track cases, ${snapping.reduce((n,c)=>n+c.steps.length,0)} snap samples, 600 key-snap samples, 20 playhead cases; ${revision}`);
