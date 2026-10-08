import {animatedPart} from './recipe-preview';
import { generatedModelSchema } from '../../../../shared/blender';
import { objectSchema, lightSchema, cameraSchema, environmentSchema, parseScene, validateSceneChannel, type Scene3D } from './schema';
import { compCenter, defaultCameraChannels, defaultLightChannels } from './space';

export const LAYER3D_DEFINITIONS = {
  object: 'powermove.3d.object', light: 'powermove.3d.light', camera: 'powermove.3d.camera'
} as const;
export type Layer3DRole = keyof typeof LAYER3D_DEFINITIONS;
export function layer3DRole(layer: any): Layer3DRole|null {
  return (Object.keys(LAYER3D_DEFINITIONS) as Layer3DRole[]).find(role => layer?.d?.definition === LAYER3D_DEFINITIONS[role]) ?? null;
}
/** Camera layers sit at the top of the stack, outside groups, above everything they film. */
export function pinCameraLayers(layers:any[]):any[] {
  const cameras=layers.filter(layer=>layer3DRole(layer)==='camera');
  if(!cameras.length)return layers;
  for(const camera of cameras)camera.group=null;
  return [...cameras,...layers.filter(layer=>layer3DRole(layer)!=='camera')];
}
/** Model groups use the same world coordinates as their contents. Mixed artwork
 * groups retain the composition's existing 2D interaction. */
export function isModelGroup(PM:any,layer:any):boolean {
  if(layer?.type!=='group')return false;
  const members=(PM.curComp?.() || PM.proj)?.layers?.filter((child:any)=>child.type!=='group' && child.type!=='audio'
    && (PM.groupAncestors?.(child) || []).some((group:any)=>group.id===layer.id)) || [];
  return members.length>0 && members.every((child:any)=>!!layer3DRole(child));
}
export const TRANSFORM_PATHS: Record<string,string> = {x:'position.x',y:'position.y',z:'position.z',rx:'rotation.x',ry:'rotation.y',rz:'rotation',sx:'scale.x',sy:'scale.y',sz:'scale.z'};
const clone = (v:any) => JSON.parse(JSON.stringify(v));

/** Persist only content on a model. Identity, hierarchy and transforms belong to the real layer. */
export function parseLayer3DData(role: Layer3DRole, raw: any): any {
  const allowed = role === 'object' ? ['object'] : role === 'light' ? ['light'] : ['camera','environment'];
  if (!raw || typeof raw !== 'object' || Object.keys(raw).some(key=>!allowed.includes(key))) throw new Error('Unknown 3D layer content');
  if (role === 'object') {
    const input=objectSchema.omit({id:true,name:true,parent:true,p:true}).strict().parse(raw.object);
    const node=parseScene({objects:[{...input,id:'model'}]}).objects[0]!;
    const {id,name,parent,p,...object}=node;
    return {object};
  }
  if (role === 'light') {
    const input=lightSchema.omit({id:true,name:true}).strict().parse(raw.light);
    if(Object.keys(raw.light?.p || {}).some(key=>['x','y','z','visible'].includes(key))) throw new Error('Use the light layer transform for position and visibility');
    const node=parseScene({lights:[{...input,id:'light'}]}).lights[0]!;
    const {x,y,z,visible,...p}=node.p;
    return {light:{type:node.type,p,castShadow:node.castShadow}};
  }
  if(Object.keys(raw.camera?.p || {}).some(key=>['x','y','z'].includes(key))) throw new Error('Use the camera layer transform for position');
  const node=parseScene({camera:cameraSchema.parse(raw.camera ?? {}),environment:environmentSchema.parse(raw.environment ?? {})});
  const {x,y,z,...p}=node.camera.p;
  return {camera:{projection:node.camera.projection,p},environment:node.environment};
}
export function layer3DProperties(layer: any): any[] {
  const role=layer3DRole(layer),data=layer?.d?.data;
  if(!role && !layer?.d?.modeling)return [];
  const out:any[]=[];
  const collect=(p:any,prefix:string,group:string)=>{
    for(const [key,prop] of Object.entries<any>(p || {})) if(prop && Array.isArray(prop.kf))
      out.push({key:`${prefix}.${key}`,prop,label:key,group});
  };
  if(layer?.d?.modeling)collect(layer.d.modeling.p,'model','Model');
  if(role==='object'){
    collect(data.object?.material?.p,'m','Material');
    collect(data.object?.material?.shader?.p,'shader','Shader');
    for(const slot of data.object?.slots || []){collect(slot.material.p,`slots.${slot.id}`,slot.name);collect(slot.material.shader?.p,`slots.${slot.id}.shader`,slot.name);}
  }
  if(role==='light')collect(data.light?.p,'light','Light');
  if(role==='camera'){collect(data.camera?.p,'camera','Camera');collect(data.environment?.p,'environment','Environment');}
  return out;
}
/** Validate before mutating live channels, including the camera's coupled clip planes. */
export function validateLayer3DChannel(layer:any,path:string,property:any):void {
  if(path.startsWith('shader.')||path.startsWith('slots.')||path.startsWith('model.')){
    const candidate=clone(layer);const field=layer3DProperties(candidate).find(f=>f.key===path);if(!field)throw new Error(`Unknown 3D channel: ${path}`);
    Object.assign(field.prop,property);
    if(path.startsWith('model.'))generatedModelSchema.parse(candidate.d.modeling);else parseLayer3DData('object',candidate.d.data);return;
  }
  const alias=Object.keys(TRANSFORM_PATHS).find(key=>TRANSFORM_PATHS[key]===path);
  if(alias){
    const factor=alias.startsWith('s')?.01:1;
    objectSchema.shape.p.unwrap().shape[alias]!.parse({...property,v:property.v*factor,kf:property.kf.map((key:any)=>({...key,v:key.v*factor}))});
  }else validateSceneChannel(path,property);
  if(layer3DRole(layer)==='camera' && (path==='camera.near' || path==='camera.far')){
    const data=clone(layer.d.data);data.camera.p[path.slice(7)]=clone(property);parseLayer3DData('camera',data);
  }
}
/** Fill a new 3D layer's transform; unspecified positions use the composition's pixel defaults. */
export function initializeLayer3D(layer:any, properties:any={}, comp:any={w:1920,h:1080}): void {
  const role=layer3DRole(layer);if(!role)return;
  layer.d.data=parseLayer3DData(role,layer.d.data);
  layer.threeD=true;
  for(const [path,raw] of Object.entries<any>(properties))if(layer.p[path])layer.p[path]=typeof raw==='object'?clone(raw):{v:raw,kf:[],expr:null};
  const camera=defaultCameraChannels(comp),light=defaultLightChannels(comp),center=compCenter(comp);
  const position=role==='light'?[light.x,light.y,light.z]:role==='camera'?[camera.x,camera.y,camera.z]:[center.x,center.y,center.z];
  for(const [key,path] of Object.entries(TRANSFORM_PATHS)){
    const fallback=key.startsWith('s')?100:key==='x'?position[0]:key==='y'?position[1]:key==='z'?position[2]:0;
    const raw=properties[path] ?? fallback;
    const prop=typeof raw==='object'?clone(raw):{v:raw,kf:[],expr:null};
    if(!Number.isFinite(prop.v) || !Array.isArray(prop.kf))throw new Error(`Invalid 3D transform ${path}`);
    layer.p[path]=prop;
  }
}
export function layer3DAssetIds(layer:any):string[] {
  const node=layer?.d?.data?.object;
  const materialIds=(m:any)=>[...Object.values<string>(m?.maps || {}),m?.shader?.source?.assetId,...(m?.shader?.graph?.nodes || []).map((n:any)=>n.image)];
  return node?[...new Set<string>([node.source?.assetId,node.blender?.assetId,...materialIds(node.material),...(node.slots || []).flatMap((s:any)=>materialIds(s.material))].filter(Boolean))]:[];
}

/** Assemble a transient world for rendering; there is no scene container in project data. */
export function compositionScene(PM:any,time=PM.time || 0, composition=PM.curComp?.() || PM.proj):Scene3D {
  const layers=(composition?.layers || []).filter((l:any)=>layer3DRole(l));
  const active=(layer:any)=>PM.active?PM.active(layer,time):layer.on!==false;
  const ev=(layer:any,prop:any,path:string)=>{
    try{
      const value=PM.evP?PM.evP(layer,prop,time,path):prop.v;
      if(typeof value!==typeof prop.v || typeof value==='number' && !Number.isFinite(value))return prop.v;
      validateSceneChannel(path,value);return value;
    }catch{return prop.v;}
  };
  const ch=(v:any)=>({v,kf:[],expr:null});
  const values=(layer:any,p:any,prefix:string)=>Object.fromEntries(Object.entries<any>(p || {}).map(([key,prop])=>[key,ch(ev(layer,prop,`${prefix}.${key}`))]));
  const transform=(layer:any)=>Object.fromEntries(Object.entries(TRANSFORM_PATHS).map(([key,path])=>[key,ch((layer.p[path]?ev(layer,layer.p[path],path):key.startsWith('s')?100:0)/(key.startsWith('s')?100:1))]));
  const cameraLayer=layers.find((l:any)=>layer3DRole(l)==='camera' && active(l));
  const objectLayers=layers.filter((l:any)=>layer3DRole(l)==='object' && active(l));
  const lightLayers=layers.filter((l:any)=>layer3DRole(l)==='light' && active(l));
  const allObjectIds=new Set(objectLayers.map((l:any)=>l.id));
  const objects=objectLayers.map((layer:any)=>{
    const node=layer.d.data.object;
    const p=transform(layer),part=animatedPart(PM,layer,time);
    if(part)for(const key of Object.keys(p))if(key in part.base)p[key]!.v+=part.base[key]!-part.prior[key]!;
    // Native orientation is applied before the separately animated rotation.
    for(const [key,axis] of [['rx','x'],['ry','y'],['rz','z']] as const) if(layer.p[`orientation.${axis}`])p[key]!.v+=ev(layer,layer.p[`orientation.${axis}`],`orientation.${axis}`);
    return {...node,...(part?{source:part.source}:{}),id:layer.id,name:layer.name,parent:allObjectIds.has(layer.parent)?layer.parent:null,p,
      material:{...node.material,p:values(layer,node.material.p,'m'),...(node.material.shader?{shader:{...node.material.shader,p:values(layer,node.material.shader.p,'shader')}}:{})},
      slots:(node.slots || []).map((slot:any)=>({...slot,material:{...slot.material,p:values(layer,slot.material.p,`slots.${slot.id}`),...(slot.material.shader?{shader:{...slot.material.shader,p:values(layer,slot.material.shader.p,`slots.${slot.id}.shader`)}}:{})}}))};
  });
  const lights=lightLayers.map((layer:any)=>{
    const node=layer.d.data.light,p=values(layer,node.p,'light'),t=transform(layer);
    return {...node,id:layer.id,name:layer.name,p:{...p,x:t.x,y:t.y,z:t.z}};
  });
  const base=parseScene({});
  const camera=cameraLayer?{...cameraLayer.d.data.camera,p:{...values(cameraLayer,cameraLayer.d.data.camera.p,'camera'),
    ...Object.fromEntries(['x','y','z'].map(key=>[key,transform(cameraLayer)[key]]))}}:base.camera;
  const environment=cameraLayer?{...cameraLayer.d.data.environment,p:values(cameraLayer,cameraLayer.d.data.environment.p,'environment')}:base.environment;
  // New models are immediately visible; explicit lights replace this studio preset.
  if(!lightLayers.length){
    let id='default_sun';while(allObjectIds.has(id))id+='x';
    lights.push(...parseScene({lights:[{id,type:'sun'}]}).lights);
  }
  camera.p.far!.v=Math.max(camera.p.far!.v,camera.p.near!.v+.01);
  const scene=parseScene({objects,lights,camera,environment});
  // Structural source references change only on content edits. Keep signatures
  // and generated geometry reusable across animated frames after validation.
  scene.objects.forEach((object,index)=>{object.source=objects[index]!.source;});
  return scene;
}
