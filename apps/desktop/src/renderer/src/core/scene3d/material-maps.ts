/** Image IDs retain the existing project format; composition maps are explicit. */
export type MaterialMap = string | {compositionId:string};
export const MATERIAL_SLOTS = ['color','normal','roughness','metalness','emissive','ao'] as const;
export type MaterialSlot = typeof MATERIAL_SLOTS[number];
export type MaterialMaps = Partial<Record<MaterialSlot,MaterialMap>>;
export interface SourceMaterialInfo {index:number;name:string;slots:MaterialSlot[];maps:MaterialMaps}

export function objectMapSets(object:any):MaterialMaps[] {
  return [object?.material?.maps || {},...Object.values<MaterialMaps>(object?.sourceMaterials || {})];
}
export function mapImageId(map:MaterialMap|undefined):string|undefined {return typeof map==='string'?map:undefined;}
export function layerCompositionIds(layer:any):string[] {
  if(!layer?.d?.data?.object && !layer?.d?.data?.scene?.objects)return [];
  const objects=layer.d.data.scene?.objects || [layer.d.data.object];
  return [...new Set<string>(objects.flatMap((object:any)=>objectMapSets(object).flatMap(maps=>
    Object.values(maps).flatMap(map=>map && typeof map==='object'?[map.compositionId]:[]))))];
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
export function compositionContains(project:any,ancestor:string,target:string):boolean {
  const queue=[ancestor],seen=new Set<string>();
  while(queue.length){const id=queue.pop()!;if(id===target)return true;if(seen.has(id))continue;seen.add(id);
    queue.push(...compositionDependencies(compositionById(project,id)));}
  return false;
}
export function listValidMaterialCompositions(PM:any,layerId:string):{id:string;name:string;w:number;h:number;fps:number;dur:number}[] {
  const comps=projectCompositions(PM.proj),owner=comps.find(comp=>comp.layers?.some((layer:any)=>layer.id===layerId));
  if(!owner)return [];
  return comps.filter(comp=>comp.id && !compositionContains(PM.proj,comp.id,owner.id))
    .map(({id,name,w,h,fps,dur})=>({id,name,w,h,fps,dur}));
}
/** Missing comps are allowed on reopen/delete; live references must form a DAG. */
export function validateCompositionMaps(project:any):void {
  const done=new Set<string>(),visiting=new Set<string>();
  const visit=(id:string)=>{
    if(visiting.has(id))throw new Error('Composition material reference would create a cycle');
    if(done.has(id))return;
    const comp=compositionById(project,id);if(!comp)return;
    visiting.add(id);for(const child of compositionDependencies(comp))visit(child);visiting.delete(id);done.add(id);
  };
  for(const comp of projectCompositions(project))if(comp.id)visit(comp.id);
}
export function listModelMaterials(PM:any,layerId:string):SourceMaterialInfo[] {
  const layer=PM.L?.(layerId),object=layer?.d?.data?.object;
  if(!object)return [];
  const asset=object.source?.assetId && PM.assets?.get?.(object.source.assetId);
  return (asset?.materials || []).map((material:any)=>({index:material.index,name:material.name,
    slots:[...MATERIAL_SLOTS],maps:{...(object.sourceMaterials?.[material.index] || {})}}));
}
