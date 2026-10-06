import {rollView,enterLocalView,exitLocalView,localViewIds,setViewAxis,setViewportMode,getViewportMode,toggleViewProjection,orbitViewStep,applyViewportPose,viewportPose,viewportState,viewLabel,activeCameraLayer,setViewFov,type ViewAxis} from './viewport';
import {frameComposition,dollyView,alignCameraToView,setActiveCamera,lockedCamera,writeCameraPose} from './navigation';
import {viewportPreferences,setViewportPreferences,activeObject,cursorLocation,setCursorLocation,selected3D,VIEWPORT_TOOLS,VIEWPORT_ORIENTATIONS,VIEWPORT_PIVOTS,type ViewportTool,type ViewportPreferences} from './editor-state';
import {selectAll,selectByType,clickSelect,boxSelect,deleteSelected,duplicateSelected,hideSelected,revealHidden,clearTransform,insertKeyframes,deleteKeyframes,parentToActive,clearParent,snap,type SelectAction,type SnapOperation} from './operators';
import {viewportController,inViewportContext,hasScene3D} from './controller';
import {renderState,setShading,VIEWPORT_SHADINGS,type ViewportShading} from './rendering';
import {editModeLayer,editSummary,toggleEditMode,selectAllPoints,deleteSelectedPoints,mergeSelectedPoints} from './editmode';

/*
 * Every Blender viewport operator by name. Keys, header menus, pies and the
 * toolbar all run these. An operator returns false when it does not apply,
 * which lets a key fall through to the editor's own shortcut.
 */

export type ViewportSnapshot=ViewportPreferences&{
  hasScene:boolean;inContext:boolean;shading:ViewportShading;
  view:{mode:'camera'|'editor';projection:'perspective'|'orthographic';axis:ViewAxis|null;label:string;fov:number};
  cursor:[number,number,number];active:string|null;selected:string[];modal:boolean;hasCamera:boolean;
  editMode:{layer:string;selected:number;total:number}|null;
};
export function viewportSnapshot(PM:any):ViewportSnapshot {
  const view=viewportState(PM),cursor=cursorLocation(PM);
  return {...viewportPreferences(PM),hasScene:hasScene3D(PM),inContext:inViewportContext(PM),shading:renderState(PM).shading,
    view:{...view,label:viewLabel(PM)},cursor:[cursor.x,cursor.y,cursor.z],active:activeObject(PM),selected:selected3D(PM),
    modal:!!viewportController(PM)?.modalActive(),hasCamera:!!activeCameraLayer(PM),editMode:editSummary(PM)};
}

const SELECT_TOOLS:ViewportTool[]=['tweak','box'];
export const VIEWPORT_OPERATORS=[
  'view.camera','view.front','view.back','view.right','view.left','view.top','view.bottom','view.projection','view.orbit','view.flip','view.walk','view.roll','view.zoom-border','view.local',
  'view.selected','view.all','view.zoom','view.align-camera','view.set-active-camera','view.cursor-reset','view.lock-camera','view.fov',
  'select.all','select.none','select.invert','select.toggle','select.type','select.click','select.box','select.box-arm','xray.toggle',
  'transform.translate','transform.rotate','transform.scale','transform.trackball','transform.clear',
  'object.delete','object.duplicate','object.hide','object.hide-unselected','object.reveal','object.keyframe-insert','object.keyframe-delete','object.parent','object.parent-clear','edit.toggle','edit.extrude','edit.merge',
  'snap','cursor.set','tool','tool.cycle-select','shading','overlays.toggle','toolbar.toggle','sidebar.toggle','snapping.toggle','orientation','pivot'
] as const;
export type ViewportOperator=typeof VIEWPORT_OPERATORS[number];

export function runViewportOperator(PM:any,name:string,args:unknown[]=[]):boolean {
  const scene=hasScene3D(PM),first=args[0];
  // Edit Mode: selection and deletion act on points; object operators do not apply.
  if(editModeLayer(PM))switch(name){
    case 'select.all':return selectAllPoints(PM,(first as SelectAction)||'select');
    case 'select.none':return selectAllPoints(PM,'deselect');
    case 'select.invert':return selectAllPoints(PM,'invert');
    case 'select.toggle':return selectAllPoints(PM,'toggle');
    case 'object.delete':return deleteSelectedPoints(PM);
    case 'select.type':case 'select.click':case 'select.box':case 'select.box-arm':case 'transform.clear':case 'object.duplicate':case 'object.hide':case 'object.hide-unselected':
    case 'object.reveal':case 'object.keyframe-insert':case 'object.keyframe-delete':case 'object.parent':case 'object.parent-clear':case 'snap':case 'view.local':
      return false;
  }
  switch(name){
    case 'edit.toggle':return scene&&toggleEditMode(PM);
    case 'edit.extrude':return !!editModeLayer(PM)&&!!viewportController(PM)?.extrude();
    case 'edit.merge':return mergeSelectedPoints(PM);
    /* view */
    case 'view.camera':if(!scene)return false;setViewportMode(PM,getViewportMode(PM)==='camera'?'editor':'camera');return true;
    case 'view.front':case 'view.back':case 'view.right':case 'view.left':case 'view.top':case 'view.bottom':
      if(!scene)return false;setViewAxis(PM,name.slice(5) as ViewAxis);return true;
    case 'view.projection':if(!scene)return false;toggleViewProjection(PM);return true;
    case 'view.orbit':{if(!scene)return false;const [yaw,pitch]=args as [number,number];orbitViewStep(PM,Number(yaw)||0,Number(pitch)||0);return true;}
    case 'view.roll':if(!scene)return false;rollView(PM,Number(first)||0);return true;
    case 'view.zoom-border':return scene&&!!viewportController(PM)?.armZoomBorder();
    case 'view.local':{
      if(!scene)return false;
      if(localViewIds(PM)){exitLocalView(PM);return true;}
      const ids=selected3D(PM).flatMap(id=>{const layer=PM.L(id);return layer?.type==='group'?(PM.curComp?.()||PM.proj).layers.filter((l:any)=>(PM.groupAncestors?.(l)||[]).some((g:any)=>g.id===id)).map((l:any)=>l.id):[id];});
      if(!enterLocalView(PM,ids))return false;frameComposition(PM,true);return true;
    }
    case 'view.walk':return scene&&!!viewportController(PM)?.startWalk();
    case 'view.flip':if(!scene)return false;orbitViewStep(PM,180,0);return true;
    case 'view.selected':return scene&&frameComposition(PM,true);
    case 'view.all':return scene&&frameComposition(PM,false);
    case 'view.zoom':{
      if(!scene)return false;const delta=Number(first)||0;
      if(getViewportMode(PM)==='camera'&&!lockedCamera(PM))return false;
      const pose=dollyView(viewportPose(PM),delta,.002);
      if(lockedCamera(PM))writeCameraPose(PM,pose,'View camera');else applyViewportPose(PM,pose,{axis:viewportState(PM).axis});
      return true;
    }
    case 'view.align-camera':return scene&&alignCameraToView(PM);
    case 'view.set-active-camera':{
      const id=activeObject(PM);if(!id)return false;const result=setActiveCamera(PM,id);setViewportMode(PM,'camera');return result;
    }
    case 'view.cursor-reset':if(!scene)return false;setCursorLocation(PM,[0,0,0]);frameComposition(PM,false);return true;
    case 'view.lock-camera':setViewportPreferences(PM,{lockCamera:typeof first==='boolean'?first:!viewportPreferences(PM).lockCamera});return true;
    case 'view.fov':setViewFov(PM,Number(first));return true;
    /* select */
    case 'select.all':return scene&&selectAll(PM,(first as SelectAction)||'select');
    case 'select.none':return selectAll(PM,'deselect');
    case 'select.invert':return scene&&selectAll(PM,'invert');
    case 'select.toggle':return scene&&selectAll(PM,'toggle');
    case 'select.type':return scene&&selectByType(PM,first as 'object'|'light'|'camera',args[1]!==false);
    case 'select.click':clickSelect(PM,(first as string)||null,!!args[1]);return true;
    case 'select.box-arm':return !!viewportController(PM)?.armBoxSelect();
    case 'select.box':{
      const {ids,mode,rect}=(first||{}) as {ids?:string[];mode?:'set'|'add'|'sub';rect?:{x0:number;y0:number;x1:number;y1:number}};
      const wires=rect?viewportController(PM)?.glyphsIn(rect)??[]:[];
      boxSelect(PM,[...new Set([...(ids||[]),...wires])],mode||'set');return true;
    }
    /* transform */
    case 'transform.translate':case 'transform.rotate':case 'transform.scale':case 'transform.trackball':
      return !!viewportController(PM)?.startTransform(name.slice(10) as 'translate'|'rotate'|'scale'|'trackball');
    case 'transform.clear':return clearTransform(PM,(first as 'translate'|'rotate'|'scale')||'translate');
    /* object */
    case 'object.delete':return deleteSelected(PM);
    case 'object.duplicate':{
      if(!duplicateSelected(PM).length)return false;
      if(args[0]!==false)viewportController(PM)?.startTransform('translate');
      return true;
    }
    case 'object.hide':return hideSelected(PM,false);
    case 'object.hide-unselected':return hideSelected(PM,true);
    case 'object.reveal':return revealHidden(PM);
    case 'object.keyframe-insert':return insertKeyframes(PM);
    case 'object.keyframe-delete':return deleteKeyframes(PM);
    case 'object.parent':return parentToActive(PM);
    case 'object.parent-clear':return clearParent(PM);
    case 'snap':return snap(PM,first as SnapOperation);
    case 'cursor.set':{const point=first as [number,number,number];setCursorLocation(PM,point);return true;}
    /* editor settings */
    case 'tool':{if(!VIEWPORT_TOOLS.includes(first as ViewportTool))return false;setViewportPreferences(PM,{tool:first as ViewportTool});return true;}
    case 'tool.cycle-select':{
      const current=viewportPreferences(PM).tool,index=SELECT_TOOLS.indexOf(current);
      setViewportPreferences(PM,{tool:SELECT_TOOLS[(index+1)%SELECT_TOOLS.length]!});return true;
    }
    case 'shading':{if(!VIEWPORT_SHADINGS.includes(first as ViewportShading))return false;setShading(PM,first as ViewportShading);return true;}
    case 'xray.toggle':setViewportPreferences(PM,{xray:!viewportPreferences(PM).xray});return true;
    case 'overlays.toggle':setViewportPreferences(PM,{overlays:!viewportPreferences(PM).overlays});return true;
    case 'toolbar.toggle':setViewportPreferences(PM,{toolbar:!viewportPreferences(PM).toolbar});return true;
    case 'sidebar.toggle':setViewportPreferences(PM,{sidebar:!viewportPreferences(PM).sidebar});return true;
    case 'snapping.toggle':setViewportPreferences(PM,{snap:!viewportPreferences(PM).snap});return true;
    case 'orientation':{if(!VIEWPORT_ORIENTATIONS.includes(first as any))return false;setViewportPreferences(PM,{orientation:first as any});return true;}
    case 'pivot':{if(!VIEWPORT_PIVOTS.includes(first as any))return false;setViewportPreferences(PM,{pivot:first as any});return true;}
  }
  throw new Error(`Unknown 3D viewport operator "${name}"`);
}
