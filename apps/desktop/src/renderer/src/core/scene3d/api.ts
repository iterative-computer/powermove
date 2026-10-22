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
    selection,select};
}
