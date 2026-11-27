// Execute tagged bodies with explicit synthetic host seams. No native expected
// output, installed editor, real project, media file or profile is consulted.
import { tag, evaluate } from './main-oracle.mjs';
import fs from 'node:fs';
const input=JSON.parse(fs.readFileSync(0,'utf8'));
const project=structuredClone(input.project);
let serial=0;
const PM={proj:project,sel:{layers:[],keys:[],chan:null},time:input.time??0,CH:{},uid:prefix=>`${prefix}-oracle-${serial++}`,
  assets:{get:id=>input.runtime?.[id]?.available?{...input.runtime[id],dur:input.runtime[id].duration,hasAudio:input.runtime[id].hasAudio}:undefined,
    loading:new Set(Object.keys(input.runtime??{}).filter(id=>input.runtime[id].loading)),
    errors:new Map(Object.entries(input.runtime??{}).filter(([,v])=>v.error).map(([id,v])=>[id,v.error])),
    cloud:new Map(Object.entries(input.runtime??{}).filter(([,v])=>v.cloud).map(([id])=>[id,{}]))},
  round:(n,p)=>Math.round(n*10**p)/10**p,clamp:(n,a,b)=>Math.max(a,Math.min(b,n)),bus:{emit(){}},invalidate(){},autosave(){},
};
const {isProperty,canAnimateContent,contentLabel,resolveContent}=evaluate(tag('apps/desktop/src/renderer/src/legacy/core/content-properties.ts'),['isProperty','canAnimateContent','contentLabel','resolveContent']);
PM.evP=(_l,p)=>p.v;
const anim=tag('apps/desktop/src/renderer/src/legacy/core/anim.ts');
evaluate(anim.slice(anim.indexOf('PM.allProps ='),anim.indexOf('PM.findProp =')),[],{PM,isProperty,canAnimateContent,contentLabel,structuredProperties:()=>[],sceneProperties:()=>[]});
const folderCode=tag('apps/desktop/src/renderer/src/legacy/core/asset-folders.ts');
const folder=evaluate(folderCode,['createFolder','deleteFolder','folderList','isWithin','moveItems','renameFolder','validFolder']);
const mediaCode=tag('apps/desktop/src/renderer/src/legacy/assistant/media-management.ts');
let history=0;
PM.hist={mark:()=>history,do:(_label,fn)=>{const before=JSON.stringify(project);const result=fn();if(before!==JSON.stringify(project)){history++;project.revision=(project.revision??0)+1;}return result;}};
PM.MediaImport={removeAsset:(p,id)=>{const removed=[];const walk=c=>{const doomed=new Set((c.layers??[]).filter(l=>l.d?.asset===id).map(l=>l.id));removed.push(...doomed);c.layers=(c.layers??[]).filter(l=>!doomed.has(l.id));for(const l of c.layers)if(doomed.has(l.parent))l.parent=null;for(const nested of Object.values(c.comps??{}))walk(nested);};walk(p);delete p.assets[id];return {removedLayerIds:removed};}};
const media=evaluate(mediaCode,['listMedia','manageMedia'],{...folder});
let result;
if(input.tool==='list_media') result=media.listMedia(PM,input.arguments??{});
else if(input.tool==='manage_media') result=media.manageMedia(PM,input.arguments??{});
else if(input.tool==='edit_video') {
  const model=tag('apps/desktop/src/renderer/src/legacy/core/model.ts');
  const space=tag('apps/desktop/src/renderer/src/legacy/core/space-3d.ts');
  const {CHANNELS_3D}=evaluate(space.match(/export const CHANNELS_3D = .*;/)[0],['CHANNELS_3D']);
  evaluate('const uid=PM.uid;\n'+model.slice(model.indexOf('const P ='),model.indexOf('/* ── project factory')),[],{PM,CHANNELS_3D});
  evaluate(model.slice(model.indexOf('PM.cloneLayer ='),model.indexOf('/* ── serialization')),[],{PM,uid:PM.uid});
  PM.groupAncestors=l=>{const out=[];for(let g=l?.group;g;){const parent=project.layers.find(l=>l.id===g);if(!parent||out.includes(parent))break;out.push(parent);g=parent.group;}return out;};
  PM.removeLayers=ids=>{project.layers=project.layers.filter(l=>!ids.includes(l.id));for(const l of project.layers)if(ids.includes(l.parent))l.parent=null;};
  PM.Edit={mutate:(label,fn)=>{const before=JSON.stringify(project);const result=fn();return {result,message:before===JSON.stringify(project)?'No source changes':label};}};
  const {editVideo}=evaluate(tag('apps/desktop/src/renderer/src/legacy/assistant/video-editing.ts'),['editVideo'],{isProperty,resolveContent:(_PM,l)=>Object.fromEntries(Object.entries(l.d??{}).map(([k,v])=>[k,isProperty(v)?v.v:v]))});
  const edit=editVideo(PM,input.arguments,{label:input.arguments.label?.trim()||'Agent composition edit'});
  result={applied:[],clip:edit.result,results:[],message:edit.message};
} else if(input.tool==='check_project') {
  PM.evP=(layer,prop,time)=>{if(!isProperty(prop))return prop;const keys=prop.kf??[],t=time-layer.from;if(!keys.length)return prop.v;if(t<=keys[0].t)return keys[0].v;for(let i=1;i<keys.length;i++){if(t<=keys[i].t){const x=keys[i-1],y=keys[i];return x.hold?x.v:x.v+(y.v-x.v)*(t-x.t)/(y.t-x.t);}}return keys.at(-1).v;};
  PM.ev=(l,key,t)=>PM.evP(l,l.p?.[key],t);
  PM.animVersion=()=>0;
  const resolveContent=(_PM,l,t)=>Object.fromEntries(Object.entries(l.d??{}).map(([k,v])=>[k,isProperty(v)?PM.evP(l,v,t):v]));
  const {sourceTime}=evaluate(tag('apps/desktop/src/renderer/src/legacy/core/retiming.ts'),['sourceTime'],{isProperty,resolveContent,evaluatedValue:(_PM,l,p,t)=>PM.evP(l,p,t)});
  const {layerTiming}=evaluate(tag('apps/desktop/src/renderer/src/legacy/assistant/media-source.ts'),['layerTiming'],{isProperty,resolveContent,sourceTime});
  PM.groupAncestors=l=>{const out=[];for(let at=l.group;at;){const g=project.layers.find(l=>l.id===at);if(!g||out.includes(g))break;out.push(g);at=g.group;}return out;};
  PM.active=(l,t)=>t>=l.from&&t<l.from+l.dur&&PM.evP(l,l.on,t)!==false&&PM.groupAncestors(l).every(g=>t>=g.from&&t<g.from+g.dur&&PM.evP(g,g.on,t)!==false);
  PM.worldOpacity=(l,t)=>PM.ev(l,'opacity',t)/100*PM.groupAncestors(l).reduce((v,g)=>v*PM.ev(g,'opacity',t)/100,1);
  PM.assets.map=new Map();
  if(!input.runtime)delete PM.assets;
  const m=tag('apps/desktop/src/renderer/src/legacy/core/model.ts');
  evaluate(m.slice(m.indexOf('PM.normalizeFill ='),m.indexOf('PM.mkProject =')),[],{PM,normalizeColorDepth:v=>v,normalizeExportDefaults:v=>v});
  const {checkProject}=evaluate(tag('apps/desktop/src/renderer/src/legacy/assistant/project-check.ts'),['checkProject'],{isProperty,layerTiming,Map});
  result=checkProject(PM);
} else if(input.tool==='export_captions') {
  const model=tag('apps/desktop/src/renderer/src/captions/model.ts');
  const types=evaluate(tag('apps/desktop/src/renderer/src/core/types/captions.ts'),['CAPTION_ALIGNS','CAPTION_PLACEMENTS','CAPTION_TEXT_CASES','CAPTION_WORD_ANIMATIONS']);
  const bindings=evaluate(model.replace(/^import[\s\S]*?;\n/gm,'').replace(/^export type \{.*\} from .*;\n/gm,''),['normalizeCaptionsContent','normalizeCues','MIN_CUE_DURATION'],types);
  const formats=evaluate(tag('apps/desktop/src/renderer/src/captions/formats.ts'),['formatCaptions'],bindings);
  const cmd=tag('apps/desktop/src/renderer/src/captions/commands.ts');
  const {exportCaptionsText}=evaluate(cmd.slice(cmd.indexOf('export function exportCaptionsText')),['exportCaptionsText'],{...bindings,...formats});
  const layer=project.layers.find(l=>l.id===input.arguments.layerId);const format=input.arguments.format??'srt';
  result={layerId:layer.id,format,cues:layer.d.cues.length,text:exportCaptionsText(layer,format)};
} else if(input.tool==='render_frames') {
  const h=tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
  PM.selLayers=()=>project.layers.filter(l=>(input.selection??[]).includes(l.id));
  PM.Export={snapshot:(time,width)=>`data:image/png;base64,${input.image}`};
  const {capture}=evaluate(h.slice(h.indexOf('function safeTimes'),h.indexOf('function propertyDigest'))+h.slice(h.indexOf('async function capture('),h.indexOf('async function observe(')),['capture'],{PM,captureAborted(){}});
  const frames=await capture(input.arguments?.times,input.arguments?.width??720);
  result={times:frames.times,width:input.arguments?.width??720,rendered:frames.images.length};
} else throw Error('Unsupported domain oracle '+input.tool);
process.stdout.write(JSON.stringify({result,project}));
