import * as THREE from 'three';
import {parent3D} from '../../legacy/core/space-3d';
import {compositionRuntime} from './service';
import {layer3DRole} from './layers';

/*
 * The editor's 3D view, modelled on Blender's region view: a pivot (target),
 * a view rotation and a distance. It is transient per composition and never
 * part of the project; exports and agent captures always use the render camera.
 *
 * Coordinates are Powermove's (Y up). The named views match Blender after the
 * standard Y-up conversion: Front looks along -Z, Right along -X, Top down -Y.
 */

export type ViewCamera=THREE.PerspectiveCamera|THREE.OrthographicCamera;
export type ViewMode='camera'|'editor';
export type ViewProjection='perspective'|'orthographic';
export type ViewAxis='front'|'back'|'right'|'left'|'top'|'bottom';
export type ViewPose={target:THREE.Vector3;rotation:THREE.Quaternion;distance:number};
type UserView={pose:ViewPose;projection:ViewProjection;axis:ViewAxis|null;autoOrtho:boolean};
type ViewState=UserView&{mode:ViewMode;fov:number;persp:THREE.PerspectiveCamera;ortho:THREE.OrthographicCamera;initialized:boolean;last:UserView|null;local:{ids:Set<string>;restore:{mode:ViewMode;view:UserView|null}}|null};

/** Orthographic frustums are ±3 units tall before zoom (see SceneRuntime.resizeCamera). */
const ORTHO_HALF=3;
const X=new THREE.Vector3(1,0,0),Y=new THREE.Vector3(0,1,0);
const axisRotation=(axis:THREE.Vector3,degrees:number)=>new THREE.Quaternion().setFromAxisAngle(axis,THREE.MathUtils.degToRad(degrees));
export const VIEW_AXES:Record<ViewAxis,THREE.Quaternion>={
  front:new THREE.Quaternion(),back:axisRotation(Y,180),right:axisRotation(Y,90),left:axisRotation(Y,-90),
  top:axisRotation(X,-90),bottom:axisRotation(X,90)
};
export const OPPOSITE_VIEW:Record<ViewAxis,ViewAxis>={front:'back',back:'front',right:'left',left:'right',top:'bottom',bottom:'top'};

const states=new WeakMap<object,Map<string,ViewState>>();
const revisions=new WeakMap<object,number>();
const listeners=new WeakMap<object,Set<(mode:ViewMode)=>void>>();
const composition=(PM:any)=>PM.curComp?.()||PM.proj;
const clonePose=(pose:ViewPose):ViewPose=>({target:pose.target.clone(),rotation:pose.rotation.clone(),distance:pose.distance});
const cloneUser=(view:UserView):UserView=>({...view,pose:clonePose(view.pose)});

function state(PM:any):ViewState {
  let comps=states.get(PM);if(!comps){comps=new Map();states.set(PM,comps);}
  const comp=composition(PM),key=`${PM.proj.id}:${comp.compId||comp.id}`;let entry=comps.get(key);
  if(!entry){
    entry={mode:'camera',pose:{target:new THREE.Vector3(),rotation:new THREE.Quaternion(),distance:6},projection:'perspective',axis:null,autoOrtho:false,
      fov:50,persp:new THREE.PerspectiveCamera(50,16/9,.01,1000),ortho:new THREE.OrthographicCamera(-ORTHO_HALF,ORTHO_HALF,ORTHO_HALF,-ORTHO_HALF,.01,1000),initialized:false,last:null,local:null};
    comps.set(key,entry);
  }
  return entry;
}
export const getViewportMode=(PM:any):ViewMode=>state(PM).mode;
export const viewportRevision=(PM:any):number=>revisions.get(PM)||0;
/** Redraw the composition preview for an editor-only view change. */
export const touchViewport=(PM:any)=>changed(PM);
function changed(PM:any){revisions.set(PM,viewportRevision(PM)+1);for(const listener of listeners.get(PM)||[])listener(getViewportMode(PM));PM.invalidate?.('render');}
export function onViewportChange(PM:any,listener:(mode:ViewMode)=>void):()=>void {
  let set=listeners.get(PM);if(!set){set=new Set();listeners.set(PM,set);}set.add(listener);
  const refresh=()=>listener(getViewportMode(PM));
  const offs=['project','composition'].map(event=>PM.bus?.on?.(event,refresh));
  return()=>{set!.delete(listener);for(const off of offs)if(typeof off==='function')off();};
}

/** The active render camera's layer (the topmost active camera), if any. */
export function activeCameraLayer(PM:any,time=PM.time):any|null {
  return composition(PM).layers.find((l:any)=>layer3DRole(l)==='camera'&&(PM.active?PM.active(l,time):true))??null;
}
/** The render camera expressed as a view pose: it orbits around its own aim point. */
export function cameraPose(PM:any):ViewPose {
  const world=compositionRuntime(PM),camera=world.camera,layer=activeCameraLayer(PM);
  camera.updateMatrixWorld(true);
  const position=new THREE.Vector3().setFromMatrixPosition(camera.matrixWorld),rotation=new THREE.Quaternion();
  camera.matrixWorld.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
  let target:THREE.Vector3;
  if(layer){
    target=new THREE.Vector3(...['X','Y','Z'].map(axis=>Number(PM.evP(layer,layer.d.data.camera.p[`target${axis}`],PM.time,`camera.target${axis}`))) as [number,number,number])
      .applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,layer,PM.time)));
  }else target=position.clone().add(new THREE.Vector3(0,0,-1).applyQuaternion(rotation).multiplyScalar(Math.max(1,position.length())));
  return {target,rotation,distance:Math.max(.05,position.distanceTo(target))};
}
function initialize(PM:any,entry:ViewState):void {
  const camera=compositionRuntime(PM).camera;
  entry.pose=cameraPose(PM);entry.axis=null;entry.autoOrtho=false;
  entry.projection=camera instanceof THREE.OrthographicCamera?'orthographic':'perspective';
  entry.fov=camera instanceof THREE.PerspectiveCamera?camera.fov:50;
  entry.initialized=true;
}
export function viewPosition(pose:ViewPose):THREE.Vector3 {
  return new THREE.Vector3(0,0,1).applyQuaternion(pose.rotation).multiplyScalar(pose.distance).add(pose.target);
}
/** Half the visible height at the pivot; shared by both projections so toggling keeps framing. */
export const viewHalfHeight=(pose:ViewPose,fov:number)=>pose.distance*Math.tan(THREE.MathUtils.degToRad(fov)/2);
function configure(entry:ViewState):ViewCamera {
  const {pose}=entry;
  if(entry.projection==='perspective'){
    const camera=entry.persp;camera.fov=entry.fov;camera.zoom=1;
    camera.near=Math.max(.001,pose.distance*.002);camera.far=Math.max(1000,pose.distance*200);
    camera.position.copy(viewPosition(pose));camera.quaternion.copy(pose.rotation);
    camera.updateProjectionMatrix();camera.updateMatrixWorld(true);return camera;
  }
  // Orthographic views look from far behind the pivot so nothing in the scene is clipped.
  const camera=entry.ortho,back=Math.max(200,pose.distance*20);
  camera.zoom=ORTHO_HALF/Math.max(1e-6,viewHalfHeight(pose,entry.fov));camera.near=.01;camera.far=back*2;
  camera.position.copy(new THREE.Vector3(0,0,1).applyQuaternion(pose.rotation).multiplyScalar(back).add(pose.target));
  camera.quaternion.copy(pose.rotation);camera.updateProjectionMatrix();camera.updateMatrixWorld(true);
  return camera;
}
/** The editor camera is transient. Render/export callers keep the runtime camera. */
export function viewportCamera(PM:any,renderCamera:ViewCamera):ViewCamera {
  const entry=state(PM);return entry.mode==='editor'&&entry.initialized?configure(entry):renderCamera;
}
/** The current view as a pose. In camera view this is the shot camera's pose. */
export function viewportPose(PM:any):ViewPose {
  const entry=state(PM);
  return entry.mode==='camera'||!entry.initialized?cameraPose(PM):clonePose(entry.pose);
}
export function viewportState(PM:any):{mode:ViewMode;projection:ViewProjection;axis:ViewAxis|null;fov:number} {
  const entry=state(PM),camera=entry.mode==='camera'?compositionRuntime(PM).camera:null;
  return {mode:entry.mode,projection:camera?camera instanceof THREE.OrthographicCamera?'orthographic':'perspective':entry.projection,axis:entry.mode==='camera'?null:entry.axis,fov:entry.fov};
}
/** Blender's view name, e.g. "User Perspective", "Front Orthographic", "Camera Perspective". */
export function viewLabel(PM:any):string {
  const view=viewportState(PM),projection=view.projection==='orthographic'?'Orthographic':'Perspective';
  if(view.mode==='camera')return `Camera ${projection}`;
  return `${view.axis?view.axis[0]!.toUpperCase()+view.axis.slice(1):'User'} ${projection}${state(PM).local?' (Local)':''}`;
}
/** Camera ⇄ the previous user view (Numpad 0). */
export function setViewportMode(PM:any,mode:ViewMode):void {
  if(mode!=='camera'&&mode!=='editor')throw new Error('Unknown 3D view');
  const entry=state(PM);if(entry.mode===mode){changed(PM);return;}
  if(mode==='camera'){if(entry.initialized)entry.last=cloneUser(entry);}
  else if(entry.last)Object.assign(entry,cloneUser(entry.last),{initialized:true});
  else initialize(PM,entry);
  entry.mode=mode;changed(PM);
}
/** Free navigation always ends in a user view; leaving a named ortho view restores perspective (Auto Perspective). */
export function applyViewportPose(PM:any,pose:ViewPose,options:{axis?:ViewAxis|null}={}):void {
  const entry=state(PM);
  if(!entry.initialized||entry.mode==='camera'){const fov=entry.initialized?entry.fov:null;initialize(PM,entry);if(fov!==null&&entry.mode!=='camera')entry.fov=fov;}
  entry.mode='editor';entry.pose=clonePose(pose);entry.pose.rotation.normalize();entry.pose.distance=Math.max(1e-4,pose.distance);
  const axis=options.axis??null;
  if(!axis&&entry.autoOrtho){entry.projection='perspective';entry.autoOrtho=false;}
  entry.axis=axis;changed(PM);
}
/** Numpad 1/3/7 (Ctrl for the opposite side). Switches to orthographic like Blender's Auto Perspective. */
export function setViewAxis(PM:any,axis:ViewAxis):void {
  if(!VIEW_AXES[axis])throw new Error('Unknown view axis');
  const pose=viewportPose(PM);applyViewportPose(PM,{...pose,rotation:VIEW_AXES[axis].clone()},{axis});
  const entry=state(PM);
  if(entry.projection==='perspective'){entry.projection='orthographic';entry.autoOrtho=true;}
  changed(PM);
}
/** Numpad 5. */
export function toggleViewProjection(PM:any):void {
  const entry=state(PM);
  if(entry.mode==='camera'){const pose=viewportPose(PM);applyViewportPose(PM,pose);}
  entry.projection=entry.projection==='perspective'?'orthographic':'perspective';entry.autoOrtho=false;changed(PM);
}
export function setViewFov(PM:any,fov:number):void {
  if(!Number.isFinite(fov))throw new Error('Enter a finite field of view');
  const entry=state(PM);if(!entry.initialized)initialize(PM,entry);
  entry.fov=Math.min(150,Math.max(1,fov));changed(PM);
}
/** Turntable orbit: yaw about world up, pitch about the view's right axis. */
export function orbitPose(start:ViewPose,yawDegrees:number,pitchDegrees:number):ViewPose {
  const rotation=axisRotation(Y,yawDegrees).multiply(start.rotation).multiply(axisRotation(X,pitchDegrees));
  return {target:start.target.clone(),rotation:rotation.normalize(),distance:start.distance};
}
/** Numpad 2/4/6/8 step the view by 15°; Numpad 9 flips to the opposite side. */
export function orbitViewStep(PM:any,yawDegrees:number,pitchDegrees:number):void {
  const entry=state(PM),axis=entry.mode==='editor'?entry.axis:null;
  const next=orbitPose(viewportPose(PM),yawDegrees,pitchDegrees);
  // Stepping a named view by 180° lands on its opposite; keep it named and orthographic.
  const named=axis&&Math.abs(yawDegrees)===180&&!pitchDegrees&&['front','back','right','left'].includes(axis)?OPPOSITE_VIEW[axis]
    :axis&&Math.abs(pitchDegrees)===180&&!yawDegrees?OPPOSITE_VIEW[axis]:null;
  applyViewportPose(PM,named?{...next,rotation:VIEW_AXES[named].clone()}:next,{axis:named});
}
/** Numpad 2/4/6/8 with Shift roll the view about its own axis. */
export function rollView(PM:any,degrees:number):void {
  const pose=viewportPose(PM),axis=state(PM).mode==='editor'?state(PM).axis:null;
  applyViewportPose(PM,{...pose,rotation:pose.rotation.clone().multiply(axisRotation(new THREE.Vector3(0,0,1),degrees))},{axis});
}

/* ── Local View (Numpad /): only the chosen models show in the editor ── */
export const localViewIds=(PM:any):ReadonlySet<string>|null=>state(PM).local?.ids??null;
export function enterLocalView(PM:any,ids:Iterable<string>):boolean {
  const entry=state(PM),set=new Set(ids);if(!set.size)return false;
  const restore={mode:entry.mode,view:entry.initialized?cloneUser(entry):null};
  entry.local={ids:set,restore};changed(PM);return true;
}
export function exitLocalView(PM:any):boolean {
  const entry=state(PM),local=entry.local;if(!local)return false;
  entry.local=null;
  if(local.restore.view)Object.assign(entry,cloneUser(local.restore.view));
  entry.mode=local.restore.mode;changed(PM);return true;
}
