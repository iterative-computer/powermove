import { readExtrusion, extrusionEligible, isExtrudedLayer } from './extrusion-data';
import { listExtrusionSurfaces, readSurfaceProperties } from './extrusion-operations';
import { createMaterialThumbnails } from './material-thumbnails';
import { readMaterialProperties } from './material-properties';
import {lightViewerState,lightPositionFromViewer,lightViewerGuides} from './light-view';
import {canAssignMaterialMap,listModelMaterials,listValidMaterialCompositions,listEligibleMaterialLayers,materialLayerLinks} from './material-maps';
import {getViewportMode,setViewportMode,onViewportChange} from './viewport';
import { createSceneGizmo,getGizmoMode,getGizmoSpace,setGizmoMode,setGizmoSpace,onGizmoState } from './gizmo';
import { compositionRuntime } from './service';
import { createScene,createObject,createLight,parseScene,SCENE3D_DEFINITION } from './schema';
import { convertLegacyScene } from './migration';
import { prepareModelImport } from './import-bundle';
import { compositionScene,layer3DRole,isModelGroup } from './layers';
import { editScene } from './operations';
import type { Scene3DAPI } from '../../kernel/api';
import {navigationNotice,createSceneNavigation,getNavigationMode,setNavigationMode,onNavigationMode,frameComposition,type SceneNavigation} from './navigation';

export function makeScene3DAPI(PM:any,emit:(event:'scene3d:selection'|'scene3d:materials',payload:any)=>void):Scene3DAPI {
  let navigation:SceneNavigation|null=null;
  const thumbnails=createMaterialThumbnails(PM);
  const materialListeners=new Set<()=>void>();
  // All mutation paths, reopen, composition rotation and history replay publish
  // these events. UI consumers never need to intercept individual commands.
  let materialSignature='';
  const changed=()=>{
    const signature=JSON.stringify([PM.proj?.revision,PM.proj?.compId,(PM.curComp?.() || PM.proj)?.layers?.map((l:any)=>[l.id,l.name,l.d?.data?.object?.material?.maps,l.d?.data?.object?.sourceMaterials,l.d?.data?.object?.materialProperties,l.d?.data?.object?.sourceMaterialProperties,l.d?.extrusion,l.threeD,l.type])]);
    if(signature===materialSignature)return;materialSignature=signature;
    emit('scene3d:materials',{layerIds:(PM.curComp?.() || PM.proj)?.layers?.filter((l:any)=>l.d?.data?.object || extrusionEligible(l)).map((l:any)=>l.id) || []});
    for(const listener of materialListeners)listener();
  };
  for(const event of ['project','layers','comps','history:project-patch'])PM.bus?.on?.(event,changed);
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
    getNavigationMode:()=>getNavigationMode(PM),setNavigationMode:mode=>setNavigationMode(PM,mode),
    onNavigationChange:listener=>({dispose:onNavigationMode(PM,listener)}),
    createNavigation:element=>(navigation=createSceneNavigation(PM,element)),
    frame:selected=>{try{navigation?navigation.frame(selected):frameComposition(PM,selected);}catch(error){PM.toast?.(error instanceof Error?error.message:String(error),{key:navigationNotice,error:true});}},
    describe(){return compositionScene(PM,PM.time);},
    selection,select};
}
