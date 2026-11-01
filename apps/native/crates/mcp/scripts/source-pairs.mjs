// Pure tagged bodies + synthetic PM services. This does not claim a live
// Electron editor trace. Native outputs are never used as expected outputs.
import { tag, evaluate } from './tag-oracle.mjs';
import fs from 'node:fs';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const PM={proj:input.project,sel:input.selection??{layers:[],keys:[],chan:null},time:input.time??0,CH:{},round:(n,p)=>Math.round(n*10**p)/10**p};
const isProperty=v=>v&&typeof v==='object'&&Object.hasOwn(v,'v')&&Array.isArray(v.kf);
PM.evP=(layer,prop,time)=>{
  // Fixture cases use static properties, or linear animation only.
  const local=time-layer.from;const keys=prop.kf;
  if(!keys?.length)return prop.v;
  if(local<=keys[0].t)return keys[0].v;
  for(let i=1;i<keys.length;i++)if(local<=keys[i].t){const a=keys[i-1],b=keys[i];return a.hold?a.v:a.v+(b.v-a.v)*(local-a.t)/(b.t-a.t);}
  return keys.at(-1).v;
};
const anim=tag('apps/desktop/src/renderer/src/legacy/core/anim.ts');
evaluate(anim.slice(anim.indexOf('PM.allProps ='),anim.indexOf('PM.findProp =')),[],{PM,isProperty,canAnimateContent:()=>true,contentLabel:k=>k,structuredProperties:()=>[],sceneProperties:()=>[]});
const h=tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
const constants=h.slice(h.indexOf('const SCENE_OPERATIONS'),h.indexOf('function safeTimes'));
const bodies=h.slice(h.indexOf('function propertyDigest'),h.indexOf('async function capture('));
const bindings={PM,videoAssets:()=>[],Object,Set};
const {projectState}=evaluate(constants+bodies,['projectState'],bindings);
const timeline=tag('apps/desktop/src/renderer/src/legacy/assistant/layer-strips.ts');
const {timelineState}=evaluate(timeline.slice(timeline.indexOf('export function timelineState'),timeline.indexOf('/** Batch gestures')),['timelineState'],{PM,isProperty,resolveContent:(_PM,l)=>Object.fromEntries(Object.entries(l.d??{}).map(([k,v])=>[k,isProperty(v)?PM.evP(l,v,l.from):v])),finite:(n,k,min,max)=>{if(typeof n!=='number'||n<min||n>max)throw Error('Invalid '+k);return n;}});
const result=input.tool==='get_timeline'?timelineState(PM,input.arguments??{}):projectState(input.arguments??{});
process.stdout.write(JSON.stringify(result));
