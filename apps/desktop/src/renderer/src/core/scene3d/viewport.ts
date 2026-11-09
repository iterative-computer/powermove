import * as THREE from 'three';
import {parent3D} from '../../legacy/core/space-3d';
import {compositionRuntime} from './service';
import {layer3DRole} from './layers';

export type ViewCamera=THREE.PerspectiveCamera|THREE.OrthographicCamera;
export type ViewPose={position:THREE.Vector3;target:THREE.Vector3;zoom:number};
export type ViewMode='camera'|'editor';
type ViewState={camera:ViewCamera|null;target:THREE.Vector3;mode:ViewMode;side:ViewSide|null};
const states=new WeakMap<object,Map<string,ViewState>>();
const revisions=new WeakMap<object,number>();
const listeners=new WeakMap<object,Set<(mode:ViewMode)=>void>>();
const composition=(PM:any)=>PM.curComp?.()||PM.proj;
function state(PM:any):ViewState {
  let comps=states.get(PM);if(!comps){comps=new Map();states.set(PM,comps);}
  const comp=composition(PM),key=`${PM.proj.id}:${comp.compId||comp.id}`;let entry=comps.get(key);
  if(!entry){entry={camera:null,target:new THREE.Vector3(),mode:'camera',side:null};comps.set(key,entry);}return entry;
}
export const getViewportMode=(PM:any):ViewMode=>state(PM).mode;
export const viewportRevision=(PM:any):number=>revisions.get(PM)||0;
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
  entry.mode='editor';entry.side=null;entry.target.copy(pose.target);
  entry.camera!.position.copy(pose.position);entry.camera!.zoom=pose.zoom;
  entry.camera!.lookAt(pose.target);entry.camera!.updateProjectionMatrix();entry.camera!.updateMatrixWorld(true);changed(PM);
}

export type ViewSide='front'|'back'|'left'|'right'|'top'|'bottom';
const SIDES:Record<ViewSide,[number,number,number]>={front:[0,0,1],back:[0,0,-1],left:[-1,0,0],right:[1,0,0],top:[0,1,0],bottom:[0,-1,0]};
/** After Effects' Front, Left, Top… views: the Free View looks straight along
    an axis at what it was looking at, from the same distance. The shot's
    camera is unchanged. */
export function lookFrom(PM:any,side:ViewSide):void {
  const axis=SIDES[side];if(!axis)throw new Error('Unknown view side');
  const pose=viewportPose(PM),distance=Math.max(.01,pose.position.distanceTo(pose.target));
  // Straight up/down needs a hair of depth so the view keeps a stable "up".
  const direction=new THREE.Vector3(...axis);if(side==='top'||side==='bottom')direction.z=1e-4;
  applyViewportPose(PM,{target:pose.target,position:pose.target.clone().addScaledVector(direction.normalize(),distance),zoom:pose.zoom});
  state(PM).side=side;changed(PM);
}
/** The axis the Free View looks along (Front, Top…), until it is orbited; null for the camera or a custom view. */
export const viewportSide=(PM:any):ViewSide|null=>{const entry=state(PM);return entry.mode==='editor'?entry.side:null;};
/** Put the Free View at a pose that looks along an axis, and remember that axis. */
export function applySidePose(PM:any,side:ViewSide,pose:ViewPose):void {applyViewportPose(PM,pose);state(PM).side=side;changed(PM);}
export const sideDirection=(side:ViewSide):THREE.Vector3=>{const d=new THREE.Vector3(...SIDES[side]);if(side==='top'||side==='bottom')d.z=1e-4;return d.normalize();};
export const VIEW_SIDES=Object.keys(SIDES) as ViewSide[];
/** Key of the open composition's view state. */
export const viewportKey=(PM:any):string=>{const comp=composition(PM);return `${PM.proj.id}:${comp.compId||comp.id}`;};
