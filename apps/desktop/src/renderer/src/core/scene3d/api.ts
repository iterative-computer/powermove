import {bridge} from '../../kernel/bridge';
import {editModel,MODEL_RECIPES,MODEL_FIELDS,modelFieldsFor} from './modeling';
import {materialPreset,MATERIAL_PRESETS,listMaterials,materialSlots,assignedMaterial} from './materials';
import {renderSettings,setRenderSettings,blenderStatus,renderState,setPreviewMode,onRenderState,renderBlenderPreview,cancelBlender} from './rendering';
import {getViewportMode,setViewportMode,onViewportChange} from './viewport';
import {onViewportSettings,notifyViewport} from './editor-state';
import {attachViewport,inViewportContext} from './controller';
import {runViewportOperator,viewportSnapshot,VIEWPORT_OPERATORS} from './commands';
import {frameComposition,navigationNotice} from './navigation';
import { createScene,createObject,createLight,parseScene,SCENE3D_DEFINITION } from './schema';
import { convertLegacyScene } from './migration';
import { prepareModelImport } from './import-bundle';
import { compositionScene,layer3DRole,isModelGroup } from './layers';
import { editScene } from './operations';
import type { Scene3DAPI } from '../../kernel/api';

const warn=(PM:any,error:unknown)=>PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:navigationNotice,error:true});
/** Coalesced viewport state notifications for header, toolbar and sidebar UI. */
function onViewportState(PM:any,listener:(state:ReturnType<typeof viewportSnapshot>)=>void):()=>void {
  let queued=false,live=true;
  const fire=()=>{if(queued||!live)return;queued=true;queueMicrotask(()=>{queued=false;if(live)listener(viewportSnapshot(PM));});};
  const offs=[onViewportSettings(PM,fire),onViewportChange(PM,fire),onRenderState(PM,fire),
    ...['sel','selection','scene3d:selection','scene3d:viewport','project','composition','layers'].map(event=>PM.bus?.on?.(event,fire))];
  return()=>{live=false;for(const off of offs)if(typeof off==='function')off();};
}
export function makeScene3DAPI(PM:any,emit:(event:'scene3d:selection',selection:{layerId:string|null;ids:string[]})=>void):Scene3DAPI {
  const selection=()=>{
    const ids=(PM.sel?.layers || []).filter((id:string)=>layer3DRole(PM.L?.(id)) || isModelGroup(PM,PM.L?.(id)));
    return {layerId:ids[0] || null,ids};
  };
  const select=(layerId:string|null,ids:string[])=>{
    const selected=ids.length?ids:layerId?[layerId]:[];
    PM.selectLayers?.(selected.filter(id=>layer3DRole(PM.L?.(id)) || isModelGroup(PM,PM.L?.(id))));
    emit('scene3d:selection',selection());PM.bus?.emit?.('scene3d:selection');PM.invalidate?.();
  };
  return {model:(args,meta)=>editModel(PM,args,{origin:'interface',...meta}),modelRecipes:MODEL_RECIPES,modelFields:MODEL_FIELDS,modelFieldsFor,materialPresets:MATERIAL_PRESETS,createMaterial:materialPreset,materials:()=>listMaterials(PM),
    setMaterial(target,slotId,material,shared=false){
      const layer=PM.L(target),slot=materialSlots(layer).find(s=>s.id===slotId);if(!slot)return {ok:false,message:'Choose a material slot'};
      const source=(PM.curComp?.()||PM.proj).layers.find((candidate:any)=>materialSlots(candidate).some(s=>s.material===material))||layer;
      const matches=shared?(PM.curComp?.()||PM.proj).layers.filter((l:any)=>materialSlots(l).some(s=>s.material.shader?.id===slot.material.shader?.id)):[layer];
      const commands=matches.map((l:any)=>{const data=JSON.parse(JSON.stringify(l.d.data)),object=data.object;if(object.slots?.length)object.slots=object.slots.map((s:any)=>(!shared&&l===layer?s.id===slotId:s.material.shader?.id===slot.material.shader?.id)?{...s,material:assignedMaterial(PM,material,source,l,s.material)}:s);else object.material=assignedMaterial(PM,material,source,l,object.material);return {type:'set_content',target:l.id,patch:{data}};});
      return PM.Edit.apply(commands,{origin:'inspector',label:'Edit material'});
    },
    getRendering:()=>renderSettings(PM.curComp?.()||PM.proj),setRendering:(patch,meta)=>setRenderSettings(PM,patch,meta),blenderStatus,chooseBlender:()=>bridge()?.blender?.choose()||Promise.resolve(null),
    previewState:()=>renderState(PM),setPreviewMode:mode=>setPreviewMode(PM,mode),onPreviewChange:listener=>({dispose:onRenderState(PM,listener)}),renderFrame:()=>renderBlenderPreview(PM),cancelRender:()=>cancelBlender(PM),
    getAutoKey:()=>!!PM.autokey,setAutoKey:value=>{PM.autokey=!!value;notifyViewport(PM);},createScene,createObject,createLight,prepareImport:prepareModelImport,convert:layerId=>convertLegacyScene(PM,layerId),edit:(args,meta)=>editScene(PM,args,{origin:'interface',...meta}),
    isGroup:layerId=>isModelGroup(PM,PM.L?.(layerId)),
    getView:()=>getViewportMode(PM),setView:mode=>setViewportMode(PM,mode),onViewChange:listener=>({dispose:onViewportChange(PM,listener)}),
    frame:selected=>{try{frameComposition(PM,selected);}catch(error){warn(PM,error);}},
    viewport:{
      attach:(stage,host)=>attachViewport(PM,stage,host),
      state:()=>viewportSnapshot(PM),
      onChange:listener=>({dispose:onViewportState(PM,listener)}),
      inContext:()=>inViewportContext(PM),
      operators:VIEWPORT_OPERATORS,
      run(operator,...args){try{return runViewportOperator(PM,operator,args);}catch(error){warn(PM,error);return true;}}
    },
    describe(){return compositionScene(PM,PM.time);},
    selection,select};
}
