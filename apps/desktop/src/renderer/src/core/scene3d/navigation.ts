import {activeCameraLayer,cameraNavigationEdit} from './camera-navigation';
import { isExtrudedLayer } from './extrusion-data';
import * as THREE from 'three';
import { compositionRuntime } from './service';
import { layer3DRole } from './layers';
import {viewportCamera,viewportPose,applyViewportPose,getViewportMode,setViewportMode,type ViewPose,type ViewMode,type ViewCamera} from './viewport';
export type {ViewPose} from './viewport';

export const navigationNotice='scene3d-navigation';
export type NavigationMode='select'|'orbit'|'pan'|'dolly';
export type NavigationTool=Exclude<NavigationMode,'select'>;
export type NavigationVariant='cursor'|'scene'|'poi'|'towards-cursor';
const variants:Record<NavigationTool,NavigationVariant[]>={orbit:['cursor','scene','poi'],pan:['cursor','poi'],dolly:['towards-cursor','cursor','poi']};
export interface SceneNavigation {
  update(rect:{width:number;height:number},selection:string[]):void;
  pointerDown(event:PointerEvent):boolean;
  wheel(event:WheelEvent):boolean;
  frame(selected?:boolean):void;
  active():boolean;
  cancel():void;
  dispose():void;
}
const states=new WeakMap<object,{mode:NavigationMode;variants:Record<NavigationTool,NavigationVariant>;listeners:Set<(mode:NavigationMode)=>void>}>();
function state(PM:any){let entry=states.get(PM);if(!entry){entry={mode:'select',variants:{orbit:'cursor',pan:'cursor',dolly:'towards-cursor'},listeners:new Set()};states.set(PM,entry);}return entry;}
export const getNavigationMode=(PM:any)=>state(PM).mode;
export function setNavigationMode(PM:any,mode:NavigationMode):void {
  if(!['select','orbit','pan','dolly'].includes(mode))throw new Error('Unknown 3D navigation tool');
  const entry=state(PM);entry.mode=mode;for(const listener of entry.listeners)listener(mode);
}
export function onNavigationMode(PM:any,listener:(mode:NavigationMode)=>void):()=>void {const entry=state(PM);entry.listeners.add(listener);return()=>entry.listeners.delete(listener);}

export const getNavigationVariant=(PM:any,tool:NavigationTool)=>state(PM).variants[tool];
export function setNavigationVariant(PM:any,tool:NavigationTool,variant:NavigationVariant):void {
  if(!variants[tool]?.includes(variant))throw new Error('Unknown 3D navigation variant');
  const entry=state(PM);entry.variants[tool]=variant;
  for(const listener of entry.listeners)listener(entry.mode);
}

/** Rotate the entire view around a pivot without jumping the aim on mouse-down. */
export function orbitAround(start:ViewPose,pivot:THREE.Vector3,dx:number,dy:number):ViewPose {
  const offset=start.position.clone().sub(pivot);
  if(offset.lengthSq()<1e-9)return start;
  const next=orbitView({...start,target:pivot},dx,dy);
  const rotation=new THREE.Quaternion().setFromUnitVectors(offset.normalize(),next.position.clone().sub(pivot).normalize());
  return {...next,target:start.target.clone().sub(pivot).applyQuaternion(rotation).add(pivot)};
}
/** Dolly with parallel aim: the clicked point stays under the same screen pixel. */
export function dollyAt(start:ViewPose,pivot:THREE.Vector3,delta:number,camera:THREE.Camera,towards=false):ViewPose {
  if(towards&&delta>0)return dollyAt(start,start.target,delta,camera);
  const next=dollyView({...start,target:pivot},delta,camera);
  if(camera instanceof THREE.OrthographicCamera){
    const forward=camera.getWorldDirection(new THREE.Vector3()),offset=pivot.clone().sub(start.position);
    offset.addScaledVector(forward,-offset.dot(forward)).multiplyScalar(1-start.zoom/next.zoom);
    return {...next,position:start.position.clone().add(offset),target:start.target.clone().add(offset)};
  }
  return {...next,target:start.target.clone().add(next.position.clone().sub(start.position))};
}
/** Empty-space navigation falls back to the plane through the view's POI. */
export function cursorPivot(camera:THREE.Camera,target:THREE.Vector3,x:number,y:number,hit?:THREE.Vector3):THREE.Vector3 {
  if(hit)return hit.clone();
  camera.updateMatrixWorld(true);
  const ray=new THREE.Raycaster();ray.setFromCamera(new THREE.Vector2(x,y),camera);
  const plane=new THREE.Plane().setFromNormalAndCoplanarPoint(camera.getWorldDirection(new THREE.Vector3()),target);
  return ray.ray.intersectPlane(plane,new THREE.Vector3()) ?? target.clone();
}

export function orbitView(start:ViewPose,dx:number,dy:number):ViewPose {
  const sphere=new THREE.Spherical().setFromVector3(start.position.clone().sub(start.target));
  sphere.radius=Math.max(.05,sphere.radius);sphere.theta-=dx*.008;sphere.phi=Math.max(.01,Math.min(Math.PI-.01,sphere.phi-dy*.008));
  return {...start,position:new THREE.Vector3().setFromSpherical(sphere).add(start.target)};
}
export function panView(start:ViewPose,dx:number,dy:number,camera:THREE.Camera,height:number,pivot=start.target):ViewPose {
  const scale=camera instanceof THREE.PerspectiveCamera?2*Math.max(.05,pivot.clone().sub(start.position).dot(camera.getWorldDirection(new THREE.Vector3())))*Math.tan(THREE.MathUtils.degToRad(camera.fov/2))/camera.zoom
    :camera instanceof THREE.OrthographicCamera?(camera.top-camera.bottom)/camera.zoom:6;
  const right=new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld,0),up=new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld,1);
  const offset=right.multiplyScalar(-dx).addScaledVector(up,dy).multiplyScalar(scale/Math.max(1,height));
  return {...start,position:start.position.clone().add(offset),target:start.target.clone().add(offset)};
}
export function dollyView(start:ViewPose,delta:number,camera:THREE.Camera):ViewPose {
  const factor=Math.exp(Math.max(-2,Math.min(2,delta*.005)));
  if(camera instanceof THREE.OrthographicCamera)return {...start,zoom:Math.max(.01,Math.min(1000,start.zoom/factor))};
  const offset=start.position.clone().sub(start.target),distance=offset.length(),lens=camera as THREE.PerspectiveCamera;
  if(distance<1e-9)offset.set(0,0,1);
  offset.setLength(Math.max(lens.near*2,.05,Math.min(lens.far*.8,Math.max(.05,distance)*factor)));
  return {...start,position:offset.add(start.target)};
}
export function frameView(start:ViewPose,box:THREE.Box3,camera:THREE.Camera):ViewPose {
  const target=box.getCenter(new THREE.Vector3()),radius=Math.max(.1,box.getBoundingSphere(new THREE.Sphere()).radius),direction=start.position.clone().sub(start.target);
  if(direction.lengthSq()<1e-9)direction.set(0,0,1);direction.normalize();
  if(camera instanceof THREE.OrthographicCamera){
    const extent=Math.min(camera.right-camera.left,camera.top-camera.bottom)/2;
    return {position:target.clone().addScaledVector(direction,Math.max(1,start.position.distanceTo(start.target))),target,zoom:extent/(radius*1.25)};
  }
  const lens=camera as THREE.PerspectiveCamera,vertical=Math.atan(Math.tan(THREE.MathUtils.degToRad(lens.fov/2))/lens.zoom),horizontal=Math.atan(Math.tan(vertical)*lens.aspect);
  return {position:target.clone().addScaledVector(direction,radius*1.25/Math.sin(Math.min(vertical,horizontal))),target,zoom:start.zoom};
}

export function frameComposition(PM:any,selected=true):void {
  const world=compositionRuntime(PM),ids=new Set<string>(PM.sel?.layers || []),box=new THREE.Box3();
  for(const [id,object] of world.objects){
    const layer=PM.L(id);if(layer3DRole(layer)!=='object' && !isExtrudedLayer(layer))continue;
    if(selected&&!ids.has(id)&&!(PM.groupAncestors?.(layer)||[]).some((g:any)=>ids.has(g.id)))continue;
    box.expandByObject(object);
  }
  if(box.isEmpty())return;
  const start=viewportPose(PM);
  applyViewportPose(PM,frameView(start,box,viewportCamera(PM,world.camera)));
}

/** Active Camera edits the shot; Free View remains a transient editor viewpoint. */
export function createSceneNavigation(PM:any,stage:HTMLElement):SceneNavigation {
  const previousTabIndex=stage.getAttribute('tabindex');
  if(previousTabIndex===null)stage.tabIndex=-1;
  let height=1,over=false,gesture:null|{start:ViewPose;viewMode:ViewMode;camera:THREE.Camera;pivot:THREE.Vector3;variant:NavigationVariant;x:number;y:number;mode:NavigationMode;edit:ReturnType<typeof cameraNavigationEdit>|null}=null;
  let disposed=false;
  const offs:(()=>void)[]=[],dragOffs:(()=>void)[]=[];
  const listen=(target:EventTarget,event:string,fn:EventListener,options?:boolean)=>{target.addEventListener(event,fn,options);offs.push(()=>target.removeEventListener(event,fn,options));};
  const available=()=>!disposed&&(PM.curComp?.()||PM.proj).layers.some((l:any)=>(!!layer3DRole(l) || isExtrudedLayer(l))&&PM.active(l,PM.time));
  const pivotAt=(event:{clientX:number;clientY:number},camera:ViewCamera,start:ViewPose,variant:NavigationVariant)=>{
    if(variant==='poi')return start.target.clone();
    const world=compositionRuntime(PM);
    if(variant==='scene'){
      const box=world.worldBounds();return box.isEmpty()?start.target.clone():box.getCenter(new THREE.Vector3());
    }
    const rect=(stage.querySelector('#gl') || stage).getBoundingClientRect();
    const x=(event.clientX-rect.left)/Math.max(1,rect.width)*2-1,y=1-(event.clientY-rect.top)/Math.max(1,rect.height)*2;
    return cursorPivot(camera,start.target,x,y,world.pickHit(x,y,undefined,camera)?.point);
  };
  const warn=(error:unknown)=>PM.toast?.(error instanceof Error?error.message:String(error),{key:navigationNotice,error:true});
  const endListeners=()=>{for(const off of dragOffs.splice(0))off();};
  function cancel(){
    if(!gesture)return;
    const {start,viewMode,edit}=gesture;gesture=null;endListeners();
    if(edit)edit.cancel();else {applyViewportPose(PM,start);setViewportMode(PM,viewMode);}
  }
  listen(stage,'pointerenter',()=>{over=true;});listen(stage,'pointerleave',()=>{over=false;});
  listen(window,'blur',()=>cancel());
  offs.push(onNavigationMode(PM,()=>cancel()));
  listen(window,'keydown',((event:KeyboardEvent)=>{
    const focused=document.activeElement as HTMLElement|null;
    if(focused?.matches('input,textarea,select,[contenteditable="true"]'))return;
    if(event.key==='Escape'&&gesture){event.preventDefault();event.stopImmediatePropagation();cancel();return;}
    if(!over||!available()||getNavigationMode(PM)==='select'||gesture)return;
    if(focused?.matches('[role="combobox"],[role="listbox"]'))return;
    if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();setNavigationMode(PM,'select');return;}
    if((event.key.toLowerCase()==='f'||event.key==='Home')&&!event.metaKey&&!event.ctrlKey&&!event.altKey){
      event.preventDefault();event.stopImmediatePropagation();try{frameComposition(PM,event.key!=='Home');}catch(error){warn(error);}
    }
  }) as EventListener,true);
  const controller:SceneNavigation={
    update(rect){height=Math.max(1,rect.height);},
    pointerDown(event){
      if(!available()||gesture||![0,1,2].includes(event.button))return false;
      const mode=event.altKey?(event.button===2?'dolly':event.button===1||event.shiftKey?'pan':'orbit'):event.button===1?(event.shiftKey?'pan':'orbit'):event.button===0?getNavigationMode(PM):'select';
      if(mode==='select')return false;
      stage.focus({preventScroll:true});
      try{
        const world=compositionRuntime(PM),viewMode=getViewportMode(PM),start=viewportPose(PM);
        const camera=viewportCamera(PM,world.camera).clone(),variant=getNavigationVariant(PM,mode);
        const layer=viewMode==='camera'?activeCameraLayer(PM):null;
        const edit=layer?cameraNavigationEdit(PM,layer,start,`${mode==='orbit'?'Orbit':mode==='pan'?'Pan':'Dolly'} camera`):null;
        gesture={start,viewMode,camera,pivot:pivotAt(event,camera,start,variant),variant,x:event.clientX,y:event.clientY,mode,edit};
        const move=(e:PointerEvent)=>{if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();
          let dx=e.clientX-gesture.x,dy=e.clientY-gesture.y;
          if(e.shiftKey&&mode!=='dolly'){if(Math.abs(dx)>=Math.abs(dy))dy=0;else dx=0;}
          try{const next=mode==='orbit'?orbitAround(gesture.start,gesture.pivot,dx,dy):mode==='pan'?panView(gesture.start,dx,dy,gesture.camera,height,gesture.pivot):dollyAt(gesture.start,gesture.pivot,dy,gesture.camera,gesture.variant==='towards-cursor');
            if(gesture.edit)gesture.edit.update(next);else applyViewportPose(PM,next);
          }catch(error){warn(error);cancel();}
        };
        const up=(e:PointerEvent)=>{
          if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();
          try{gesture.edit?.commit();gesture=null;endListeners();}catch(error){warn(error);cancel();}
        };
        const lost=()=>cancel();
        window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);window.addEventListener('pointercancel',lost,true);
        dragOffs.push(()=>window.removeEventListener('pointermove',move,true),()=>window.removeEventListener('pointerup',up,true),()=>window.removeEventListener('pointercancel',lost,true));
        event.preventDefault();event.stopImmediatePropagation();return true;
      }catch(error){if(gesture)cancel();warn(error);event.preventDefault();event.stopImmediatePropagation();return true;}
    },
    wheel(event){
      if(!available()||(getNavigationMode(PM)==='select'&&!event.altKey))return false;
      event.preventDefault();event.stopImmediatePropagation();if(gesture)return true;
      try{
        const delta=event.deltaY*(event.deltaMode===1?16:event.deltaMode===2?height:1),start=viewportPose(PM);
        const camera=viewportCamera(PM,compositionRuntime(PM).camera),variant=getNavigationVariant(PM,'dolly');
        const next=dollyAt(start,pivotAt(event,camera,start,variant),delta,camera,variant==='towards-cursor');
        const layer=getViewportMode(PM)==='camera'?activeCameraLayer(PM):null;
        if(layer){const edit=cameraNavigationEdit(PM,layer,start,'Dolly camera');
          try{edit.update(next);edit.commit();}catch(error){edit.cancel();throw error;}
        }else applyViewportPose(PM,next);
      }catch(error){cancel();warn(error);}return true;
    },
    active:()=>!!gesture,cancel,
    frame(selected=true){if(gesture)cancel();frameComposition(PM,selected);},
    dispose(){cancel();disposed=true;for(const off of offs.splice(0))off();if(previousTabIndex===null)stage.removeAttribute('tabindex');else stage.setAttribute('tabindex',previousTabIndex);}
  };
  // Capture shot-camera tools before the viewer's ordinary layer/2D handlers.
  // This also supports installed viewer forks using the existing controller API.
  listen(stage,'pointerdown',((event:PointerEvent)=>{
    if(getViewportMode(PM)==='camera' && (getNavigationMode(PM)!=='select'||event.altKey))controller.pointerDown(event);
  }) as EventListener,true);
  listen(stage,'wheel',((event:WheelEvent)=>{
    if(getViewportMode(PM)==='camera')controller.wheel(event);
  }) as EventListener,true);
  return controller;
}
