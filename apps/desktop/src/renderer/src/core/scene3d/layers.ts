import { environmentProperties,evaluateEnvironmentNode } from './environment-data';
import { extrusionProperties,isExtrudedLayer,isShared3DPlane } from './extrusion-data';
import { objectMapSets,mapImageId } from './material-maps';
import { objectSchema, lightSchema, cameraSchema, environmentSchema, parseScene, validateSceneChannel, type Scene3D } from './schema';

export const LAYER3D_DEFINITIONS = {
  object: 'powermove.3d.object', light: 'powermove.3d.light', camera: 'powermove.3d.camera'
} as const;
export type Layer3DRole = keyof typeof LAYER3D_DEFINITIONS;
export function layer3DRole(layer: any): Layer3DRole|null {
  return (Object.keys(LAYER3D_DEFINITIONS) as Layer3DRole[]).find(role => layer?.d?.definition === LAYER3D_DEFINITIONS[role]) ?? null;
}
const LIGHT_KINDS:Record<string,{label:string;icon:string}>={
  sun:{label:'Sun Light',icon:'sun'},point:{label:'Point Light',icon:'lightbulb'},spot:{label:'Spot Light',icon:'spotlight'},
  area:{label:'Softbox Light',icon:'softbox'},ambient:{label:'Ambient Light',icon:'ambient'},environment:{label:'Environment Light',icon:'skyLight'}
};
/** What a 3D layer is, in the words and glyph the inspector, Timeline and
 *  menus show: a World is not a "3D Model", an Environment Light not a "Light". */
/** `surrounds`: a World, Ambient or Environment light lights the whole scene
 * from everywhere. It has no place, so no Transform, gizmo or parent. */
export function layer3DKind(layer:any):{label:string;icon:string;surrounds?:boolean}|null {
  const role=layer3DRole(layer),data=layer?.d?.data;
  if(role==='camera')return {label:'Camera',icon:'cam'};
  if(role==='light'){const kind=LIGHT_KINDS[data?.light?.type] ?? {label:'Light',icon:'lightbulb'};return ['ambient','environment'].includes(data?.light?.type)?{...kind,surrounds:true}:kind;}
  if(role!=='object')return null;
  const object=data?.object;
  if(object?.world)return {label:'World',icon:'globe',surrounds:true};
  if(object?.terrain)return {label:'Terrain',icon:'mountains'};
  if(object?.scatter)return {label:'Scatter',icon:'tree'};
  if(object?.fog)return {label:'Fog Volume',icon:'fog'};
  return {label:'3D Model',icon:'cube'};
}
/** Whether a layer can take part in Link Layers (parenting) at all. */
export function layerCanLink(PM:any,layer:any):boolean {
  return !!layer && PM.TYPE_META?.[layer.type]?.transform!==false && !layer3DKind(layer)?.surrounds;
}
/** Model groups use the same world coordinates as their contents. Mixed artwork
 * groups retain the composition's existing 2D interaction. */
export function isModelGroup(PM:any,layer:any):boolean {
  if(layer?.type!=='group')return false;
  const members=(PM.curComp?.() || PM.proj)?.layers?.filter((child:any)=>child.type!=='group' && child.type!=='audio' && child.type!=='null'
    && (PM.groupAncestors?.(child) || []).some((group:any)=>group.id===layer.id)) || [];
  return members.length>0 && members.every((child:any)=>!!layer3DRole(child));
}
/** Nulls driving models share their scene-unit coordinate system. New controls
 * persist that basis; older controls are recognized from their descendants. */
export function isModelNull(PM:any,layer:any):boolean {
  if(layer?.type!=='null'||layer.threeD!==true)return false;
  if(layer.d?.boundsNullSpace)return layer.d.boundsNullSpace==='scene';
  const layers=(PM.curComp?.() || PM.proj)?.layers || [],children=new Map<string,any[]>();
  for(const child of layers)for(const id of new Set([child.parent,child.group].filter(Boolean))){const list=children.get(id) || [];list.push(child);children.set(id,list);}
  const seen=new Set([layer.id]),queue=[layer.id];let models=0;
  for(let i=0;i<queue.length;i++)for(const child of children.get(queue[i]) || []){
    if(seen.has(child.id))continue;seen.add(child.id);queue.push(child.id);
    if(['group','null','audio'].includes(child.type))continue;
    if(!layer3DRole(child))return false;models++;
  }
  return models>0;
}
/** Native nulls in an extrusion scene follow the same camera as the artwork. */
export function isSceneNull(PM:any,layer:any):boolean {
  const comp=PM.curComp?.()||PM.proj;
  return layer?.type==='null'&&layer.threeD===true&&(isModelNull(PM,layer)||(comp?.layers||[]).some((child:any)=>isExtrudedLayer(child)||isShared3DPlane(comp,child)));
}
/** Timeline/inspector names for a part's channels. */
export const PART_LABELS:Record<string,string>={x:'Position X',y:'Position Y',z:'Position Z',rx:'X Rotation',ry:'Y Rotation',rz:'Z Rotation',scale:'Scale',visible:'Visible'};
/** Model-only groups cannot flatten their contents, just like models themselves. */
export function isAlways3DLayer(PM:any,layer:any):boolean {
  return !!layer3DRole(layer) || isModelGroup(PM,layer);
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
    return {light:{type:node.type,p,castShadow:node.castShadow,...(node.world?{world:node.world}:{})}};
  }
  if(Object.keys(raw.camera?.p || {}).some(key=>['x','y','z'].includes(key))) throw new Error('Use the camera layer transform for position');
  const node=parseScene({camera:cameraSchema.parse(raw.camera ?? {}),environment:environmentSchema.parse(raw.environment ?? {})});
  const {x,y,z,...p}=node.camera.p;
  return {camera:{projection:node.camera.projection,p},environment:node.environment};
}
export function layer3DProperties(layer: any): any[] {
  const role=layer3DRole(layer),data=layer?.d?.data;
  if(!role)return [...extrusionProperties(layer),...environmentProperties(layer?.d)];
  const out:any[]=[];
  const collect=(p:any,prefix:string,group:string)=>{
    for(const [key,prop] of Object.entries<any>(p || {})) if(prop && Array.isArray(prop.kf))
      out.push({key:`${prefix}.${key}`,prop,label:({opacity:'Opacity',roughness:'Roughness',metallic:'Metallic',emissionColor:'Emission Color',emissionIntensity:'Emission Intensity',ambientResponse:'Ambient Response'} as Record<string,string>)[key] || key,group});
  };
  if(role==='object'){
    out.push(...environmentProperties(data.object));
    collect(data.object?.material?.p,'m','Material');
    collect(data.object?.materialProperties,'mi','Material');
    for(const [index,p] of Object.entries(data.object?.sourceMaterialProperties || {}))collect(p,`mi.${index}`,`Material ${index}`);
    for(const [key,part] of Object.entries<any>(data.object?.parts || {}))for(const [field,prop] of Object.entries<any>(part?.p || {}))
      if(prop && Array.isArray(prop.kf))out.push({key:`pt.${key}.${field}`,prop,label:PART_LABELS[field] || field,group:part.name || 'Part'});
  }
  if(role==='light'){collect(data.light?.p,'light','Light');out.push(...environmentProperties(data.light));}
  if(role==='camera'){collect(data.camera?.p,'camera','Camera');collect(data.environment?.p,'environment','Environment');}
  return out;
}
/** Validate before mutating live channels, including the camera's coupled clip planes. */
export function validateLayer3DChannel(layer:any,path:string,property:any):void {
  const alias=Object.keys(TRANSFORM_PATHS).find(key=>TRANSFORM_PATHS[key]===path);
  if(alias){
    const factor=alias.startsWith('s')?.01:1;
    objectSchema.shape.p.unwrap().shape[alias]!.parse({...property,v:property.v*factor,kf:property.kf.map((key:any)=>({...key,v:key.v*factor}))});
  }else validateSceneChannel(path,property);
  if(layer3DRole(layer)==='object' && /^(terrain|scatter)\./.test(path)){
    const data=clone(layer.d.data),[kind,key]=path.split('.');
    if(key?.startsWith('region')){const [,region,field]=path.split('.');data.object.terrain.regions[Number(region!.slice(6))].p[field!]=clone(property);}
    else if(data.object[kind!])data.object[kind!].p[key!]=clone(property);
    parseLayer3DData('object',data);
  }
  if(layer3DRole(layer)==='camera' && (path==='camera.near' || path==='camera.far')){
    const data=clone(layer.d.data);data.camera.p[path.slice(7)]=clone(property);parseLayer3DData('camera',data);
  }
}
export function initializeLayer3D(layer:any, properties:any={}): void {
  const role=layer3DRole(layer);if(!role)return;
  layer.d.data=parseLayer3DData(role,layer.d.data);
  layer.threeD=true;
  for(const [path,raw] of Object.entries<any>(properties))if(layer.p[path])layer.p[path]=typeof raw==='object'?clone(raw):{v:raw,kf:[],expr:null};
  const position=role==='light'?[-3,5,4]:role==='camera'?[0,0,6]:[0,0,0];
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
  const sources=[node?.world?.source,node?.terrain?.heightSource,layer?.d?.data?.light?.world?.source].filter((v):v is string=>typeof v==='string');
  if(!node)return [...new Set<string>([...sources,...Object.values<any>(layer?.d?.extrusion?.surfaces || {}).flatMap(surface=>Object.values(surface.maps || {}).map(map=>mapImageId(map as any))).filter((id):id is string=>!!id)])];
  return node?[...new Set<string>([...sources,node.source?.assetId,...objectMapSets(node).flatMap(maps=>Object.values(maps).map(mapImageId))].filter((id):id is string=>!!id))]:[];
}

/** Assemble a transient world for rendering; there is no scene container in project data. */
const sceneWarnings=new WeakMap<Scene3D,string[]>();
export const compositionSceneWarnings=(scene:Scene3D):readonly string[]=>sceneWarnings.get(scene) || [];
export function compositionScene(PM:any,time=PM.time || 0, composition=PM.curComp?.() || PM.proj):Scene3D {
  const warnings:string[]=[];
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
  const scatterSources=new Set<string>();
  for(const layer of layers)if(active(layer) && layer.d?.data?.object?.scatter){scatterSources.add(layer.d.data.object.scatter.model);scatterSources.add(layer.d.data.object.scatter.surface);}
  const objectLayers=layers.filter((l:any)=>layer3DRole(l)==='object' && (active(l) || scatterSources.has(l.id)));
  const lightLayers=layers.filter((l:any)=>layer3DRole(l)==='light' && active(l));
  const allObjectIds=new Set(objectLayers.map((l:any)=>l.id));
  const objects=objectLayers.map((layer:any)=>{
    const node=evaluateEnvironmentNode(layer.d.data.object,(prop,path)=>ev(layer,prop,path),message=>warnings.push(message));
    const p=transform(layer);p.visible=ch(active(layer));
    // Native orientation is applied before the separately animated rotation.
    for(const [key,axis] of [['rx','x'],['ry','y'],['rz','z']] as const) if(layer.p[`orientation.${axis}`])p[key]!.v+=ev(layer,layer.p[`orientation.${axis}`],`orientation.${axis}`);
    return {...node,id:layer.id,name:layer.name,parent:allObjectIds.has(layer.parent)?layer.parent:null,p,
      material:{...node.material,p:values(layer,node.material.p,'m')},
      materialProperties:values(layer,node.materialProperties,'mi'),
      sourceMaterialProperties:Object.fromEntries(Object.entries(node.sourceMaterialProperties || {}).map(([index,p])=>[index,values(layer,p,`mi.${index}`)])),
      parts:Object.fromEntries(Object.entries<any>(node.parts || {}).map(([key,part])=>[key,{name:part.name,p:values(layer,part.p,`pt.${key}`)}]))};
  });
  const lights=lightLayers.map((layer:any)=>{
    const node=evaluateEnvironmentNode(layer.d.data.light,(prop,path)=>ev(layer,prop,path)),p=values(layer,node.p,'light'),t=transform(layer);
    return {...node,id:layer.id,name:layer.name,p:{...p,x:t.x,y:t.y,z:t.z}};
  });
  const candidates=objects.reduce((n:number,object:any)=>n+(object.scatter?.p.count.v || 0),0);
  if(candidates>1_000_000){
    const ratio=1_000_000/candidates;
    for(const object of objects)if(object.scatter)object.scatter.p.count.v=Math.floor(object.scatter.p.count.v*ratio);
    warnings.push('Studio reduced animated scatter density to fit the one-million-instance budget. Saved counts are unchanged.');
  }
  // Never replace the implicit camera of a saved, unversioned composition.
  // Schema defaults remain v1 for legacy scene containers and templates too.
  const base=parseScene(composition?.scene3dDefaultsVersion===2?{camera:{p:{x:0,y:0,z:6}}}:{});
  const camera=cameraLayer?{...cameraLayer.d.data.camera,p:{...values(cameraLayer,cameraLayer.d.data.camera.p,'camera'),
    ...Object.fromEntries(['x','y','z'].map(key=>[key,transform(cameraLayer)[key]]))}}:base.camera;
  const environment=cameraLayer?{...cameraLayer.d.data.environment,p:values(cameraLayer,cameraLayer.d.data.environment.p,'environment')}:base.environment;
  // New models are immediately visible; explicit lights replace this studio preset.
  if(!lightLayers.length && !objects.some((o:any)=>o.world?.mode!=='none' && o.world)){
    let id='default_sun';while((composition?.layers || []).some((layer:any)=>layer.id===id))id+='x';
    lights.push(...parseScene({lights:[{id,type:'sun'}]}).lights);
  }
  camera.p.far!.v=Math.max(camera.p.far!.v,camera.p.near!.v+.01);
  const scene=parseScene({objects,lights,camera,environment});
  // Structural source references change only on content edits. Keep signatures
  // and generated geometry reusable across animated frames after validation.
  scene.objects.forEach((object,index)=>{object.source=objects[index]!.source;});
  sceneWarnings.set(scene,[...new Set(warnings)]);
  return scene;
}
