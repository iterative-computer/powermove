// Regenerate independent tagged 3D read fixtures. Test-only dependency: zod
// 4.6.5 (the tag's version) in the npm prefix PM_ORACLE_MODULE_ROOT. Never run an app.
import {evaluate,root} from './tag-oracle.mjs';
import {execFileSync} from 'node:child_process';
const tag=file=>execFileSync('git',['show',`b81b94c66:${file}`],{cwd:root,encoding:'utf8'});
import {createRequire} from 'node:module';
import path from 'node:path';
import fs from 'node:fs';
const zodRoot=process.env.PM_ORACLE_MODULE_ROOT;
if(!zodRoot) throw Error('Set PM_ORACLE_MODULE_ROOT to a disposable npm prefix containing the tagged Zod 4.6.5');
const require=createRequire(path.join(zodRoot,'package.json'));
const {z}=require('zod');
if(require('zod/package.json').version!=='4.6.5')throw Error('Tagged zod 4.6.5 required');
let layers;
const schema=evaluate(tag('apps/desktop/src/renderer/src/core/scene3d/schema.ts'),['parseScene','validateSceneChannel','sceneProperties','objectSchema','lightSchema','cameraSchema','environmentSchema'],{z,layer3DProperties:(...a)=>layers.layer3DProperties(...a)});
layers=evaluate(tag('apps/desktop/src/renderer/src/core/scene3d/layers.ts'),['layer3DRole','layer3DProperties','compositionScene','initializeLayer3D'],schema);
const project={id:'synthetic-scene',name:'Synthetic scene',w:640,h:360,fps:30,dur:12,revision:0,layers:[],assets:{model:{id:'model',name:'Synthetic model',kind:'model',format:'glb'},image:{id:'image',name:'Synthetic texture',kind:'image'}}};
let serial=0;
const PM={proj:project,time:0,sel:{layers:[],keys:[],chan:null},CH:{},uid:prefix=>`${prefix}_synthetic_${serial++}`,evP:(_l,p)=>p.v,active:l=>l.on!==false,layerDefinition:()=>null};
const model=tag('apps/desktop/src/renderer/src/legacy/core/model.ts');
const space=tag('apps/desktop/src/renderer/src/legacy/core/space-3d.ts');
const {CHANNELS_3D}=evaluate(space.match(/export const CHANNELS_3D = .*;/)[0],['CHANNELS_3D']);
evaluate('const uid=PM.uid;\n'+model.slice(model.indexOf('const P ='),model.indexOf('/* ── project factory')),[],{PM,CHANNELS_3D});
const anim=tag('apps/desktop/src/renderer/src/legacy/core/anim.ts');
const isProperty=p=>p&&typeof p==='object'&&Array.isArray(p.kf);
evaluate(anim.slice(anim.indexOf('PM.allProps ='),anim.indexOf('PM.findProp =')),[],{PM,isProperty,canAnimateContent:()=>true,contentLabel:k=>k,structuredProperties:()=>[],sceneProperties:schema.sceneProperties});
const h=tag('apps/desktop/src/renderer/src/legacy/assistant/harness.ts');
function canonical(value){if(Array.isArray(value))return value.map(canonical);if(value&&typeof value==='object')return Object.fromEntries(Object.keys(value).sort().map(k=>[k,canonical(value[k])]));return value;}
function digest(args={}) {for(const key of Object.keys(project))project[key]=canonical(project[key]);const request={tool:'get_3d_scene',arguments:args};const code=h.slice(h.indexOf("  if (request.tool === 'get_3d_scene')"),h.indexOf("  if (request.tool === 'get_project_state')"));const {run}=evaluate('function run(){'+code+'}', ['run'],{PM,request,...layers,currentRevision:()=>project.revision,refreshLiveTransaction(){},toolText:value=>({type:'text',text:JSON.stringify(value)})});return JSON.parse(run().content[0].text);}
const fixtures=[{project:structuredClone(project),arguments:{},result:digest()}];
const omit=(object,keys)=>Object.fromEntries(Object.entries(object).filter(([k])=>!keys.includes(k)));
const scene=schema.parseScene({objects:[{id:'object',source:{primitive:'box'}}],lights:[{id:'light',type:'point'}]});
for(const [role,data] of [['object',{object:omit(scene.objects[0],['id','name','parent','p'])}],['light',{light:{...omit(scene.lights[0],['id','name']),p:omit(scene.lights[0].p,['x','y','z','visible'])}}],['camera',{camera:{...scene.camera,p:omit(scene.camera.p,['x','y','z'])},environment:scene.environment}]]){const layer=PM.mkLayer('extension',{name:`Synthetic ${role}`,d:{definition:`powermove.3d.${role}`,data}});layers.initializeLayer3D(layer);project.layers.push(layer);}
fixtures.push({project:structuredClone(project),arguments:{target:project.layers[0].id},result:digest({target:project.layers[0].id})});
const output={evidence:'3D source b81b94c66; independent TypeScript; zod 4.6.5; synthetic static 3D layers; no live app or native output',fixtures};
fs.writeFileSync(new URL('../fixtures/scene3d-source-read.json',import.meta.url),JSON.stringify(output,null,2)+'\n');
