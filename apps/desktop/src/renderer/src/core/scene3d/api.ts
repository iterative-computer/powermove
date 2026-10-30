import * as THREE from 'three';
import { MATERIAL_PRESETS, SHOT_TYPES, surfaceAt, highlightPlacement } from '../studio/operations';

/** Softboxes being dragged by light by pointing, with where each started. */
const highlightDrags = new Map<string,{x:number;y:number;z:number;tx:number;ty:number;tz:number}>();
import { studioProgress } from '../studio';
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

export function makeScene3DAPI(PM:any,emit:(event:'scene3d:selection',selection:{layerId:string|null;ids:string[]})=>void):Scene3DAPI {
  let navigation:SceneNavigation|null=null;
  const selection=()=>{
    const ids=(PM.sel?.layers || []).filter((id:string)=>layer3DRole(PM.L?.(id)) || isModelGroup(PM,PM.L?.(id)));
    return {layerId:ids[0] || null,ids};
  };
  const select=(layerId:string|null,ids:string[])=>{
    const selected=ids.length?ids:layerId?[layerId]:[];
    PM.selectLayers?.(selected.filter(id=>layer3DRole(PM.L?.(id)) || isModelGroup(PM,PM.L?.(id))));
    emit('scene3d:selection',selection());PM.bus?.emit?.('scene3d:selection');PM.invalidate?.();
  };
  return {getMode:getGizmoMode,setMode:setGizmoMode,getSpace:getGizmoSpace,setSpace:setGizmoSpace,onGizmoChange:listener=>({dispose:onGizmoState(listener)}),createGizmo:element=>createSceneGizmo(PM,element,time=>compositionRuntime(PM,time)),createScene,createObject,createLight,prepareImport:prepareModelImport,convert:layerId=>convertLegacyScene(PM,layerId),edit:(args,meta)=>editScene(PM,args,{origin:'interface',...meta}),
    isGroup:layerId=>isModelGroup(PM,PM.L?.(layerId)),
    getView:()=>getViewportMode(PM),setView:mode=>{navigation?.cancel();if(mode==='camera')setNavigationMode(PM,'select');setViewportMode(PM,mode);},onViewChange:listener=>({dispose:onViewportChange(PM,listener)}),
    getNavigationMode:()=>getNavigationMode(PM),setNavigationMode:mode=>setNavigationMode(PM,mode),
    onNavigationChange:listener=>({dispose:onNavigationMode(PM,listener)}),
    createNavigation:element=>(navigation=createSceneNavigation(PM,element)),
    frame:selected=>{try{navigation?navigation.frame(selected):frameComposition(PM,selected);}catch(error){PM.toast?.(error instanceof Error?error.message:String(error),{key:navigationNotice,error:true});}},
    describe(){return compositionScene(PM,PM.time);},
    studio:{materials:MATERIAL_PRESETS,shots:SHOT_TYPES,progress:()=>studioProgress(PM),
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
