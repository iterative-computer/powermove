import * as THREE from 'three';
import {TransformControls} from 'three/addons/controls/TransformControls.js';
import {deltaCommands,upgradeGroupCommands,allowedModes,type TransformTarget,type TransformMode} from './targets';
import type {ViewportOrientation,ViewportPivot} from './editor-state';

/*
 * The Move, Rotate and Scale tools' gizmo. The overlay canvas never takes
 * pointer events: the viewport asks `hover`/`pointerDown` first and only a
 * press on a handle becomes a gesture. Every gesture edits the real layers'
 * own channels inside one Edit transaction (one Undo).
 */

const gizmoNotice='scene3d-gizmo';
const LABEL:Record<TransformMode,[string,string]>={translate:['Move 3D layer','Move 3D layers'],rotate:['Rotate 3D layer','Rotate 3D layers'],scale:['Scale 3D layer','Scale 3D layers']};
const SNAP={translate:1,rotate:5,scale:.1};

export interface GizmoSync {
  targets:TransformTarget[];
  mode:TransformMode|null;
  orientation:ViewportOrientation;
  pivot:ViewportPivot;
  activeId:string|null;
  cursor:THREE.Vector3;
  snap:boolean;
  /** View camera with a stage-wide view offset (pointer NDC spans the stage). */
  camera:THREE.PerspectiveCamera|THREE.OrthographicCamera;
}
export interface TransformGizmo {
  readonly helper:THREE.Object3D;
  attached():boolean;
  sync(state:GizmoSync):void;
  hover(clientX:number,clientY:number):boolean;
  pointerDown(event:PointerEvent):boolean;
  active():boolean;
  cancel():void;
  dispose():void;
}

export function createTransformGizmo(PM:any,stage:HTMLElement,canvas:HTMLCanvasElement,refresh:()=>void,resolve:()=>TransformTarget[]):TransformGizmo {
  // TransformControls reads the attached object's parent, so the pivot lives under an identity root.
  const pivot=new THREE.Object3D(),root=new THREE.Object3D();root.add(pivot);
  const controls=new TransformControls(new THREE.PerspectiveCamera(),canvas);
  // The canvas ignores pointers, so TransformControls' own listeners never fire;
  // the viewport drives pointerHover/pointerDown/pointerMove/pointerUp directly.
  controls.disconnect();
  const helper=controls.getHelper();
  let state:GizmoSync|null=null,targets:TransformTarget[]=[];
  let gesture:{label:string;start:TransformTarget[];frames:THREE.Matrix4[];pivotStart:THREE.Matrix4;pivotInverse:THREE.Matrix4;mode:TransformMode}|null=null;
  let fine=false,alt=false,snapping=false,suppressContextUntil=0,disposed=false;
  const cleanup:(()=>void)[]=[];let dragOffs:(()=>void)[]=[];
  const listen=(target:any,event:string,callback:any,options?:any)=>{target.addEventListener(event,callback,options);cleanup.push(()=>target.removeEventListener(event,callback,options));};
  const warn=(error:unknown)=>PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:gizmoNotice,error:true});

  function frameFor(t:TransformTarget|null,s:GizmoSync):THREE.Quaternion {
    if(s.orientation==='local'&&t)return t.rotation.clone();
    if(s.orientation==='view'){const q=new THREE.Quaternion();s.camera.matrixWorld.decompose(new THREE.Vector3(),q,new THREE.Vector3());return q;}
    return new THREE.Quaternion();
  }
  function place(list:TransformTarget[],s:GizmoSync){
    const active=list.find(t=>t.id===s.activeId)??list.at(-1)!;
    if(s.pivot==='cursor')pivot.position.copy(s.cursor);
    else if(s.pivot==='active')pivot.position.copy(active.position);
    else if(s.pivot==='bounds'){const box=new THREE.Box3();for(const t of list)box.expandByPoint(t.position);box.getCenter(pivot.position);}
    else {pivot.position.set(0,0,0);for(const t of list)pivot.position.add(t.position);pivot.position.divideScalar(list.length);}
    pivot.quaternion.copy(frameFor(active,s));pivot.scale.set(1,1,1);pivot.updateMatrixWorld(true);
  }
  function setSnap(on:boolean){
    if(on===snapping)return;snapping=on;
    controls.setTranslationSnap(on?SNAP.translate:null);controls.setRotationSnap(on?THREE.MathUtils.degToRad(SNAP.rotate):null);controls.setScaleSnap(on?SNAP.scale:null);
  }
  const modifiers=(event:{ctrlKey:boolean;metaKey?:boolean;shiftKey:boolean;altKey:boolean})=>{setSnap(!!state?.snap!==(event.ctrlKey||!!event.metaKey));fine=event.shiftKey;alt=event.altKey;};
  const pointer=(clientX:number,clientY:number,button=0)=>{
    const bounds=stage.getBoundingClientRect();
    return {x:(clientX-bounds.left)/Math.max(1,bounds.width)*2-1,y:-(clientY-bounds.top)/Math.max(1,bounds.height)*2+1,button};
  };
  function apply(){
    if(!gesture)return;
    pivot.updateMatrixWorld(true);
    let current=pivot.matrixWorld.clone();
    if(fine){
      // Shift: a tenth of the pointer's motion, measured from the start.
      const [p0,q0,s0]=[new THREE.Vector3(),new THREE.Quaternion(),new THREE.Vector3()],[p1,q1,s1]=[new THREE.Vector3(),new THREE.Quaternion(),new THREE.Vector3()];
      gesture.pivotStart.decompose(p0,q0,s0);current.decompose(p1,q1,s1);
      current=new THREE.Matrix4().compose(p0.lerp(p1,.1),q0.slerp(q1,.1),s0.lerp(s1,.1));
    }
    // The change in the pivot's own frame, re-applied around each target's frame.
    const local=gesture.pivotInverse.clone().multiply(current);
    const commands=gesture.start.flatMap((t,i)=>{
      const frame=gesture!.frames[i]!,delta=frame.clone().multiply(local).multiply(frame.clone().invert());
      return deltaCommands(PM,t,delta,gesture!.mode,{moveAim:alt});
    });
    if(!commands.length)return;
    const result=PM.Edit.apply(commands,{origin:'canvas',label:gesture.label});
    if(!result.ok){warn(result.message);cancel();}
  }
  controls.addEventListener('objectChange',()=>apply());
  function endDrag(){for(const off of dragOffs.splice(0))off();}
  function stop(){controls.dragging=false;controls.axis=null;}
  function finish(){
    if(!gesture)return;const label=gesture.label;suppressContextUntil=performance.now()+500;
    gesture=null;endDrag();stop();
    try{PM.Edit.commit(label);}catch(error){warn(error);}
    refresh();
  }
  function cancel(){
    if(!gesture)return;suppressContextUntil=performance.now()+500;
    gesture=null;endDrag();stop();
    try{PM.Edit.cancel();}catch(error){warn(error);}
    refresh();
  }
  listen(window,'keydown',(event:KeyboardEvent)=>{
    if(!gesture)return;
    if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();cancel();return;}
    modifiers(event);
  },true);
  listen(window,'keyup',(event:KeyboardEvent)=>{if(gesture)modifiers(event);},true);
  listen(window,'blur',()=>cancel());
  // macOS turns Control-click into a context menu, including snap drags; right-click cancels like Blender.
  listen(stage,'contextmenu',(event:MouseEvent)=>{
    if(!gesture&&performance.now()>suppressContextUntil)return;
    event.preventDefault();event.stopImmediatePropagation();
    if(gesture&&!event.ctrlKey)cancel();
  },true);

  return {
    helper,
    attached:()=>!!controls.object,
    sync(next){
      state=next;if(gesture||disposed)return;
      controls.camera=next.camera;
      targets=next.targets;
      const modes=allowedModes(targets);
      if(next.mode&&targets.length&&modes.includes(next.mode)){
        place(targets,next);controls.setMode(next.mode);controls.setSpace(next.orientation==='global'?'world':'local');controls.attach(pivot);
      }else controls.detach();
    },
    hover(clientX,clientY){
      if(gesture)return true;
      if(!controls.object)return false;
      controls.pointerHover(pointer(clientX,clientY) as any);canvas.dataset.gizmoAxis=controls.axis||'';refresh();
      return controls.axis!==null;
    },
    pointerDown(event){
      if(gesture||event.button!==0||!controls.object||!state)return false;
      const start=pointer(event.clientX,event.clientY,event.button);
      controls.pointerHover(start as any);
      if(controls.axis===null)return false;
      const mode=state.mode!,count=targets.length;
      if(!count)return false;
      const label=LABEL[mode][count===1?0:1];
      try{PM.Edit.begin(label,{origin:'canvas'});}catch(error){warn(error);return false;}
      const upgrades=upgradeGroupCommands(PM,targets);
      if(upgrades.length){
        const result=PM.Edit.apply(upgrades,{origin:'canvas'});
        if(!result.ok){PM.Edit.cancel();warn(result.message);return false;}
        targets=resolve();place(targets,state);
      }
      pivot.updateMatrixWorld(true);
      const pivotStart=pivot.matrixWorld.clone();
      const frames=targets.map(t=>state!.pivot==='individual'?new THREE.Matrix4().compose(t.position,pivot.quaternion,new THREE.Vector3(1,1,1)):pivotStart.clone());
      gesture={label,start:targets,frames,mode,pivotStart,pivotInverse:pivotStart.clone().invert()};
      modifiers(event);
      controls.pointerDown(start as any);
      if(!controls.dragging){cancel();return false;}
      const move=(e:PointerEvent)=>{if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();modifiers(e);controls.pointerMove(pointer(e.clientX,e.clientY,-1) as any);refresh();};
      const up=(e:PointerEvent)=>{if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();controls.pointerUp(pointer(e.clientX,e.clientY,0) as any);finish();};
      const lost=()=>cancel();
      window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);window.addEventListener('pointercancel',lost,true);
      dragOffs=[()=>window.removeEventListener('pointermove',move,true),()=>window.removeEventListener('pointerup',up,true),()=>window.removeEventListener('pointercancel',lost,true)];
      event.preventDefault();event.stopImmediatePropagation();
      return true;
    },
    active:()=>!!gesture,
    cancel,
    dispose(){
      if(disposed)return;cancel();disposed=true;
      for(const off of cleanup.splice(0))off();
      controls.detach();controls.dispose();helper.removeFromParent();
    }
  };
}
