import * as THREE from 'three';
import {parent3D} from '../../legacy/core/space-3d';
import {compositionRuntime} from './service';
import {layer3DRole} from './layers';

export type ViewCamera=THREE.PerspectiveCamera|THREE.OrthographicCamera;
export type ViewPose={position:THREE.Vector3;target:THREE.Vector3;zoom:number};
export type ViewMode='camera'|'editor';
type ViewState={camera:ViewCamera|null;target:THREE.Vector3;mode:ViewMode};
const states=new WeakMap<object,Map<string,ViewState>>();
const revisions=new WeakMap<object,number>();
const listeners=new WeakMap<object,Set<(mode:ViewMode)=>void>>();
const composition=(PM:any)=>PM.curComp?.()||PM.proj;
function state(PM:any):ViewState {
  let comps=states.get(PM);if(!comps){comps=new Map();states.set(PM,comps);}
  const comp=composition(PM),key=`${PM.proj.id}:${comp.compId||comp.id}`;let entry=comps.get(key);
  if(!entry){entry={camera:null,target:new THREE.Vector3(),mode:'camera'};comps.set(key,entry);}return entry;
}
export const getViewportMode=(PM:any):ViewMode=>state(PM).mode;
export const viewportRevision=(PM:any):number=>revisions.get(PM)||0;
/* Every mode and pose change notifies `onViewportChange` listeners (the
   kernel's `scene3d.onViewChange`): the native engine's viewer re-sends its
   editor camera on each (extensions/viewer/viewer.ts). */
function changed(PM:any){revisions.set(PM,viewportRevision(PM)+1);for(const listener of listeners.get(PM)||[])listener(getViewportMode(PM));PM.invalidate?.('render');}
export function onViewportChange(PM:any,listener:(mode:ViewMode)=>void):()=>void {
  let set=listeners.get(PM);if(!set){set=new Set();listeners.set(PM,set);}set.add(listener);
  const refresh=()=>listener(getViewportMode(PM));
  const offs=['project','composition'].map(event=>PM.bus?.on?.(event,refresh));
  return()=>{set!.delete(listener);for(const off of offs)if(typeof off==='function')off();};
}
function initialize(PM:any,entry:ViewState):void {
  const world=compositionRuntime(PM),layer=composition(PM).layers.find((l:any)=>layer3DRole(l)==='camera'&&PM.active(l,PM.time));
  entry.camera=world.camera.clone();
  entry.target=layer?new THREE.Vector3(...['X','Y','Z'].map(axis=>PM.evP(layer,layer.d.data.camera.p[`target${axis}`],PM.time,`camera.target${axis}`)) as [number,number,number])
    .applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,layer,PM.time))):new THREE.Vector3();
}
/** The editor camera is transient. Render/export callers keep the runtime camera. */
export function viewportCamera(PM:any,renderCamera:ViewCamera):ViewCamera {
  const entry=state(PM);return entry.mode==='editor'&&entry.camera?entry.camera:renderCamera;
}
/** The editor camera as the native engine's view override takes it
 *  (contract 2 `EditorCamera`): the world-to-camera matrix, column-major,
 *  and a perspective camera's vertical field of view with three.js's zoom
 *  folded in (the engine's editor camera has no zoom), or an orthographic
 *  camera's half-height. Null in the Camera view. */
export type EngineEditorCamera={
  view:number[];
  projection:{kind:'perspective';fovYDegrees:number;near:number;far:number}|{kind:'orthographic';halfHeight:number;near:number;far:number};
};
export function viewportEngineCamera(PM:any):EngineEditorCamera|null {
  const entry=state(PM),camera=entry.camera;
  if(entry.mode!=='editor'||!camera)return null;
  camera.updateMatrixWorld(true);
  const view=new THREE.Matrix4().copy(camera.matrixWorld).invert().toArray();
  const zoom=camera.zoom>0?camera.zoom:1;
  if((camera as THREE.OrthographicCamera).isOrthographicCamera){
    const o=camera as THREE.OrthographicCamera;
    return {view,projection:{kind:'orthographic',halfHeight:(o.top-o.bottom)/(2*zoom),near:o.near,far:o.far}};
  }
  const p=camera as THREE.PerspectiveCamera;
  const fovYDegrees=THREE.MathUtils.radToDeg(2*Math.atan(Math.tan(THREE.MathUtils.degToRad(p.fov)/2)/zoom));
  return {view,projection:{kind:'perspective',fovYDegrees,near:p.near,far:p.far}};
}
export function viewportPose(PM:any):ViewPose {
  const entry=state(PM);if(entry.mode==='camera'||!entry.camera)initialize(PM,entry);
  return {position:entry.camera!.position.clone(),target:entry.target.clone(),zoom:entry.camera!.zoom};
}
export function setViewportMode(PM:any,mode:ViewMode):void {
  if(mode!=='camera'&&mode!=='editor')throw new Error('Unknown 3D view');
  const entry=state(PM);if(mode==='editor'&&!entry.camera)initialize(PM,entry);entry.mode=mode;changed(PM);
}
export function applyViewportPose(PM:any,pose:ViewPose):void {
  const entry=state(PM);if(!entry.camera||entry.mode==='camera')initialize(PM,entry);
  entry.mode='editor';entry.target.copy(pose.target);
  entry.camera!.position.copy(pose.position);entry.camera!.zoom=pose.zoom;
  entry.camera!.lookAt(pose.target);entry.camera!.updateProjectionMatrix();entry.camera!.updateMatrixWorld(true);changed(PM);
}
