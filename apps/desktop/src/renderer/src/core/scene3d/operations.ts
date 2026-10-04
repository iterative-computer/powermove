import { createScene, createObject, createLight, parseScene, sceneProperties, SCENE3D_DEFINITION } from './schema';
import { LAYER3D_DEFINITIONS, layer3DRole, parseLayer3DData, TRANSFORM_PATHS } from './layers';
import type { EditCommand, EditMeta, EditResult } from '../types/commands';

export interface Scene3DEdit {
  operation: 'create'|'add_object'|'add_light'|'add_camera'|'update_object'|'update_light'|'remove'|'duplicate'|'set_camera'|'set_environment';
  target?: string; id?: string; name?: string; object?: any; light?: any; camera?:any; patch?: any; scene?: any;
}
const clone = <T>(v: T): T => JSON.parse(JSON.stringify(v));
const badKeys = new Set(['__proto__','prototype','constructor']);
function assertPlain(v: any, depth = 0): void {
  if (depth > 32) throw new Error('3D edit is too deeply nested');
  if (v === null || ['string','boolean'].includes(typeof v)) return;
  if (typeof v === 'number' && Number.isFinite(v)) return;
  if (!v || typeof v !== 'object' || (!Array.isArray(v) && Object.getPrototypeOf(v) !== Object.prototype)) throw new Error('3D edits require plain JSON');
  for (const [k,x] of Object.entries(v)) { if (badKeys.has(k)) throw new Error('Unsafe 3D field'); assertPlain(x,depth+1); }
}
const nativeProperties=(p:any={})=>Object.fromEntries(Object.entries<any>(p).filter(([key])=>TRANSFORM_PATHS[key]).map(([key,value])=>{
  const factor=key.startsWith('s')?100:1;
  return [TRANSFORM_PATHS[key],typeof value==='object'?{...value,v:value.v*factor,kf:(value.kf || []).map((k:any)=>({...k,v:k.v*factor}))}:value*factor];
}));

/** Models, lights and cameras are ordinary layers. Commands cross the normal guarded edit boundary. */
export function sceneCommands(PM: any, args: Scene3DEdit, meta: EditMeta = {}): {commands: EditCommand[];id?: string} {
  assertPlain(args);
  const keys = new Set(['operation','target','id','name','object','light','camera','patch','scene']);
  if (Object.keys(args).some(k=>!keys.has(k))) throw new Error('Unknown 3D edit field');
  if (JSON.stringify(args).length > 8_000_000) throw new Error('3D edit exceeds the geometry budget');
  const commands:any[]=[];
  const uid=(role:string)=>String(PM.uid?.(role) || `${role}_${crypto.randomUUID().replaceAll('-','')}`);
  const add=(role:'object'|'light'|'camera',node:any, suppliedId?:string)=>{
    const id=suppliedId || uid(role);
    if(PM.L?.(id))throw new Error(`Layer id already exists: ${id}`);
    let data:any,props:any,visible=true;
    if(role==='object'){
      const parsed=parseScene({objects:[{...node,id,parent:null}]}).objects[0]!;
      const {id:_,name,parent,p,...object}=parsed;data={object};props=nativeProperties(p);
      const visibility=p.visible!;
      if(visibility.kf.length || visibility.expr)throw new Error('Animated visibility is not supported; use layer opacity');
      visible=visibility.v;
    }else if(role==='light'){
      const parsed=parseScene({lights:[{...node,id}]}).lights[0]!;
      const {x,y,z,visible:visibility,...p}=parsed.p;data={light:{type:parsed.type,p,castShadow:parsed.castShadow}};props=nativeProperties({x,y,z});
      if(visibility!.kf.length || visibility!.expr)throw new Error('Animated visibility is not supported; use light intensity');
      visible=visibility!.v;
    }else{
      const parsed=parseScene({camera:node.camera || node,environment:node.environment});
      const {x,y,z,...p}=parsed.camera.p;data={camera:{projection:parsed.camera.projection,p},environment:parsed.environment};props=nativeProperties({x,y,z});
    }
    commands.push({type:'add_layer',id,layerType:'extension',name:node.name || args.name || (role==='object'?'3D Model':role==='light'?'Light':'Camera'),
      content:{definition:LAYER3D_DEFINITIONS[role],data:parseLayer3DData(role,data)},properties:props,
      visible,
      ...(role==='object'&&node.parent?{parent:node.parent}:{}),select:true});
    return id;
  };
  if(args.operation==='create'){
    if(!args.scene)return {commands,id:add('object',args.object || {source:{primitive:'box'}},args.object?.id || args.id)};
    const scene=parseScene(args.scene),mapping=new Map<string,string>();
    for(const node of scene.objects)mapping.set(node.id,uid('object'));
    let first:string|undefined;
    const pending=[...scene.objects],added=new Set<string>();
    while(pending.length){const index=pending.findIndex(n=>!n.parent || added.has(n.parent));const node=pending.splice(index,1)[0]!;
      const id=add('object',{...node,parent:node.parent?mapping.get(node.parent):null},mapping.get(node.id));added.add(node.id);first ||= id;}
    for(const node of scene.lights)add('light',node);
    const cameraId=add('camera',{camera:scene.camera,environment:scene.environment});first ||= cameraId;
    return {commands,id:first};
  }
  if(args.operation==='add_object')return {commands,id:add('object',args.object || {source:{primitive:'box'}},args.object?.id || args.id)};
  if(args.operation==='add_light')return {commands,id:add('light',args.light || {},args.light?.id || args.id)};
  if(args.operation==='add_camera')return {commands,id:add('camera',args.camera || args.patch || {},args.id)};
  const selected=args.id || args.target || PM.sel?.layers?.[0];
  let layer=PM.L?.(selected);
  if(!layer && typeof selected==='string'){
    const matches=(PM.curComp?.() || PM.proj).layers.filter((l:any)=>l.name===selected && layer3DRole(l));
    if(matches.length>1)throw new Error('Multiple 3D layers have that name; use a layer ID');
    layer=matches[0];
  }
  if(args.operation==='set_camera' || args.operation==='set_environment'){
    if(layer3DRole(layer)!=='camera')layer=(PM.curComp?.() || PM.proj).layers.find((l:any)=>layer3DRole(l)==='camera' && l.on!==false);
    if(!layer){const id=add('camera',args.operation==='set_camera'?{camera:args.patch || {}}:{camera:{},environment:args.patch || {}});return {commands,id};}
  }
  const role=layer3DRole(layer);
  if(!layer || !role)throw new Error('Choose a 3D model, light or camera layer');
  if(layer.lock || (PM.groupAncestors?.(layer) || []).some((g:any)=>g.lock))throw new Error('Unlock the 3D layer before editing it');
  const setChannel=(path:string,value:any)=>{
    const prop=PM.findProp?.(layer,path) || layer.p?.[path] || sceneProperties(layer).find(x=>x.key===path)?.prop;
    if(!prop)throw new Error(`Unknown 3D channel: ${path}`);
    if(typeof value==='object' && value!==null){
      if(JSON.stringify(value)===JSON.stringify(prop))return;
      commands.push({type:'set_property',target:layer.id,path,value:value.v,mode:'value'});
      commands.push({type:'replace_keyframes',target:layer.id,path,keyframes:(value.kf || []).map((k:any)=>({time:k.t,value:k.v,ease:k.ease || 'linear',hold:k.hold || false})),expression:value.expr || null});
    }else if(value!==undefined){
      commands.push({type:'set_property',target:layer.id,path,value,mode:PM.autokey?'keyframe':'auto'});
    }
  };
  const content=clone(layer.d.data),patch=clone(args.patch || {});
  if(patch.id && patch.id!==layer.id)throw new Error('3D IDs cannot be changed');
  delete patch.id;delete patch.name;delete patch.parent;
  const transformPatch=(p:any)=>{
    if(p && (typeof p!=='object' || Array.isArray(p)))throw new Error('3D properties must be an object');
    const contentFields=new Set(sceneProperties(layer).map(field=>field.key.split('.')[1]));
    for(const key of Object.keys(p || {}))if(!TRANSFORM_PATHS[key] && key!=='visible' && (args.operation==='update_object' || !contentFields.has(key)))throw new Error(`Unknown 3D property: ${key}`);
    const native=nativeProperties(p);
    for(const [path,value] of Object.entries(native))setChannel(path,value);
    if(p?.visible!==undefined){
      const raw=p.visible;
      if(typeof raw!=='boolean' && (!raw || typeof raw!=='object' || typeof raw.v!=='boolean' || raw.kf?.length || raw.expr))throw new Error('Visibility must be a boolean');
      commands.push({type:'set_layer',target:layer.id,patch:{visible:typeof raw==='object'?raw.v:raw}});
    }
  };
  switch(args.operation){
    case 'remove':
      if(meta.origin==='agent' && Object.keys(layer.locked_intent || {}).length)throw new Error('Preserved hand-edited 3D layer');
      commands.push({type:'delete_layers',targets:[layer.id]});break;
    case 'duplicate':{
      const id=uid(role),copy=clone(layer);
      for(const {prop} of [...Object.values<any>(copy.p || {}).map(prop=>({prop})),...sceneProperties(copy)])for(const key of prop.kf || [])key.i=uid('kf');
      commands.push({type:'add_layer',id,layerType:'extension',name:args.name || `${layer.name} copy`,content:{definition:layer.d.definition,data:copy.d.data},
        properties:copy.p,from:layer.from,duration:layer.dur,parent:layer.parent || undefined,blend:layer.blend,visible:layer.on,select:true});
      return {commands,id};
    }
    case 'update_object':{
      if(role!=='object')throw new Error('Choose a model layer');
      transformPatch(patch.p);delete patch.p;
      if(patch.material?.p){for(const [key,value] of Object.entries(patch.material.p))setChannel(`m.${key}`,value);delete patch.material.p;}
      const next={...content.object,...patch,material:patch.material?{...content.object.material,...patch.material}:content.object.material};
      delete next.name;delete next.parent;
      content.object=next;
      break;
    }
    case 'update_light':{
      if(role!=='light')throw new Error('Choose a light layer');
      const p=patch.p || {};transformPatch(p);
      for(const [key,value] of Object.entries(p))if(!TRANSFORM_PATHS[key] && key!=='visible')setChannel(`light.${key}`,value);
      delete patch.p;delete patch.name;
      content.light={...content.light,...patch};break;
    }
    case 'set_camera':{
      const p=patch.p || {};transformPatch(p);
      for(const [key,value] of Object.entries(p))if(!TRANSFORM_PATHS[key])setChannel(`camera.${key}`,value);
      delete patch.p;content.camera={...content.camera,...patch};break;
    }
    case 'set_environment':{
      for(const [key,value] of Object.entries(patch.p || {}))setChannel(`environment.${key}`,value);
      delete patch.p;content.environment={...content.environment,...patch};break;
    }
    default:throw new Error('Unknown 3D operation');
  }
  if(args.patch?.name!==undefined)commands.push({type:'set_layer',target:layer.id,patch:{name:args.patch.name}});
  if(args.patch?.parent!==undefined)commands.push({type:'set_layer',target:layer.id,patch:{parent:args.patch.parent}});
  const checked=parseLayer3DData(role,content);
  // Validate channel values against their schema before executing any part of a batch.
  const candidate=clone(layer);candidate.d.data=clone(checked);
  for(const command of commands){
    if(command.type==='set_property'){
      const prop=candidate.p?.[command.path] || sceneProperties(candidate).find(x=>x.key===command.path)?.prop;
      if(prop)prop.v=command.value;
    }
  }
  parseLayer3DData(role,candidate.d.data);
  if(JSON.stringify(checked)!==JSON.stringify(layer.d.data))commands.push({type:'set_content',target:layer.id,patch:{data:checked}});
  return {commands,id:layer.id};
}

export function editScene(PM: any,args: Scene3DEdit,meta: EditMeta = {}): EditResult {
  try {
    const plan = sceneCommands(PM,args,meta);
    const result = PM.Edit.apply(plan.commands,{label:'Edit 3D layers',...meta});
    if (result.ok) result.data.result = {id:plan.id || null,layerId:plan.id || args.target || PM.sel?.layers?.[0] || null};
    return result;
  } catch (e) { return {ok:false,message:e instanceof Error ? e.message : String(e)}; }
}

export function validateSceneLayer(layer: any,PM?:any): void {
  const role=layer3DRole(layer);
  if(role)layer.d.data=parseLayer3DData(role,layer.d.data);
  else if(layer.d?.definition===SCENE3D_DEFINITION)layer.d.data.scene=parseScene(layer.d.data.scene);
  else return;
  const ids=new Set<string>();
  for(const {prop} of sceneProperties(layer))for(const key of prop.kf){
    if(!key.i||ids.has(key.i))key.i=PM?.uid?.('kf') || crypto.randomUUID();
    ids.add(key.i);
  }
}
export function findSceneProperty(layer: any,path: string): any {
  return sceneProperties(layer).find(p=>p.key===path)?.prop || null;
}
