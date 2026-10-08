import * as THREE from 'three';
import {compositionRuntime} from './service';
import {parent3D,world3D} from '../../legacy/core/space-3d';
import {layer3DRole} from './layers';
import {viewportPose,applyViewportPose,getViewportMode,setViewportMode,viewportState,activeCameraLayer,viewPosition,orbitPose,type ViewPose} from './viewport';
import {viewportPreferences,selected3D} from './editor-state';
import {editableLayer,propertyCommand,roleOf} from './targets';
import {sceneFromComp,compFromScene} from './space';

/*
 * 3D navigation. Middle-drag (or Alt+drag) orbits, adding Shift pans and Ctrl
 * zooms. In a free view the mouse wheel zooms and a trackpad orbits (Shift
 * pans, Ctrl or pinch zooms). Camera view keeps the composition's own 2D zoom
 * and pan; orbiting leaves it for a free view. With "Camera follows view" on,
 * navigating in camera view moves the camera layer itself.
 */

export const navigationNotice='scene3d-navigation';
export type NavigationKind='orbit'|'pan'|'dolly';
const ORBIT_DEGREES_PER_PIXEL=.45;

export const orbitView=(start:ViewPose,dx:number,dy:number):ViewPose=>orbitPose(start,-dx*ORBIT_DEGREES_PER_PIXEL,-dy*ORBIT_DEGREES_PER_PIXEL);
/** The scene follows the pointer: world units per pixel at the pivot's depth. */
export function panView(start:ViewPose,dx:number,dy:number,fov:number,height:number):ViewPose {
  const perPixel=2*start.distance*Math.tan(THREE.MathUtils.degToRad(fov)/2)/Math.max(1,height);
  const right=new THREE.Vector3(1,0,0).applyQuaternion(start.rotation),up=new THREE.Vector3(0,1,0).applyQuaternion(start.rotation);
  return {...start,rotation:start.rotation.clone(),target:start.target.clone().addScaledVector(right,-dx*perPixel).addScaledVector(up,dy*perPixel)};
}
/** Positive delta moves away from the pivot. */
export function dollyView(start:ViewPose,delta:number,scale=.005):ViewPose {
  const factor=Math.exp(Math.max(-2,Math.min(2,delta*scale)));
  return {...start,rotation:start.rotation.clone(),target:start.target.clone(),distance:Math.min(1e5,Math.max(.01,start.distance*factor))};
}
/** Keep the view's rotation and fit the bounding sphere into the narrower field of view. */
export function frameView(start:ViewPose,box:THREE.Box3,fov:number,aspect:number):ViewPose {
  const target=box.getCenter(new THREE.Vector3()),radius=Math.max(.1,box.getBoundingSphere(new THREE.Sphere()).radius);
  const vertical=THREE.MathUtils.degToRad(fov)/2,horizontal=Math.atan(Math.tan(vertical)*aspect);
  return {target,rotation:start.rotation.clone(),distance:radius*1.15/Math.sin(Math.min(vertical,horizontal))};
}

const comp=(PM:any)=>PM.curComp?.()||PM.proj;
/** Scene-space bounds of the selection (or everything): models, 3D-enabled 2D layers, and lights and cameras as points. */
export function viewportBounds(PM:any,selected=true,exclude:string|null=null):THREE.Box3 {
  const world=compositionRuntime(PM),ids=new Set<string>(selected?selected3D(PM):[]),box=new THREE.Box3(),toScene=sceneFromComp(comp(PM));
  for(const layer of comp(PM).layers){
    const role=roleOf(PM,layer);if(!role||role==='group'||!PM.active(layer,PM.time)||layer.id===exclude)continue;
    if(selected&&!ids.has(layer.id)&&!(PM.groupAncestors?.(layer)||[]).some((g:any)=>ids.has(g.id)))continue;
    const matrix=toScene.clone().multiply(new THREE.Matrix4().fromArray(world3D(PM,layer,PM.time)));
    if(role==='object'){const object=world.objects.get(layer.id);if(object)box.expandByObject(object);}
    else if(role==='plane'){
      const b=PM.GL?.bounds?.(layer,PM.time);
      if(b)for(const [x,y] of [[b.x0,b.y0],[b.x1,b.y0],[b.x1,b.y1],[b.x0,b.y1]])box.expandByPoint(new THREE.Vector3(x,y,0).applyMatrix4(matrix));
    }
    else box.expandByPoint(new THREE.Vector3().setFromMatrixPosition(matrix));
  }
  return box;
}
const viewAspect=(PM:any)=>{const c=comp(PM);return (c?.w||16)/Math.max(1,c?.h||9);};
const viewFov=(PM:any)=>{
  if(getViewportMode(PM)==='editor')return viewportState(PM).fov;
  const camera=compositionRuntime(PM).camera;return camera instanceof THREE.PerspectiveCamera?camera.fov:50;
};
/** Frame the selection, or everything. Returns false when there is nothing to frame. */
export function frameComposition(PM:any,selected=true):boolean {
  const box=viewportBounds(PM,selected,lockedCamera(PM)?.id??null);if(box.isEmpty())return false;
  const pose=frameView(viewportPose(PM),box,viewFov(PM),viewAspect(PM));
  if(lockedCamera(PM))writeCameraPose(PM,pose,'View camera');else applyViewportPose(PM,pose);
  return true;
}

/* ── the real camera ─────────────────────────────────── */
/** The camera layer navigation moves when "Camera follows view" is on in camera view. */
export function lockedCamera(PM:any):any|null {
  if(getViewportMode(PM)!=='camera'||!viewportPreferences(PM).lockCamera)return null;
  const layer=activeCameraLayer(PM);return layer&&editableLayer(PM,layer)?layer:null;
}
/** Edits that place the camera layer at a view pose (position and aim), keyed when animated or auto-keying. */
export function cameraPoseCommands(PM:any,layer:any,pose:ViewPose):any[] {
  // The pose is in scene units; camera channels are composition pixels in the parent's frame.
  const inverse=new THREE.Matrix4().fromArray(parent3D(PM,layer,PM.time)).invert().multiply(compFromScene(comp(PM)));
  const position=viewPosition(pose).applyMatrix4(inverse),target=pose.target.clone().applyMatrix4(inverse);
  return [...(['x','y','z'] as const).map(axis=>propertyCommand(PM,layer.id,`position.${axis}`,position[axis])),
    ...(['X','Y','Z'] as const).map(axis=>propertyCommand(PM,layer.id,`camera.target${axis}`,target[axis.toLowerCase() as 'x'|'y'|'z']))];
}
export function writeCameraPose(PM:any,pose:ViewPose,label:string,layer=activeCameraLayer(PM)):boolean {
  if(!layer)throw new Error('Add a camera first');
  if(!editableLayer(PM,layer))throw new Error('Unlock the camera first');
  const result=PM.Edit.apply(cameraPoseCommands(PM,layer,pose),{origin:'canvas',label});
  if(!result.ok)throw new Error(result.message);
  return true;
}
/** Move the active camera to the current view, then look through it. */
export function alignCameraToView(PM:any):boolean {
  const layer=activeCameraLayer(PM);if(!layer)throw new Error('Add a camera first');
  const pose=viewportPose(PM);writeCameraPose(PM,pose,'Align camera to view',layer);
  setViewportMode(PM,'camera');
  return true;
}
/** The selected camera becomes the composition camera (the topmost camera layer). */
export function setActiveCamera(PM:any,id:string):boolean {
  const layers=comp(PM).layers,layer=PM.L(id);
  if(layer3DRole(layer)!=='camera')throw new Error('Select a camera');
  const first=layers.findIndex((l:any)=>layer3DRole(l)==='camera');
  if(first<0||layers[first]===layer)return true;
  const result=PM.Edit.apply({type:'reorder_layer',target:id,index:first},{origin:'command',label:'Set active camera'});
  if(!result.ok)throw new Error(result.message);
  return true;
}

export interface NavigationHost {
  /** Composition rectangle height in CSS pixels, for pan speed. */
  height():number;
  /** Camera view pans and zooms the composition frame, like 2D. */
  panComposition(dx:number,dy:number):void;
  zoomComposition(factor:number,clientX:number,clientY:number):void;
}
export interface SceneNavigation {
  pointerDown(event:PointerEvent):boolean;
  wheel(event:WheelEvent):boolean;
  /** Start a drag from the navigation gizmo's buttons. */
  drag(event:PointerEvent,kind:NavigationKind):void;
  active():boolean;
  cancel():void;
  dispose():void;
}
const isTrackpad=(event:WheelEvent)=>{
  if(event.deltaMode!==0)return false;
  const legacy=Math.abs(Number((event as any).wheelDeltaY)||0);
  return !(legacy>=119&&Math.abs(legacy/120-Math.round(legacy/120))<.05);
};

export function createSceneNavigation(PM:any,stage:HTMLElement,host:NavigationHost,available:()=>boolean):SceneNavigation {
  let gesture:null|{start:ViewPose;kind:NavigationKind;x:number;y:number;camera:any|null;fov:number;composition:boolean}=null;
  let disposed=false,wheelEdit:{camera:any;timer:ReturnType<typeof setTimeout>|null}|null=null;
  const offs:(()=>void)[]=[],dragOffs:(()=>void)[]=[];
  const warn=(error:unknown)=>PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:navigationNotice,error:true});
  const endListeners=()=>{for(const off of dragOffs.splice(0))off();};
  function cancel(){
    if(!gesture)return;const {start,camera}=gesture;gesture=null;endListeners();
    if(camera)PM.Edit.cancel();else if(getViewportMode(PM)==='editor')applyViewportPose(PM,start);
  }
  const onBlur=()=>cancel();window.addEventListener('blur',onBlur);offs.push(()=>window.removeEventListener('blur',onBlur));
  const onKey=(event:KeyboardEvent)=>{if(event.key==='Escape'&&gesture){event.preventDefault();event.stopImmediatePropagation();cancel();}};
  window.addEventListener('keydown',onKey,true);offs.push(()=>window.removeEventListener('keydown',onKey,true));
  function flushWheelEdit(){if(!wheelEdit)return;const edit=wheelEdit;wheelEdit=null;if(edit.timer)clearTimeout(edit.timer);try{PM.Edit.commit('View camera');}catch(error){warn(error);}}
  function apply(pose:ViewPose,camera:any|null){
    if(camera){const result=PM.Edit.apply(cameraPoseCommands(PM,camera,pose),{origin:'canvas',label:'View camera'});if(!result.ok)throw new Error(result.message);}
    else applyViewportPose(PM,pose);
  }
  function begin(event:PointerEvent,kind:NavigationKind):boolean {
    flushWheelEdit();
    const camera=lockedCamera(PM),inCamera=getViewportMode(PM)==='camera';
    // Camera view without a lock: pan/zoom the composition frame; orbiting leaves for a user view.
    const composition=inCamera&&!camera&&kind!=='orbit';
    stage.focus?.({preventScroll:true});
    const start=viewportPose(PM),fov=viewFov(PM);
    if(camera)PM.Edit.begin('View camera',{origin:'canvas'});
    gesture={start,kind,x:event.clientX,y:event.clientY,camera,fov,composition};
    let last={x:event.clientX,y:event.clientY};
    const move=(e:PointerEvent)=>{
      if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();
      const slow=e.shiftKey&&kind!=='pan'?.1:1,dx=(e.clientX-gesture.x)*slow,dy=(e.clientY-gesture.y)*slow;
      try{
        if(gesture.composition){
          if(kind==='pan')host.panComposition(e.clientX-last.x,e.clientY-last.y);
          else host.zoomComposition(Math.exp(-(e.clientY-last.y)*.01),event.clientX,event.clientY);
        }else apply(kind==='orbit'?orbitView(gesture.start,dx,dy):kind==='pan'?panView(gesture.start,dx,dy,gesture.fov,host.height()):dollyView(gesture.start,dy),gesture.camera);
      }catch(error){warn(error);cancel();}
      last={x:e.clientX,y:e.clientY};
    };
    const up=(e:PointerEvent)=>{
      if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();
      const {camera}=gesture;gesture=null;endListeners();
      if(camera){try{PM.Edit.commit('View camera');}catch(error){warn(error);}}
    };
    const lost=()=>cancel();
    window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);window.addEventListener('pointercancel',lost,true);
    dragOffs.push(()=>window.removeEventListener('pointermove',move,true),()=>window.removeEventListener('pointerup',up,true),()=>window.removeEventListener('pointercancel',lost,true));
    event.preventDefault();event.stopImmediatePropagation();
    return true;
  }
  return {
    pointerDown(event){
      if(disposed||gesture||!available())return false;
      const middle=event.button===1,emulated=event.button===0&&event.altKey&&!event.metaKey;
      if(!middle&&!emulated)return false;
      const kind:NavigationKind=event.shiftKey?'pan':event.ctrlKey||event.metaKey?'dolly':'orbit';
      try{return begin(event,kind);}catch(error){warn(error);cancel();event.preventDefault();return true;}
    },
    drag(event,kind){if(!disposed&&!gesture){try{begin(event,kind);}catch(error){warn(error);cancel();}}},
    wheel(event){
      if(disposed||!available())return false;
      const camera=lockedCamera(PM);
      if(getViewportMode(PM)==='camera'&&!camera)return false;
      event.preventDefault();event.stopImmediatePropagation();if(gesture)return true;
      try{
        const scale=event.deltaMode===1?16:event.deltaMode===2?host.height():1,dx=event.deltaX*scale,dy=event.deltaY*scale;
        const start=viewportPose(PM),trackpad=isTrackpad(event);
        let pose:ViewPose;
        if(event.ctrlKey||event.metaKey)pose=dollyView(start,dy,trackpad?.01:.002);
        else if(!trackpad)pose=dollyView(start,dy,.002);
        else if(event.shiftKey)pose=panView(start,-dx,-dy,viewFov(PM),host.height());
        else pose=orbitPose(start,dx*.35,dy*.35);
        if(camera){
          if(!wheelEdit){PM.Edit.begin('View camera',{origin:'canvas'});wheelEdit={camera,timer:null};}
          if(wheelEdit.timer)clearTimeout(wheelEdit.timer);wheelEdit.timer=setTimeout(flushWheelEdit,350);
          apply(pose,camera);
        }else applyViewportPose(PM,pose);
      }catch(error){warn(error);}
      return true;
    },
    active:()=>!!gesture,cancel,
    dispose(){cancel();flushWheelEdit();disposed=true;for(const off of offs.splice(0))off();}
  };
}
