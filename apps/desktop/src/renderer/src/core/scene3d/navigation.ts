import * as THREE from 'three';
import { compositionRuntime } from './service';
import { layer3DRole } from './layers';
import {viewportCamera,viewportPose,applyViewportPose,getViewportMode,setViewportMode,type ViewPose,type ViewMode} from './viewport';
export type {ViewPose} from './viewport';

export const navigationNotice='scene3d-navigation';
export type NavigationMode='select'|'orbit'|'pan'|'dolly';
export interface SceneNavigation {
  update(rect:{width:number;height:number},selection:string[]):void;
  pointerDown(event:PointerEvent):boolean;
  wheel(event:WheelEvent):boolean;
  frame(selected?:boolean):void;
  active():boolean;
  cancel():void;
  dispose():void;
}
const states=new WeakMap<object,{mode:NavigationMode;listeners:Set<(mode:NavigationMode)=>void>}>();
function state(PM:any){let entry=states.get(PM);if(!entry){entry={mode:'select',listeners:new Set()};states.set(PM,entry);}return entry;}
export const getNavigationMode=(PM:any)=>state(PM).mode;
export function setNavigationMode(PM:any,mode:NavigationMode):void {
  if(!['select','orbit','pan','dolly'].includes(mode))throw new Error('Unknown 3D navigation tool');
  const entry=state(PM);entry.mode=mode;for(const listener of entry.listeners)listener(mode);
}
export function onNavigationMode(PM:any,listener:(mode:NavigationMode)=>void):()=>void {const entry=state(PM);entry.listeners.add(listener);return()=>entry.listeners.delete(listener);}

export function orbitView(start:ViewPose,dx:number,dy:number):ViewPose {
  const sphere=new THREE.Spherical().setFromVector3(start.position.clone().sub(start.target));
  sphere.radius=Math.max(.05,sphere.radius);sphere.theta-=dx*.008;sphere.phi=Math.max(.01,Math.min(Math.PI-.01,sphere.phi-dy*.008));
  return {...start,position:new THREE.Vector3().setFromSpherical(sphere).add(start.target)};
}
export function panView(start:ViewPose,dx:number,dy:number,camera:THREE.Camera,height:number):ViewPose {
  const scale=camera instanceof THREE.PerspectiveCamera?2*start.position.distanceTo(start.target)*Math.tan(THREE.MathUtils.degToRad(camera.fov/2))/camera.zoom
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
    const layer=PM.L(id);if(layer3DRole(layer)!=='object')continue;
    if(selected&&!ids.has(id)&&!(PM.groupAncestors?.(layer)||[]).some((g:any)=>ids.has(g.id)))continue;
    box.expandByObject(object);
  }
  if(box.isEmpty())return;
  const start=viewportPose(PM);
  applyViewportPose(PM,frameView(start,box,viewportCamera(PM,world.camera)));
}

/** Navigation changes only the transient editor viewpoint, never the shot. */
export function createSceneNavigation(PM:any,stage:HTMLElement):SceneNavigation {
  const previousTabIndex=stage.getAttribute('tabindex');
  if(previousTabIndex===null)stage.tabIndex=-1;
  let height=1,over=false,gesture:null|{start:ViewPose;viewMode:ViewMode;camera:THREE.Camera;x:number;y:number;mode:NavigationMode}=null;
  let disposed=false;
  const offs:(()=>void)[]=[],dragOffs:(()=>void)[]=[];
  const listen=(target:EventTarget,event:string,fn:EventListener,options?:boolean)=>{target.addEventListener(event,fn,options);offs.push(()=>target.removeEventListener(event,fn,options));};
  const available=()=>!disposed&&(PM.curComp?.()||PM.proj).layers.some((l:any)=>layer3DRole(l)==='object'&&PM.active(l,PM.time));
  const warn=(error:unknown)=>PM.toast?.(error instanceof Error?error.message:String(error),{key:navigationNotice,error:true});
  const endListeners=()=>{for(const off of dragOffs.splice(0))off();};
  function cancel(){if(!gesture)return;const {start,viewMode}=gesture;gesture=null;endListeners();applyViewportPose(PM,start);setViewportMode(PM,viewMode);}
  listen(stage,'pointerenter',()=>{over=true;});listen(stage,'pointerleave',()=>{over=false;});
  listen(window,'blur',()=>cancel());
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
  return {
    update(rect){height=Math.max(1,rect.height);},
    pointerDown(event){
      if(!available()||gesture||![0,1].includes(event.button))return false;
      const mode=event.button===1||event.altKey?(event.shiftKey?'pan':'orbit'):getNavigationMode(PM);
      if(mode==='select')return false;
      stage.focus({preventScroll:true});
      try{
        const world=compositionRuntime(PM),viewMode=getViewportMode(PM),start=viewportPose(PM);
        gesture={start,viewMode,camera:viewportCamera(PM,world.camera).clone(),x:event.clientX,y:event.clientY,mode};
        const move=(e:PointerEvent)=>{if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();
          const dx=(e.clientX-gesture.x)*(e.shiftKey&&mode!=='pan'?.1:1),dy=(e.clientY-gesture.y)*(e.shiftKey&&mode!=='pan'?.1:1);
          try{applyViewportPose(PM,mode==='orbit'?orbitView(gesture.start,dx,dy):mode==='pan'?panView(gesture.start,dx,dy,gesture.camera,height):dollyView(gesture.start,dy,gesture.camera));}catch(error){warn(error);cancel();}
        };
        const up=(e:PointerEvent)=>{if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();gesture=null;endListeners();};
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
        applyViewportPose(PM,dollyView(start,delta,viewportCamera(PM,compositionRuntime(PM).camera)));
      }catch(error){cancel();warn(error);}return true;
    },
    active:()=>!!gesture,cancel,
    frame(selected=true){if(gesture)cancel();frameComposition(PM,selected);},
    dispose(){cancel();disposed=true;for(const off of offs.splice(0))off();if(previousTabIndex===null)stage.removeAttribute('tabindex');else stage.setAttribute('tabindex',previousTabIndex);}
  };
}
