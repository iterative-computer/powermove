import {setViewAxis,setViewportMode,getViewportMode,toggleViewProjection,viewportState,viewLabel,activeCameraLayer,type ViewAxis} from './viewport';
import {frameComposition,alignCameraToView,setActiveCamera} from './navigation';
import {viewportPreferences,setViewportPreferences,selected3D,type ViewportPreferences} from './editor-state';
import {viewportController,hasScene3D} from './controller';
import {renderState,setShading,VIEWPORT_SHADINGS,type ViewportShading} from './rendering';
import {applyLightingPreset,LIGHTING_PRESETS} from './lighting';
import {layer3DRole} from './layers';

/*
 * The 3D view's few operations, by name, for the viewer's view menu, the
 * command palette and extensions. An operation returns false when it does not
 * apply.
 */

export type ViewportSnapshot=ViewportPreferences&{
  hasScene:boolean;shading:ViewportShading;
  view:{mode:'camera'|'editor';projection:'perspective'|'orthographic';axis:ViewAxis|null;label:string;fov:number};
  selected:string[];hasCamera:boolean;dragging:boolean;
};
export function viewportSnapshot(PM:any):ViewportSnapshot {
  const view=viewportState(PM);
  return {...viewportPreferences(PM),hasScene:hasScene3D(PM),shading:renderState(PM).shading,view:{...view,label:viewLabel(PM)},
    selected:selected3D(PM),hasCamera:!!activeCameraLayer(PM),dragging:!!viewportController(PM)?.active()};
}

export const VIEWPORT_OPERATORS=[
  'view.camera','view.free','view.front','view.back','view.right','view.left','view.top','view.bottom','view.projection',
  'view.selected','view.all','view.align-camera','view.set-active-camera','view.lock-camera',
  'shading','overlays.toggle','select.box','lighting.preset'
] as const;
export type ViewportOperator=typeof VIEWPORT_OPERATORS[number];

export function runViewportOperator(PM:any,name:string,args:unknown[]=[]):boolean {
  const scene=hasScene3D(PM),first=args[0];
  switch(name){
    case 'view.camera':if(!scene)return false;setViewportMode(PM,'camera');return true;
    case 'view.free':if(!scene)return false;setViewportMode(PM,'editor');return true;
    case 'view.front':case 'view.back':case 'view.right':case 'view.left':case 'view.top':case 'view.bottom':
      if(!scene)return false;setViewAxis(PM,name.slice(5) as ViewAxis);return true;
    case 'view.projection':if(!scene)return false;toggleViewProjection(PM);return true;
    case 'view.selected':return scene&&frameComposition(PM,true);
    case 'view.all':return scene&&frameComposition(PM,false);
    case 'view.align-camera':return scene&&alignCameraToView(PM);
    case 'view.set-active-camera':{
      const id=typeof first==='string'?first:selected3D(PM).find(id=>layer3DRole(PM.L(id))==='camera');
      if(!id)return false;setActiveCamera(PM,id);setViewportMode(PM,'camera');return true;
    }
    case 'view.lock-camera':setViewportPreferences(PM,{lockCamera:typeof first==='boolean'?first:!viewportPreferences(PM).lockCamera});return true;
    case 'shading':{if(!VIEWPORT_SHADINGS.includes(first as ViewportShading))return false;setShading(PM,first as ViewportShading);return true;}
    case 'overlays.toggle':setViewportPreferences(PM,{overlays:typeof first==='boolean'?first:!viewportPreferences(PM).overlays});return true;
    case 'select.box':{
      // The viewer's marquee also picks up light and camera wires inside the box.
      const {ids,mode,rect}=(first||{}) as {ids?:string[];mode?:'set'|'add'|'sub';rect?:{x0:number;y0:number;x1:number;y1:number}};
      const wires=rect?viewportController(PM)?.glyphsIn(rect)??[]:[],chosen=[...new Set([...(ids||[]),...wires])];
      const current:string[]=[...(PM.sel?.layers||[])];
      PM.selectLayers?.(mode==='add'?[...new Set([...current,...chosen])]:mode==='sub'?current.filter(id=>!chosen.includes(id)):chosen);
      PM.invalidate?.();return true;
    }
    case 'lighting.preset':return applyLightingPreset(PM,String(first)).length>0;
  }
  throw new Error(`Unknown 3D view operation "${name}"`);
}
export {LIGHTING_PRESETS};
