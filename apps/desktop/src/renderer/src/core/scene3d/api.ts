import { resolveStudioQuality, STUDIO_QUALITY_PRESETS } from '../../../../shared/studio-quality';
import { describeStudioQuality, STUDIO_QUALITY_CHOICES, STUDIO_QUALITY_FIELDS } from '../../../../shared/studio-quality-ui';
import { extrusionFontStatus } from './extrusion-fonts';
import { readExtrusion, extrusionEligible, isExtrudedLayer } from './extrusion-data';
import { viewerService } from '../../legacy/core/services';
import { listExtrusionSurfaces, readSurfaceProperties } from './extrusion-operations';
import { createMaterialThumbnails } from './material-thumbnails';
import { readMaterialProperties } from './material-properties';
import {lightViewerState,lightPositionFromViewer,lightViewerGuides,sunAnglesFromViewer,nullViewerPosition,nullViewerState} from './light-view';
import {canAssignMaterialMap,listModelMaterials,listValidMaterialCompositions,listEligibleMaterialLayers,materialLayerLinks} from './material-maps';
import * as THREE from 'three';
import { MATERIAL_PRESETS, SHOT_TYPES, surfaceAt, highlightPlacement } from '../studio/operations';

/** Softboxes being dragged by light by pointing, with where each started. */
const highlightDrags = new Map<string,{x:number;y:number;z:number;tx:number;ty:number;tz:number}>();
import { studioProgress, studioPreview, setStudioPreview, onStudioPreview } from '../studio';
import { studioSwatch } from '../studio/swatch';
import { skyThumbnail } from '../studio/sky-thumbnail';
import {getViewportMode,setViewportMode,onViewportChange,lookFrom} from './viewport';
import {viewsState,setViewLayout,setActivePane,setPaneView,renderPane,paneSelection,onViewsChange,PANE_VIEWS} from './views';
import { createSceneGizmo,getGizmoMode,getGizmoSpace,setGizmoMode,setGizmoSpace,onGizmoState } from './gizmo';
import { compositionRuntime } from './service';
import { geometryBounds } from './geometry-bounds';
import { createScene,createObject,createLight,parseScene,SCENE3D_DEFINITION } from './schema';
import { convertLegacyScene } from './migration';
import { prepareModelImport } from './import-bundle';
import { compositionScene,layer3DRole,isModelGroup } from './layers';
import { editScene } from './operations';
import { viewportCamera } from './viewport';
import { PART_DEFAULTS, PART_FIELDS, getActivePart, listLayerParts, onActivePartChange, partKeyOf, partPath, partsRoot, pruneActiveParts, setActivePart, type PartValues } from './model-parts';
import type { Scene3DAPI } from '../../kernel/api';
import { getLightTool, setLightTool, onLightTool, lightToolOf, lightInfo } from './light-tool';
import {navigationNotice,createSceneNavigation,getNavigationMode,setNavigationMode,onNavigationMode,frameComposition,type SceneNavigation} from './navigation';

export function makeScene3DAPI(PM:any,emit:(event:'scene3d:selection'|'scene3d:materials',payload:any)=>void):Scene3DAPI {
  let navigation:SceneNavigation|null=null;
  const thumbnails=createMaterialThumbnails(PM);
  const materialListeners=new Set<()=>void>();
  const qualityListeners=new Set<()=>void>();
  let qualitySignature='';
  // All mutation paths, reopen, composition rotation and history replay publish
  // these events. UI consumers never need to intercept individual commands.
  let materialSignature='';
  const changed=()=>{
    const signature=JSON.stringify([PM.proj?.revision,PM.proj?.compId,(PM.curComp?.() || PM.proj)?.layers?.map((l:any)=>[l.id,l.name,l.group,l.d?.data?.object?.material?.maps,l.d?.data?.object?.sourceMaterials,l.d?.data?.object?.materialProperties,l.d?.data?.object?.sourceMaterialProperties,l.d?.extrusion,l.threeD,l.type])]);
    const qualityKey=JSON.stringify([(PM.curComp?.() || PM.proj)?.compId,(PM.curComp?.() || PM.proj)?.studioQuality]);
    if(qualityKey!==qualitySignature){qualitySignature=qualityKey;for(const listener of qualityListeners)listener();}
    if(signature===materialSignature)return;materialSignature=signature;
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
    renderQuality: {
      presets: STUDIO_QUALITY_PRESETS,
      choices: STUDIO_QUALITY_CHOICES,
      fields: STUDIO_QUALITY_FIELDS,
      describe: (quality, frames) => describeStudioQuality(quality ?? (PM.curComp?.() || PM.proj).studioQuality, frames),
      onChange: listener => {qualityListeners.add(listener);return {dispose:()=>qualityListeners.delete(listener)};},
      get: () => resolveStudioQuality((PM.curComp?.() || PM.proj).studioQuality),
      set: (quality, meta) => editScene(PM, {operation:'set_render_quality', ...(typeof quality === 'string' ? {preset:quality} : {patch:quality})}, {origin:'interface',label:'Change render quality',...meta}),
    },
    getExtrusion:(layerId,time)=>readExtrusion(PM,PM.L?.(layerId),time),
    extrusionFontStatus:layerId=>{const layer=PM.L?.(layerId);return layer?.type==='text'?extrusionFontStatus(PM,layer.d):{state:'ready',message:null};},
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
    sunFromViewer:(u,v,time)=>sunAnglesFromViewer(PM,time ?? PM.time,u,v),
    lightViewerGuides:(layerId,time,rect,view)=>lightViewerGuides(PM,layerId,time,rect,view),
    isGroup:layerId=>isModelGroup(PM,PM.L?.(layerId)),
    getView:()=>getViewportMode(PM),setView:mode=>{navigation?.cancel();if(mode==='camera')setNavigationMode(PM,'select');setViewportMode(PM,mode);},onViewChange:listener=>({dispose:onViewportChange(PM,listener)}),
    lookFrom:side=>{navigation?.cancel();lookFrom(PM,side);},
    views:{
      get:()=>viewsState(PM),
      setLayout:layout=>{navigation?.cancel();setViewLayout(PM,layout);},
      setActive:index=>{navigation?.cancel();setActivePane(PM,index);},
      setPaneView:(index,view)=>{navigation?.cancel();setPaneView(PM,index,view);},
      render:(index,width,height)=>renderPane(PM,index,width,height),
      selection:(index,width,height)=>paneSelection(PM,index,width,height),
      options:PANE_VIEWS,
      onChange:listener=>({dispose:onViewsChange(PM,listener)}),
    },
    getNavigationMode:()=>getNavigationMode(PM),setNavigationMode:mode=>{if(mode!=='select')setLightTool(PM,{on:false});setNavigationMode(PM,mode);},
    onNavigationChange:listener=>({dispose:onNavigationMode(PM,listener)}),
    createNavigation:element=>(navigation=createSceneNavigation(PM,element)),
    frame:selected=>{try{navigation?navigation.frame(selected):frameComposition(PM,selected);}catch(error){PM.toast?.(error instanceof Error?error.message:String(error),{key:navigationNotice,error:true});}},
    describe(){return compositionScene(PM,PM.time);},
    listParts:layerId=>listLayerParts(PM,PM.L?.(layerId)),
    getPart:(layerId,key,time)=>readPart(layerId,key,time),
    setPart:(layerId,key,patch,meta)=>editScene(PM,{operation:'set_part',target:layerId,part:key,patch},{origin:'interface',label:'Edit model part',...meta}),
    getActivePart:layerId=>getActivePart(PM,layerId),
    setActivePart:(layerId,key)=>setActivePart(PM,layerId,key),
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
      const parts=listLayerParts(PM,layer),chain=key.split('_').map((_,i,all)=>all.slice(0,i+1).join('_')).filter(k=>parts.some(p=>p.key===k));
      let start=0;while(start<chain.length-1 && parts.filter(p=>p.parent===(start?chain[start-1]:null)).length===1)start++;
      const current=getActivePart(PM,layerId),at=current?chain.indexOf(current):-1;
      return chain[at>=0?Math.min(chain.length-1,at+1):start] ?? null;
    },
    lights:(()=>{
      const tool=()=>lightToolOf(PM),selected=()=>(PM.sel?.layers || []).find((id:string)=>PM.L?.(id)?.d?.data?.light) ?? null;
      return {
        getTool:()=>getLightTool(PM),setTool:patch=>{if(patch.on)setNavigationMode(PM,'select');setLightTool(PM,patch);},onToolChange:listener=>({dispose:onLightTool(PM,listener)}),
        selected,info:()=>lightInfo(PM,selected()),surface:(u,v)=>tool().surface(u,v),
        begin:(u,v)=>{
          const s=getLightTool(PM),id=tool().begin(selected(),u,v,s.aim,s.kind,s.adding);
          if(id && s.adding)setLightTool(PM,{adding:false});
          return id;
        },
        move:(u,v)=>tool().move(u,v),end:()=>tool().end(),cancel:()=>tool().cancel(),active:()=>tool().active(),
        set:(what,value)=>{const id=selected();return id?tool().set(id,what,value):null;},commit:()=>tool().commit(),
      };
    })(),
    studio:{materials:MATERIAL_PRESETS,shots:SHOT_TYPES,progress:()=>studioProgress(PM),
      health:()=>{const status=PM.Studio?.status?.();return {failure:status?.failure?String(status.failure):null,fallback:status?.declined?String(status.declined):null};},
      getPreview:()=>studioPreview(PM),setPreview:q=>setStudioPreview(PM,q),onPreviewChange:listener=>({dispose:onStudioPreview(PM,listener)}),
      swatch:(id)=>{const preset=MATERIAL_PRESETS.find(m=>m.id===id);return preset?studioSwatch(id,preset.p,()=>!!(PM.playing||PM.interactionActive?.()||viewerService(PM)?.isNavigating?.()||PM.agentFrameCapture||PM.Export?.busy||PM.Preview?.preparing||PM.Preview?.active)):Promise.resolve(null);},
      skyThumbnail:(sky,size)=>skyThumbnail(sky,size),
      highlightAt:(u,v)=>{
        const runtime=compositionRuntime(PM,PM.time),camera=PM._renderCamera3D || runtime.camera;
        const hit=surfaceAt(runtime.scene,camera,u,v);
        if(!hit)return null;
        const result:any=editScene(PM,{operation:'light_highlight',at:hit.point,normal:hit.normal},{origin:'canvas',label:'Light by pointing'});
        if(!result.ok){PM.toast?.(result.message,{error:true});return null;}
        return PM.proj.layers.filter((l:any)=>l.d?.data?.light?.type==='area').at(-1)?.id ?? null;
      },
      slideHighlight:(lightId,u,v)=>{
        const light=PM.L(lightId),data=light?.d?.data?.light;
        if(data?.type!=='area')return;
        const runtime=compositionRuntime(PM,PM.time),camera=PM._renderCamera3D || runtime.camera;
        const hit=surfaceAt(runtime.scene,camera,u,v);
        if(!hit)return;
        // Keep the softbox's distance from the surface; remember the start for the commit.
        const p=light.p,start=highlightDrags.get(lightId) ?? {x:p['position.x'].v,y:p['position.y'].v,z:p['position.z'].v,tx:data.p.targetX.v,ty:data.p.targetY.v,tz:data.p.targetZ.v};
        highlightDrags.set(lightId,start);
        const at=new THREE.Vector3(...hit.point),distance=new THREE.Vector3(p['position.x'].v,p['position.y'].v,p['position.z'].v).distanceTo(new THREE.Vector3(data.p.targetX.v,data.p.targetY.v,data.p.targetZ.v));
        const eye=new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld);
        const place=highlightPlacement(at,new THREE.Vector3(...hit.normal),eye,distance);
        p['position.x'].v=place.x;p['position.y'].v=place.y;p['position.z'].v=place.z;
        data.p.targetX.v=at.x;data.p.targetY.v=at.y;data.p.targetZ.v=at.z;
        PM.touch?.();PM.invalidate?.();
      },
      commitHighlight:(lightId)=>{
        const start=highlightDrags.get(lightId);highlightDrags.delete(lightId);
        const light=PM.L(lightId),data=light?.d?.data?.light;
        if(!start || !light || !data)return;
        const p=light.p,end={x:p['position.x'].v,y:p['position.y'].v,z:p['position.z'].v,targetX:data.p.targetX.v,targetY:data.p.targetY.v,targetZ:data.p.targetZ.v};
        // Restore, then apply the whole move as one edit.
        p['position.x'].v=start.x;p['position.y'].v=start.y;p['position.z'].v=start.z;
        data.p.targetX.v=start.tx;data.p.targetY.v=start.ty;data.p.targetZ.v=start.tz;
        const result=editScene(PM,{operation:'update_light',target:lightId,patch:{p:end}},{origin:'canvas',label:'Move highlight'});
        if(!result.ok)PM.toast?.(result.message,{error:true});
      }},
    selection,select};
}
