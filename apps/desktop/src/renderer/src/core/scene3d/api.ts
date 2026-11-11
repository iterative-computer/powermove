import {activeCameraLayer} from './camera-navigation';
import { readExtrusion, extrusionEligible, isExtrudedLayer } from './extrusion-data';
import { listExtrusionSurfaces, readSurfaceProperties } from './extrusion-operations';
import { createMaterialThumbnails } from './material-thumbnails';
import { readMaterialProperties } from './material-properties';
import {lightViewerState,lightPositionFromViewer,lightViewerGuides,nullViewerPosition,nullViewerState} from './light-view';
import {canAssignMaterialMap,listModelMaterials,listValidMaterialCompositions,listEligibleMaterialLayers,materialLayerLinks} from './material-maps';
import {getViewportMode,setViewportMode,onViewportChange} from './viewport';
import { createSceneGizmo,getGizmoMode,getGizmoSpace,setGizmoMode,setGizmoSpace,onGizmoState } from './gizmo';
import { compositionRuntime } from './service';
import { geometryBounds } from './geometry-bounds';
import { createScene,createObject,createLight,parseScene,SCENE3D_DEFINITION } from './schema';
import { convertLegacyScene } from './migration';
import { prepareModelImport } from './import-bundle';
import { compositionScene,layer3DRole,isModelGroup } from './layers';
import { editScene } from './operations';
import { viewportCamera } from './viewport';
import { PART_DEFAULTS, PART_FIELDS, getActivePart, getSelectedParts, setSelectedParts, listLayerParts, onActivePartChange, partKeyOf, partPath, partsRoot, pruneActiveParts, setActivePart, type PartValues } from './model-parts';
import type { Scene3DAPI } from '../../kernel/api';
import {getNavigationVariant,setNavigationVariant,navigationNotice,createSceneNavigation,getNavigationMode,setNavigationMode,onNavigationMode,frameComposition,type SceneNavigation} from './navigation';

export function makeScene3DAPI(PM:any,emit:(event:'scene3d:selection'|'scene3d:materials',payload:any)=>void):Scene3DAPI {
  let navigation:SceneNavigation|null=null;
  const thumbnails=createMaterialThumbnails(PM);
  const materialListeners=new Set<()=>void>();
  // All mutation paths, reopen, composition rotation and history replay publish
  // these events. UI consumers never need to intercept individual commands.
  let materialSignature='';
  const changed=()=>{
    const signature=JSON.stringify([PM.proj?.revision,PM.proj?.compId,(PM.curComp?.() || PM.proj)?.layers?.map((l:any)=>[l.id,l.name,l.group,l.d?.data?.object?.material?.maps,l.d?.data?.object?.sourceMaterials,l.d?.data?.object?.materialProperties,l.d?.data?.object?.sourceMaterialProperties,l.d?.extrusion,l.threeD,l.type])]);
    if(signature===materialSignature)return;materialSignature=signature;
    for(const id of PM.sel?.layers || []){
      const selected=getSelectedParts(PM,id);if(!selected.length)continue;
      const groups=PM.L?.(id)?.d?.data?.object?.parts || {};
      setSelectedParts(PM,id,selected.filter(key=>!key.startsWith('group_') || !!groups[key]?.members));
    }
    emit('scene3d:materials',{layerIds:(PM.curComp?.() || PM.proj)?.layers?.filter((l:any)=>l.d?.data?.object || extrusionEligible(l)).map((l:any)=>l.id) || []});
    for(const listener of materialListeners)listener();
  };
  for(const event of ['project','layers','comps','history:project-patch'])PM.bus?.on?.(event,changed);
  PM.bus?.on?.('sel',()=>pruneActiveParts(PM,PM.sel?.layers || []));
  const readPart=(layerId:string,key:string,time=PM.time || 0)=>{
    const layer=PM.L?.(layerId),part=layer?.d?.data?.object?.parts?.[key];
    const values={...PART_DEFAULTS} as PartValues;
    if(part)for(const field of PART_FIELDS){const prop=part.p?.[field];if(prop){const value=PM.evP(layer,prop,time,partPath(key,field));(values as any)[field]=typeof value===typeof PART_DEFAULTS[field]?value:prop.v;}}
    return {installed:!!part,channels:part?.p ?? null,values,paths:Object.fromEntries(PART_FIELDS.map(field=>[field,partPath(key,field)])) as Record<keyof PartValues,string>};
  };
  const selection=()=>{
    const ids=(PM.sel?.layers || []).filter((id:string)=>layer3DRole(PM.L?.(id)) || isExtrudedLayer(PM.L?.(id)) || isModelGroup(PM,PM.L?.(id)));
    return {layerId:ids[0] || null,ids};
  };
  const select=(layerId:string|null,ids:string[])=>{
    const selected=ids.length?ids:layerId?[layerId]:[];
    PM.selectLayers?.(selected.filter(id=>layer3DRole(PM.L?.(id)) || isExtrudedLayer(PM.L?.(id)) || isModelGroup(PM,PM.L?.(id))));
    emit('scene3d:selection',selection());PM.bus?.emit?.('scene3d:selection');PM.invalidate?.();
  };
  return {
    geometryBounds:(ids,time)=>geometryBounds(PM,ids,time),
    nullViewerPosition:(id,time,rect)=>nullViewerPosition(PM,id,time,rect),
    nullViewerState:(id,time,rect)=>nullViewerState(PM,id,time,rect),
    getExtrusion:(layerId,time)=>readExtrusion(PM,PM.L?.(layerId),time),
    setExtrusion:(layerId,patch,meta)=>editScene(PM,{operation:'set_extrusion',target:layerId,patch},{origin:'interface',...meta}),
    listSurfaces:layerId=>listExtrusionSurfaces(PM,layerId),
    getSurfaceMaterialProperties:(layerId,surface,time)=>readSurfaceProperties(PM,layerId,surface,time),
    setSurfaceMaterialProperties:(layerId,surface,patch,meta)=>editScene(PM,{operation:'set_surface_material_properties',target:layerId,surface,patch},{origin:'interface',...meta}),
    setSurfaceMaterialMap:(layerId,surface,slot,map,meta)=>editScene(PM,{operation:'set_surface_material_map',target:layerId,surface,slot,map},{origin:'interface',...meta}),
    listMaterials:layerId=>listModelMaterials(PM,layerId),listValidCompositions:layerId=>listValidMaterialCompositions(PM,layerId),
    listEligibleSourceLayers:layerId=>listEligibleMaterialLayers(PM,layerId),
    getMaterialThumbnail:(layerId,source,options)=>thumbnails.get(layerId,source,options),
    clearMaterialThumbnails:()=>thumbnails.clear(),
    canAssignMaterialMap:(layerId,index,slot,source)=>canAssignMaterialMap(PM,layerId,index,slot,source),
    materialLinks:layerId=>materialLayerLinks(PM,layerId),
    getMaterialProperties:(layerId,index,time)=>readMaterialProperties(PM,layerId,index,time),
    setMaterialProperties:(layerId,materialIndex,patch,meta)=>editScene(PM,{operation:'set_material_properties',target:layerId,materialIndex,patch},{origin:'interface',...meta}),
    onMaterialsChange:listener=>{materialListeners.add(listener);return {dispose:()=>{materialListeners.delete(listener);}};},
    setMaterialMap:(layerId,materialIndex,slot,map,meta)=>editScene(PM,{operation:'set_material_map',target:layerId,materialIndex,slot,map},{origin:'interface',...meta}),
    getMode:getGizmoMode,setMode:setGizmoMode,getSpace:getGizmoSpace,setSpace:setGizmoSpace,onGizmoChange:listener=>({dispose:onGizmoState(listener)}),createGizmo:element=>createSceneGizmo(PM,element,time=>compositionRuntime(PM,time)),createScene,createObject,createLight,prepareImport:prepareModelImport,convert:layerId=>convertLegacyScene(PM,layerId),edit:(args,meta)=>editScene(PM,args,{origin:'interface',...meta}),
    lightViewerState:(layerId,time,rect,view)=>lightViewerState(PM,layerId,time,rect,view),
    lightPositionFromViewer:(layerId,time,pointer,rect,view)=>lightPositionFromViewer(PM,layerId,time,pointer,rect,view),
    lightViewerGuides:(layerId,time,rect,view)=>lightViewerGuides(PM,layerId,time,rect,view),
    isGroup:layerId=>isModelGroup(PM,PM.L?.(layerId)),
    getView:()=>getViewportMode(PM),setView:mode=>{navigation?.cancel();if(mode==='camera')setNavigationMode(PM,'select');setViewportMode(PM,mode);},onViewChange:listener=>({dispose:onViewportChange(PM,listener)}),
    getNavigationMode:()=>getNavigationMode(PM),setNavigationMode:mode=>{navigation?.cancel();if(mode!=='select'&&getViewportMode(PM)==='camera'&&!activeCameraLayer(PM))setViewportMode(PM,'editor');setNavigationMode(PM,mode);},
    onNavigationChange:listener=>({dispose:onNavigationMode(PM,listener)}),
    getNavigationVariant:tool=>getNavigationVariant(PM,tool),setNavigationVariant:(tool,variant)=>{navigation?.cancel();setNavigationVariant(PM,tool,variant);},
    createNavigation:element=>(navigation=createSceneNavigation(PM,element)),
    frame:selected=>{try{navigation?navigation.frame(selected):frameComposition(PM,selected);}catch(error){PM.toast?.(error instanceof Error?error.message:String(error),{key:navigationNotice,error:true});}},
    describe(){return compositionScene(PM,PM.time);},
    listParts:layerId=>listLayerParts(PM,PM.L?.(layerId)),
    getPart:(layerId,key,time)=>readPart(layerId,key,time),
    setPart:(layerId,key,patch,meta)=>editScene(PM,{operation:'set_part',target:layerId,part:key,patch},{origin:'interface',label:'Edit model part',...meta}),
    getActivePart:layerId=>getActivePart(PM,layerId),
    setActivePart:(layerId,key)=>setActivePart(PM,layerId,key),
    getSelectedParts:layerId=>getSelectedParts(PM,layerId),
    setSelectedParts:(layerId,keys)=>setSelectedParts(PM,layerId,keys),
    groupParts(layerId,keys,name){
      const previous=new Set(listLayerParts(PM,PM.L(layerId)).map(part=>part.key));
      const result=editScene(PM,{operation:'group_parts',target:layerId,parts:[...keys],...(name?{name}:{})},{origin:'inspector',label:'Group model parts'});
      if(result.ok){const group=listLayerParts(PM,PM.L(layerId)).find(part=>!previous.has(part.key));if(group)setActivePart(PM,layerId,group.key);}
      return result;
    },
    ungroupParts(layerId,key){
      const members=PM.L(layerId)?.d?.data?.object?.parts?.[key]?.members || [];
      const result=editScene(PM,{operation:'ungroup_parts',target:layerId,part:key},{origin:'inspector',label:'Ungroup model parts'});
      if(result.ok)setSelectedParts(PM,layerId,members);return result;
    },
    renamePartGroup:(layerId,key,name)=>editScene(PM,{operation:'rename_part_group',target:layerId,part:key,name},{origin:'inspector',label:'Rename part group'}),
    onActivePartChange:listener=>({dispose:onActivePartChange(PM,listener)}),
    pickPart(layerId,x,y){
      const layer=PM.L?.(layerId);if(layer3DRole(layer)!=='object')return null;
      const runtime=compositionRuntime(PM,PM.time),object=runtime.objects.get(layerId);if(!object)return null;
      const camera=viewportCamera(PM,runtime.camera);runtime.resizeCamera(PM.proj.w,PM.proj.h,camera);
      const hit=runtime.pickHit(x/PM.proj.w*2-1,1-y/PM.proj.h*2,new Set([layerId]),camera);
      if(!hit)return null;
      const root=partsRoot(object),key=partKeyOf(root,hit.object);if(!key)return null;
      // Like a group in a design tool: the first double-click takes the outer
      // part, the next one goes a level deeper toward what was clicked.
      const parts=listLayerParts(PM,layer),chain:string[]=[];
      for(let part=parts.find(part=>part.key===key);part;part=parts.find(candidate=>candidate.key===part!.parent))chain.unshift(part.key);
      let start=0;while(start<chain.length-1 && !parts.find(part=>part.key===chain[start])?.group && parts.filter(p=>p.parent===(start?chain[start-1]:null)).length===1)start++;
      const current=getActivePart(PM,layerId),at=current?chain.indexOf(current):-1;
      return chain[at>=0?Math.min(chain.length-1,at+1):start] ?? null;
    },
    selection,select};
}
