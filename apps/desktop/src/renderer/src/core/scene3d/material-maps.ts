import { extrusionEligible, isExtrudedLayer, EXTRUSION_SURFACES, type ExtrusionSurface } from './extrusion-data';
/** Asset IDs retain the old format. Layer links are local to their receiver's composition. */
export type MaterialMap = string | {compositionId:string} | {layerId:string};
export const MATERIAL_SLOTS = ['color','normal','roughness','metalness','emissive','ao'] as const;
export type MaterialSlot = typeof MATERIAL_SLOTS[number];
export type MaterialMaps = Partial<Record<MaterialSlot,MaterialMap>>;
export interface SourceMaterialInfo {index:number;name:string;slots:MaterialSlot[];maps:MaterialMaps}
export type MaterialSourceGroup = 'Comps'|'Groups'|'Footage'|'Solids'|'Text & Shapes';
export interface MaterialSourceLayer {id:string;name:string;type:string;group:MaterialSourceGroup}
export interface MaterialLayerLink {surface?:ExtrusionSurface;materialIndex:number|null;slot:MaterialSlot;layerId:string;status:'ready'|'missing'|'ineligible'|'cycle'}
const sourceGroups:Record<string,MaterialSourceGroup>={precomp:'Comps',group:'Groups',image:'Footage',video:'Footage',solid:'Solids',text:'Text & Shapes',shape:'Text & Shapes'};
export function materialSourceEligible(layer:any,comp?:any):boolean {
  const is3D=(layer:any)=>layer.threeD || layer.d?.definition?.startsWith('powermove.3d.') || layer.d?.data?.object || layer.d?.data?.camera || layer.d?.data?.scene || isExtrudedLayer(layer);
  return !!sourceGroups[layer?.type] && !is3D(layer) && !(comp && materialSourceLayers(comp,layer.id).some(is3D));
}
/** Group sources own their complete subtree; membership is independent of stack order. */
export function materialSourceLayers(comp:any,sourceId:string):any[] {
  const source=comp.layers.find((layer:any)=>layer.id===sourceId);
  if(source?.type!=='group')return source?[source]:[];
  const children=new Map<string,any[]>();
  for(const layer of comp.layers){const list=children.get(layer.group)||[];list.push(layer);children.set(layer.group,list);}
  const ids=new Set<string>(),pending=[source];
  while(pending.length){const layer=pending.pop()!;if(ids.has(layer.id))continue;ids.add(layer.id);if(layer.type==='group')pending.push(...children.get(layer.id)||[]);}
  return comp.layers.filter((layer:any)=>ids.has(layer.id));
}
/** Hidden source roots still supply pixels; child visibility and timing remain authored. */
export function materialSourceActive(PM:any,comp:any,sourceId:string,layer:any,time:number):boolean {
  if(layer.id===sourceId)return layer.type==='group' || time>=layer.from-1e-6 &&
    (time<layer.from+layer.dur-1e-6 || Math.abs(time-comp.dur)<=1e-6 && Math.abs(time-layer.from-layer.dur)<=1e-6);
  if(!PM.active({...layer,group:null},time))return false;
  const ancestors=PM.groupAncestors?.(layer,comp.layers)||[],at=ancestors.findIndex((group:any)=>group.id===sourceId);
  return !ancestors.slice(0,Math.max(0,at)).some((group:any)=>{
    const on=group.on;return !(on&&typeof on==='object'?(PM.evP?.(group,on,time,'l.on')??on.v):on);
  });
}
/** Generated geometry supplies UVs; custom and imported meshes must own them. */
export function materialReceiverHasUVs(PM:any,layer:any):boolean {
  const source=layer?.d?.data?.object?.source;
  if(isExtrudedLayer(layer))return true;
  if(!source)return false;
  if(source.mesh)return !!source.mesh.uvs?.length;
  if(source.assetId){
    const asset=PM.assets?.get?.(source.assetId);
    if(asset?.object3d){let hasUV=false;asset.object3d.traverse((mesh:any)=>{if(mesh.isMesh && mesh.geometry?.getAttribute('uv'))hasUV=true;});return hasUV;}
    if(asset?.mesh)return !!asset.mesh.texcoords?.length;
  }
  return true;
}
export function objectMapSets(object:any):MaterialMaps[] {
  return [object?.material?.maps || {},...Object.values<MaterialMaps>(object?.sourceMaterials || {})];
}
export function layerMaterialMaps(layer:any):MaterialMap[] {
  const objects=layer?.d?.data?.scene?.objects || (layer?.d?.data?.object?[layer.d.data.object]:[]);
  return [...objects.flatMap((object:any)=>objectMapSets(object).flatMap(maps=>Object.values(maps))),...Object.values<any>(layer?.d?.extrusion?.surfaces || {}).flatMap(surface=>Object.values<MaterialMap>(surface.maps || {}))];
}
export function mapImageId(map:MaterialMap|undefined):string|undefined {return typeof map==='string'?map:undefined;}
export function layerCompositionIds(layer:any):string[] {
  return [...new Set(layerMaterialMaps(layer).flatMap(map=>typeof map==='object' && 'compositionId' in map?[map.compositionId]:[]))];
}
export function compositionDependencies(comp:any):string[] {
  return [...new Set<string>((comp?.layers || []).flatMap((layer:any)=>[
    ...(layer.type==='precomp' && layer.d?.comp?[layer.d.comp]:[]),...layerCompositionIds(layer)]))];
}
export function projectCompositions(project:any):any[] {
  return [{...project,id:project.compId || project.id,name:project.compName || project.name},...Object.values<any>(project.comps || {})];
}
export function compositionById(project:any,id:string):any {
  return id===(project.compId || project.id)?project:project.comps?.[id];
}
export function materialOwner(project:any,layerId:string):any {
  return projectCompositions(project).find(comp=>comp.layers?.some((layer:any)=>layer.id===layerId));
}
export function compositionContains(project:any,ancestor:string,target:string):boolean {
  const queue=[ancestor],seen=new Set<string>();
  while(queue.length){const id=queue.pop()!;if(id===target)return true;if(seen.has(id))continue;seen.add(id);
    queue.push(...compositionDependencies(compositionById(project,id)));}
  return false;
}
/** A shared graph catches layer -> precomp -> comp-material -> receiver loops,
 * including effect/matte providers used when rasterizing the source layer. */
function materialGraph(project:any):Map<string,string[]> {
  const graph=new Map<string,string[]>();
  for(const comp of projectCompositions(project)){
    const key=(id:string)=>JSON.stringify(['layer',comp.id,id]);
    graph.set(JSON.stringify(['comp',comp.id]),(comp.layers || []).map((l:any)=>key(l.id)));
    const ids=new Set((comp.layers || []).map((l:any)=>l.id)),children=new Map<string,string[]>();
    for(const layer of comp.layers || [])if(layer.group){const list=children.get(layer.group)||[];list.push(key(layer.id));children.set(layer.group,list);}
    for(const layer of comp.layers || []){
      const deps:string[]=[];
      if(layer.type==='group')deps.push(...children.get(layer.id)||[]);
      if(layer.type==='precomp' && layer.d?.comp)deps.push(JSON.stringify(['comp',layer.d.comp]));
      for(const map of layerMaterialMaps(layer))if(typeof map==='object'){
        if('compositionId' in map)deps.push(JSON.stringify(['comp',map.compositionId]));
        else if(ids.has(map.layerId))deps.push(key(map.layerId));
      }
      // Source-space materials include masks and effects; their providers also
      // belong to the graph, even if currently hidden or disabled.
      for(const fx of layer.fx || [])for(const ref of Object.values<any>(fx.inputs || {}))
        if(ref?.layerId && ids.has(ref.layerId) && ref.layerId!==layer.id)deps.push(key(ref.layerId));
      if(layer.matteSource && ids.has(layer.matteSource))deps.push(key(layer.matteSource));
      graph.set(key(layer.id),deps);
    }
  }
  return graph;
}
function reaches(graph:Map<string,string[]>,start:string,target:string):boolean {
  const pending=[start],seen=new Set<string>();
  while(pending.length){const key=pending.pop()!;if(key===target)return true;if(seen.has(key))continue;seen.add(key);pending.push(...graph.get(key) || []);}
  return false;
}
export function materialLayerStatus(project:any,receiverId:string,sourceId:string):MaterialLayerLink['status'] {
  const owner=materialOwner(project,receiverId),source=owner?.layers?.find((l:any)=>l.id===sourceId);
  if(!source)return 'missing';
  if(sourceId===receiverId)return 'cycle';
  if(!materialSourceEligible(source,owner))return 'ineligible';
  const key=(id:string)=>JSON.stringify(['layer',owner.id,id]);
  return reaches(materialGraph(project),key(sourceId),key(receiverId))?'cycle':'ready';
}
export function listEligibleMaterialLayers(PM:any,receiverId:string):{group:MaterialSourceGroup;layers:MaterialSourceLayer[]}[] {
  const owner=materialOwner(PM.proj,receiverId),receiver=owner?.layers.find((l:any)=>l.id===receiverId);
  if(!materialReceiverHasUVs(PM,receiver))return [];
  const groups:MaterialSourceGroup[]=['Comps','Groups','Footage','Solids','Text & Shapes'];
  return groups.map(group=>({group,layers:(owner.layers || []).filter((layer:any)=>sourceGroups[layer.type]===group && materialLayerStatus(PM.proj,receiverId,layer.id)==='ready')
    .map((layer:any)=>({id:layer.id,name:layer.name,type:layer.type,group}))}));
}
export function materialLayerLinks(PM:any,receiverId:string):MaterialLayerLink[] {
  const layer=materialOwner(PM.proj,receiverId)?.layers.find((l:any)=>l.id===receiverId),object=layer?.d?.data?.object;
  const sets:[number|null,MaterialMaps][]=extrusionEligible(layer)?EXTRUSION_SURFACES.map((surface,index)=>[index,layer.d?.extrusion?.surfaces?.[surface]?.maps || {}]):[[null,object?.material?.maps || {}],...Object.entries<MaterialMaps>(object?.sourceMaterials || {}).map(([index,maps]):[number,MaterialMaps]=>[Number(index),maps])];
  return sets.flatMap(([materialIndex,maps])=>Object.entries(maps).flatMap(([slot,map])=>typeof map==='object' && 'layerId' in map
    ?[{materialIndex,...(extrusionEligible(layer)?{surface:EXTRUSION_SURFACES[materialIndex!]}:{}),slot:slot as MaterialSlot,layerId:map.layerId,status:materialLayerStatus(PM.proj,receiverId,map.layerId)}]:[]));
}
export function remapMaterialLayerLinks(layer:any,ids:ReadonlyMap<string,string>):void {
  for(const map of layerMaterialMaps(layer))if(typeof map==='object' && 'layerId' in map && ids.has(map.layerId))map.layerId=ids.get(map.layerId)!;
}
export function listValidMaterialCompositions(PM:any,layerId:string):{id:string;name:string;w:number;h:number;fps:number;dur:number}[] {
  const comps=projectCompositions(PM.proj),owner=materialOwner(PM.proj,layerId);
  if(!owner)return [];
  return comps.filter(comp=>comp.id && !compositionContains(PM.proj,comp.id,owner.id))
    .map(({id,name,w,h,fps,dur})=>({id,name,w,h,fps,dur}));
}
/** Missing links stay serialized and are flagged by the API; rendering falls
 * back to the original slot. Undoing deletion can therefore restore the link. */
export function validateCompositionMaps(project:any):void {
  const graph=materialGraph(project),done=new Set<string>(),visiting=new Set<string>();
  const visit=(id:string)=>{
    if(visiting.has(id))throw new Error('Composition material reference would create a cycle');
    if(done.has(id))return;
    visiting.add(id);for(const child of compositionDependencies(compositionById(project,id)))visit(child);visiting.delete(id);done.add(id);
  };
  for(const comp of projectCompositions(project))if(comp.id)visit(comp.id);
  for(const comp of projectCompositions(project))for(const layer of comp.layers || [])for(const map of layerMaterialMaps(layer))if(typeof map==='object'){
    const receiver=JSON.stringify(['layer',comp.id,layer.id]);
    const source='layerId' in map?JSON.stringify(['layer',comp.id,map.layerId]):JSON.stringify(['comp',map.compositionId]);
    if(reaches(graph,source,receiver))throw new Error('Composition or layer material reference would create a cycle');
  }
  for(const comp of projectCompositions(project))for(const layer of comp.layers || [])for(const map of layerMaterialMaps(layer))
    if(typeof map==='object' && 'layerId' in map){const source=comp.layers.find((l:any)=>l.id===map.layerId);if(source && !materialSourceEligible(source,comp))throw new Error('Layer material source must be a group, precomp, solid, image, video, text or shape');}
}
export function listModelMaterials(PM:any,layerId:string):SourceMaterialInfo[] {
  const layer=PM.L?.(layerId),object=layer?.d?.data?.object;
  if(isExtrudedLayer(layer))return EXTRUSION_SURFACES.map((surface,index)=>({index,name:surface[0]!.toUpperCase()+surface.slice(1),slots:[...MATERIAL_SLOTS],maps:{...layer.d.extrusion.surfaces?.[surface]?.maps}}));
  if(!object)return [];
  const asset=object.source?.assetId && PM.assets?.get?.(object.source.assetId);
  return (asset?.materials || (!object.source?.assetId?[{index:0,name:'Material'}]:[])).map((material:any)=>({index:material.index,name:material.name,
    slots:[...MATERIAL_SLOTS],maps:{...(object.sourceMaterials?.[material.index] || {})}}));
}

export type MaterialAssignmentCheck = {ok:true;reason?:never} | {ok:false;reason:string};
/** Read-only drag-over validation. The setter runs the same checks again at drop time. */
export function canAssignMaterialMap(PM:any,receiverId:string,materialIndex:number|null,slot:MaterialSlot,source:MaterialMap|null):MaterialAssignmentCheck {
  const fail=(reason:string):MaterialAssignmentCheck=>({ok:false,reason});
  const owner=materialOwner(PM.proj,receiverId),receiver=owner?.layers?.find((l:any)=>l.id===receiverId);
  // Extruded text and shapes take sources on their Front, Bevel, Side and Back (0–3).
  if(!receiver?.d?.data?.object && !isExtrudedLayer(receiver))return fail('Choose a model layer');
  if(receiver.lock || (PM.groupAncestors?.(receiver) || []).some((l:any)=>l.lock))return fail('This layer is locked');
  if(!MATERIAL_SLOTS.includes(slot))return fail('Choose a material map slot');
  if(materialIndex!=null && (!Number.isInteger(materialIndex) || materialIndex<0))return fail('Invalid source material index');
  if(source!==null && materialIndex!=null && !listModelMaterials(PM,receiverId).some(m=>m.index===materialIndex))return fail('Source material is unavailable');
  if(source===null)return {ok:true};
  if(typeof source==='string')return source.length?{ok:true}:fail('Choose an image asset');
  if(!source || typeof source!=='object' || Object.keys(source).length!==1)return fail('Choose a material source');
  if(slot!=='color' && slot!=='emissive')return fail('Composition and layer textures support base color and emissive slots');
  if('layerId' in source){
    const status=materialLayerStatus(PM.proj,receiverId,source.layerId);
    if(status==='cycle')return fail('Layer material reference would create a cycle');
    if(status!=='ready')return fail('Choose an eligible source layer in the same composition');
  }else if('compositionId' in source){
    if(!compositionById(PM.proj,source.compositionId))return fail('Composition is unavailable');
    if(reaches(materialGraph(PM.proj),JSON.stringify(['comp',source.compositionId]),JSON.stringify(['layer',owner.id,receiverId])))
      return fail('Composition material reference would create a cycle');
  }else return fail('Choose a material source');
  if(!materialReceiverHasUVs(PM,receiver))return fail('This mesh needs UV coordinates to receive a layer material');
  return {ok:true};
}
