import {viewportCamera} from './viewport';
import * as THREE from 'three';
import { TransformControls } from 'three/addons/controls/TransformControls.js';
import { layer3DRole, isModelGroup, type Layer3DRole } from './layers';
import { parent3D,world3D } from '../../legacy/core/space-3d';
import { themeColor } from './theme';
import type { SceneRuntime } from './runtime';

const gizmoNotice='scene3d-gizmo';

/*
 * Blender-style transform gizmo drawn over the composition viewer. The canvas
 * never takes pointer events: the viewer asks `hover`/`pointerDown` first and
 * only a press on a handle becomes a gizmo gesture. Every gesture edits the
 * real layers' own channels inside one Edit transaction (one Undo).
 */

export type GizmoMode = 'translate'|'rotate'|'scale';
export type GizmoSpace = 'world'|'local';
export interface GizmoRect { x:number;y:number;width:number;height:number }
export interface SceneGizmo {
  /** Follow the composition frame (stage-relative CSS px) and the layer selection. */
  update(rect:GizmoRect,selection?:string[]):void;
  /** True when the client point is over a handle; highlights it. */
  hover(clientX:number,clientY:number):boolean;
  /** Take a press that lands on a handle. Returns false to let the viewer handle it. */
  pointerDown(event:PointerEvent):boolean;
  /** True while a gizmo gesture is in progress. */
  active():boolean;
  /** True when every id is a 3D layer, so the viewer replaces its 2D handles. */
  covers(ids:readonly string[]):boolean;
  cancel():void;
  dispose():void;
}

/* ── shared mode / space (viewer chrome and any 3D view read the same state) ── */
type GizmoState = {mode:GizmoMode;space:GizmoSpace};
let state:GizmoState={mode:'translate',space:'world'};
const stateListeners=new Set<(state:GizmoState)=>void>();
const publish=()=>{for(const listener of stateListeners)listener({...state});};
export const getAutoKey=(PM:any):boolean=>!!PM.autokey;
export function setAutoKey(PM:any,enabled:boolean){PM.autokey=!!enabled;publish();}
export const getGizmoMode=():GizmoMode=>state.mode;
export const getGizmoSpace=():GizmoSpace=>state.space;
export function setGizmoMode(mode:GizmoMode):void {
  if(!['translate','rotate','scale'].includes(mode))throw new Error(`Unknown gizmo mode "${String(mode)}"`);
  if(state.mode!==mode){state={...state,mode};publish();}
}
export function setGizmoSpace(space:GizmoSpace):void {
  if(space!=='world'&&space!=='local')throw new Error(`Unknown gizmo space "${String(space)}"`);
  if(state.space!==space){state={...state,space};publish();}
}
export function onGizmoState(listener:(state:GizmoState)=>void):()=>void {stateListeners.add(listener);return()=>stateListeners.delete(listener);}

/** Which handles apply: a sun only aims, a point light only moves, a camera never scales. */
export function gizmoModes(role:Layer3DRole,lightType?:string):GizmoMode[] {
  if(role==='object')return ['translate','rotate','scale'];
  if(role==='camera')return ['translate','rotate'];
  return lightType==='sun'?['rotate']:lightType==='point'?['translate']:['translate','rotate'];
}

const LABEL:Record<GizmoMode,[string,string]>={translate:['Move 3D layer','Move 3D layers'],rotate:['Rotate 3D layer','Rotate 3D layers'],scale:['Scale 3D layer','Scale 3D layers']};
const SNAP={translate:1,rotate:15,scale:.1};
const tidy=(value:number)=>Math.round(value*1e9)/1e9;
const vec=(values:number[])=>new THREE.Vector3(values[0],values[1],values[2]);

type Target = {
  id:string;layer:any;role:Layer3DRole|'group';modes:GizmoMode[];
  /** Objects: the runtime node. Lights and cameras are placed from their layer values. */
  object:THREE.Object3D|null;
  world:THREE.Matrix4;parentInverse:THREE.Matrix4;position:THREE.Vector3;target:THREE.Vector3|null;
  orientation:THREE.Vector3;order:THREE.EulerOrder;
};

export function createSceneGizmo(PM:any,stage:HTMLElement,runtime:(time:number)=>SceneRuntime|null):SceneGizmo {
  const renderer=new THREE.WebGLRenderer({alpha:true,antialias:true});
  renderer.setPixelRatio(Math.min(2,window.devicePixelRatio || 1));
  const canvas=renderer.domElement;
  canvas.setAttribute('aria-hidden','true');canvas.dataset.sceneGizmo='';
  canvas.style.cssText='position:absolute;left:0;top:0;width:100%;height:100%;pointer-events:none;z-index:4';
  stage.append(canvas);
  const scene=new THREE.Scene();
  const pivot=new THREE.Object3D();scene.add(pivot);
  let camera:THREE.Camera=new THREE.PerspectiveCamera();
  const controls=new TransformControls(camera as THREE.PerspectiveCamera,canvas);
  // The canvas ignores pointers, so TransformControls' own listeners never fire;
  // the viewer drives pointerHover/pointerDown/pointerMove/pointerUp directly.
  controls.disconnect();
  scene.add(controls.getHelper());
  const outlines=new Map<string,THREE.BoxHelper>();
  let accent=themeColor(stage,'--accent',0xff7a33);
  let rect:GizmoRect={x:0,y:0,width:1,height:1},ids:string[]=[],targets:Target[]=[];
  let frame=0,disposed=false,gesture:{label:string;start:Target[];pivotInverse:THREE.Matrix4;pivotStart:THREE.Matrix4;mode:GizmoMode}|null=null;
  let fine=false,alt=false,snapping=false;
  let suppressContextUntil=0;
  const cleanup:(()=>void)[]=[];
  const listen=(target:any,event:string,callback:any,options?:any)=>{target.addEventListener(event,callback,options);cleanup.push(()=>target.removeEventListener(event,callback,options));};

  const layer=(id:string)=>PM.L?.(id);
  const roleOf=(l:any)=>layer3DRole(l) || (isModelGroup(PM,l)?'group':null);
  const editable=(l:any)=>!!l&&!l.lock&&!(PM.groupAncestors?.(l)||[]).some((g:any)=>g.lock);
  const ev=(l:any,prop:any,path:string,fallback=0)=>prop?Number(PM.evP(l,prop,PM.time,path)):fallback;
  const channel=(l:any,path:string,fallback=0)=>ev(l,l.p?.[path],path,fallback);
  const activeCamera=()=>(PM.curComp?.()||PM.proj)?.layers?.find((l:any)=>layer3DRole(l)==='camera'&&(PM.active?PM.active(l,PM.time):true))?.id ?? null;
  function warn(error:unknown){PM.toast?.(error instanceof Error?error.message:String(error),{key:gizmoNotice,error:true});}

  /* ── targets ─────────────────────────────────────────── */
  function resolveTargets(runtimeWorld:SceneRuntime|null):Target[] {
    const out:Target[]=[],active=activeCamera();
    for(const id of ids) {
      const l=layer(id),role=roleOf(l);
      // The active camera is the view itself; it is edited from the inspector.
      if(!role||!editable(l)||(role==='camera'&&id===active))continue;
      if(PM.active&&!PM.active(l,PM.time))continue;
      const lightType=role==='light'?l.d?.data?.light?.type:undefined;
      const object=role==='object'?runtimeWorld?.objects.get(id) ?? null:null;
      if(role==='object'&&!object)continue;
      const position=role==='object'?object!.getWorldPosition(new THREE.Vector3())
        :new THREE.Vector3().setFromMatrixPosition(new THREE.Matrix4().fromArray(world3D(PM,l,PM.time)));
      const content=role==='light'?l.d?.data?.light?.p:role==='camera'?l.d?.data?.camera?.p:null,prefix=role==='light'?'light':'camera';
      const target=content?vec(['X','Y','Z'].map(axis=>ev(l,content[`target${axis}`],`${prefix}.target${axis}`))).applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,l,PM.time))):null;
      let world=new THREE.Matrix4();
      if(object){object.updateWorldMatrix(true,false);world=object.matrixWorld.clone();}
      else if(role==='group')world.fromArray(world3D(PM,l,PM.time));
      else {
        // A light or camera's frame looks from its position toward its target.
        const look=new THREE.Matrix4().lookAt(position,target&&target.distanceToSquared(position)>1e-12?target:position.clone().add(new THREE.Vector3(0,0,-1)),new THREE.Vector3(0,1,0));
        world.copy(look).setPosition(position);
      }
      if(role==='object'||role==='group')position.copy(vec(['x','y','z'].map(axis=>channel(l,`anchor.${axis}`))).applyMatrix4(world));
      if(role==='group'&&!l.threeD){
        const box=new THREE.Box3();
        for(const child of (PM.curComp?.()||PM.proj).layers)if((PM.groupAncestors?.(child)||[]).some((g:any)=>g.id===id)){
          const node=runtimeWorld?.objects.get(child.id);if(node)box.expandByObject(node);
        }
        if(!box.isEmpty())box.getCenter(position);
      }
      out.push({id,layer:l,role,modes:role==='group'?['translate','rotate','scale']:gizmoModes(role,lightType),object,world,
        parentInverse:new THREE.Matrix4().fromArray(parent3D(PM,l,PM.time)).invert(),position,target,
        orientation:vec(['x','y','z'].map(axis=>channel(l,`orientation.${axis}`))),order:(object?.rotation.order ?? 'ZYX') as THREE.EulerOrder});
    }
    const roots=new Set((PM.transformRoots?.(out.map(t=>t.id)) || out.map(t=>t.layer)).map((l:any)=>l.id));
    return out.filter(t=>roots.has(t.id));
  }
  const allowed=(list:Target[])=>list.length?(['translate','rotate','scale'] as GizmoMode[]).filter(mode=>list.every(t=>t.modes.includes(mode))):[];

  function placePivot(list:Target[]) {
    const center=new THREE.Vector3();for(const t of list)center.add(t.position);center.divideScalar(list.length);
    pivot.position.copy(center);pivot.quaternion.identity();pivot.scale.set(1,1,1);
    if(state.space==='local'&&list.length===1)list[0]!.world.decompose(new THREE.Vector3(),pivot.quaternion,new THREE.Vector3());
    pivot.updateMatrixWorld(true);
  }

  /* ── drawing ─────────────────────────────────────────── */
  function syncCamera(world:SceneRuntime|null) {
    const width=Math.max(2,stage.clientWidth),height=Math.max(2,stage.clientHeight);
    renderer.setSize(width,height,false);
    if(!world)return false;
    // A private copy: the view offset extends the camera frustum to the whole
    // stage so handles stay visible past the composition edge.
    const view=viewportCamera(PM,world.camera).clone() as THREE.PerspectiveCamera|THREE.OrthographicCamera;
    if(view instanceof THREE.PerspectiveCamera)view.aspect=rect.width/rect.height;
    view.setViewOffset(rect.width,rect.height,-rect.x,-rect.y,width,height);
    view.updateProjectionMatrix();view.updateMatrixWorld(true);
    camera=view;controls.camera=view;
    return true;
  }
  function syncOutlines(world:SceneRuntime|null) {
    const wanted=new Set<string>();
    const outlined=new Set(ids.flatMap(id=>isModelGroup(PM,layer(id))?(PM.curComp?.()||PM.proj).layers.filter((l:any)=>layer3DRole(l)==='object'&&(PM.groupAncestors?.(l)||[]).some((g:any)=>g.id===id)).map((l:any)=>l.id):[id]));
    for(const id of outlined) {
      const object=world?.objects.get(id);if(!object||layer3DRole(layer(id))!=='object')continue;
      wanted.add(id);
      let outline=outlines.get(id);
      if(!outline){outline=new THREE.BoxHelper(object,accent);(outline.material as THREE.LineBasicMaterial).depthTest=false;outlines.set(id,outline);scene.add(outline);}
      else{outline.setFromObject(object);(outline.material as THREE.LineBasicMaterial).color.copy(accent);}
    }
    for(const [id,outline] of outlines)if(!wanted.has(id)){outline.removeFromParent();outline.dispose();outlines.delete(id);}
  }
  function draw() {
    frame=0;if(disposed)return;
    try{renderer.setClearColor(0,0);renderer.clear();renderer.render(scene,camera);}catch(error){warn(error);}
  }
  const request=()=>{if(!frame&&!disposed)frame=requestAnimationFrame(draw);};

  /* ── gesture ─────────────────────────────────────────── */
  const command=(id:string,path:string,value:number)=>({type:'set_property',target:id,path,value:tidy(value),
    time:PM.time,mode:PM.autokey?'keyframe':'auto',preserveHandEdits:false,markIntent:'human'});
  function apply() {
    if(!gesture)return;
    pivot.updateMatrixWorld(true);
    let current=pivot.matrixWorld.clone();
    if(fine) {
      // Shift: a tenth of the pointer's motion, measured from the start.
      const [p0,q0,s0]=[new THREE.Vector3(),new THREE.Quaternion(),new THREE.Vector3()],[p1,q1,s1]=[new THREE.Vector3(),new THREE.Quaternion(),new THREE.Vector3()];
      gesture.pivotStart.decompose(p0,q0,s0);current.decompose(p1,q1,s1);
      current=new THREE.Matrix4().compose(p0.lerp(p1,.1),q0.slerp(q1,.1),s0.lerp(s1,.1));
    }
    const delta=current.multiply(gesture.pivotInverse),commands:any[]=[];
    const rotation=new THREE.Quaternion();delta.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
    for(const t of gesture.start) {
      if(t.role==='object' || t.role==='group') {
        const local=t.parentInverse.clone().multiply(delta.clone().multiply(t.world));
        const position=new THREE.Vector3(),quaternion=new THREE.Quaternion(),scale=new THREE.Vector3();local.decompose(position,quaternion,scale);
        const anchor=vec(['x','y','z'].map(axis=>channel(t.layer,`anchor.${axis}`)));
        position.add(anchor.applyMatrix3(new THREE.Matrix3().setFromMatrix4(local)));
        const orientation=new THREE.Quaternion().setFromEuler(new THREE.Euler(...t.orientation.toArray().map(THREE.MathUtils.degToRad) as [number,number,number],'ZYX'));
        quaternion.multiply(orientation.invert());
        const euler=new THREE.Euler().setFromQuaternion(quaternion,'ZYX'),deg=THREE.MathUtils.radToDeg;
        // The runtime adds native orientation to rotation; write back rotation alone.
        const values:[string,number][]=[['position.x',position.x],['position.y',position.y],['position.z',position.z],
          ['rotation.x',deg(euler.x)],['rotation.y',deg(euler.y)],['rotation',deg(euler.z)],
          ['scale.x',scale.x*100],['scale.y',scale.y*100],['scale.z',scale.z*100]];
        for(const [path,value] of values)if(gesture.mode==='translate'?path.startsWith('position.'):gesture.mode==='rotate'?path.startsWith('rotation') || path.startsWith('position.'):path.startsWith('scale.') || path.startsWith('position.'))commands.push(command(t.id,path,value));
        continue;
      }
      const prefix=t.role==='light'?'light':'camera';
      const position=t.position.clone().applyMatrix4(delta);
      if(gesture.mode==='translate'){
        const motion=position.clone().sub(t.position).applyMatrix3(new THREE.Matrix3().setFromMatrix4(t.parentInverse));
        const localPosition=vec(['x','y','z'].map(axis=>channel(t.layer,`position.${axis}`))).add(motion);
        for(const [axis,value] of [['x',localPosition.x],['y',localPosition.y],['z',localPosition.z]] as const)commands.push(command(t.id,`position.${axis}`,value));
      }
      if(t.target) {
        // Moving keeps the aim point (Alt moves it along); rotating swings it around the position.
        const target=gesture.mode==='rotate'?position.clone().add(t.target.clone().sub(t.position).applyQuaternion(rotation))
          :alt?t.target.clone().add(position.clone().sub(t.position)):null;
        if(target){target.applyMatrix4(t.parentInverse);for(const [axis,value] of [['X',target.x],['Y',target.y],['Z',target.z]] as const)commands.push(command(t.id,`${prefix}.target${axis}`,value));}
      }
    }
    if(!commands.length)return;
    const result=PM.Edit.apply(commands,{origin:'canvas',label:gesture.label});
    if(!result.ok){warn(result.message);cancel();}
  }
  function setSnap(on:boolean) {
    if(on===snapping)return;snapping=on;
    controls.setTranslationSnap(on?SNAP.translate:null);controls.setRotationSnap(on?THREE.MathUtils.degToRad(SNAP.rotate):null);controls.setScaleSnap(on?SNAP.scale:null);
  }
  function modifiers(event:{ctrlKey:boolean;shiftKey:boolean;altKey:boolean}){setSnap(event.ctrlKey);fine=event.shiftKey;alt=event.altKey;}
  const pointer=(clientX:number,clientY:number,button=0)=>{
    const bounds=stage.getBoundingClientRect();
    return {x:(clientX-bounds.left)/Math.max(1,bounds.width)*2-1,y:-(clientY-bounds.top)/Math.max(1,bounds.height)*2+1,button};
  };
  let dragOffs:(()=>void)[]=[];
  function endDrag(){for(const off of dragOffs.splice(0))off();}
  function stopControls(){if(controls.dragging){controls.dragging=false;}controls.axis=null;}
  function finish() {
    if(!gesture)return;const label=gesture.label;
    suppressContextUntil=performance.now()+500;
    gesture=null;endDrag();stopControls();
    try{PM.Edit.commit(label);}catch(error){warn(error);}
    refresh();
  }
  function cancel() {
    if(!gesture)return;
    suppressContextUntil=performance.now()+500;
    gesture=null;endDrag();stopControls();
    try{PM.Edit.cancel();}catch(error){warn(error);}
    refresh();
  }
  controls.addEventListener('objectChange',()=>apply());

  /* ── public controller ───────────────────────────────── */
  function refresh() {
    if(disposed)return;
    const world=runtime(PM.time);
    if(!syncCamera(world)){controls.detach();syncOutlines(null);request();return;}
    accent=themeColor(stage,'--accent',0xff7a33);
    syncOutlines(world);
    if(!gesture) {
      targets=resolveTargets(world);
      const modes=allowed(targets);
      if(targets.length&&modes.includes(state.mode)){placePivot(targets);controls.setMode(state.mode);controls.setSpace(state.space);controls.attach(pivot);}
      else controls.detach();
    }
    request();
  }
  const offState=onGizmoState(()=>refresh());
  listen(window,'keydown',(event:KeyboardEvent)=>{
    if(!gesture)return;
    if(event.key==='Escape'){event.preventDefault();event.stopImmediatePropagation();cancel();return;}
    modifiers(event);
  },true);
  listen(window,'keyup',(event:KeyboardEvent)=>{if(gesture)modifiers(event);},true);
  listen(window,'blur',()=>cancel());
  // macOS turns Control-click into a context menu, including snap drags.
  // The menu event can arrive after pointerup, when the gesture has committed.
  listen(stage,'contextmenu',(event:MouseEvent)=>{
    if(!gesture && performance.now()>suppressContextUntil)return;
    event.preventDefault();event.stopImmediatePropagation();
    if(gesture && !event.ctrlKey)cancel();
  },true);

  return {
    update(next,selection) {
      rect={x:next.x,y:next.y,width:Math.max(1,next.width),height:Math.max(1,next.height)};
      if(selection)ids=selection.length && selection.every(id=>!!roleOf(layer(id)))?[...selection]:[];
      refresh();
    },
    hover(clientX,clientY) {
      if(gesture)return true;
      if(!controls.object)return false;
      controls.pointerHover(pointer(clientX,clientY) as any);canvas.dataset.gizmoAxis=controls.axis || '';request();
      return controls.axis!==null;
    },
    pointerDown(event) {
      if(gesture||event.button!==0||!controls.object)return false;
      const start=pointer(event.clientX,event.clientY,event.button);
      controls.pointerHover(start as any);
      if(controls.axis===null)return false;
      const mode=state.mode,count=targets.length;
      if(!count || ids.some(id=>!editable(layer(id))))return false;
      try{PM.Edit.begin(LABEL[mode][count===1?0:1],{origin:'canvas'});}catch(error){warn(error);return false;}
      const upgrades:any[]=[];
      for(const t of targets.filter(t=>t.role==='group'&&!t.layer.threeD)){
        const anchor=t.position.clone().applyMatrix4(t.world.clone().invert());
        const old=vec(['x','y','z'].map(axis=>channel(t.layer,`anchor.${axis}`)));
        const local=t.parentInverse.clone().multiply(t.world),motion=anchor.clone().sub(old).applyMatrix3(new THREE.Matrix3().setFromMatrix4(local));
        const position=vec(['x','y','z'].map(axis=>channel(t.layer,`position.${axis}`))).add(motion);
        upgrades.push({type:'set_layer',target:t.id,patch:{threeD:true}});
        for(const [axis,value] of [['x',anchor.x],['y',anchor.y],['z',anchor.z]] as const)upgrades.push(command(t.id,`anchor.${axis}`,value));
        for(const [axis,value] of [['x',position.x],['y',position.y],['z',position.z]] as const)upgrades.push(command(t.id,`position.${axis}`,value));
      }
      if(upgrades.length){const result=PM.Edit.apply(upgrades,{origin:'canvas'});if(!result.ok){PM.Edit.cancel();warn(result.message);return false;}targets=resolveTargets(runtime(PM.time));}
      pivot.updateMatrixWorld(true);
      gesture={label:LABEL[mode][count===1?0:1],start:targets,mode,pivotStart:pivot.matrixWorld.clone(),pivotInverse:pivot.matrixWorld.clone().invert()};
      modifiers(event);
      controls.pointerDown(start as any);
      if(!controls.dragging){cancel();return false;}
      const move=(e:PointerEvent)=>{if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();modifiers(e);controls.pointerMove(pointer(e.clientX,e.clientY,-1) as any);request();};
      const up=(e:PointerEvent)=>{if(!gesture)return;e.preventDefault();e.stopImmediatePropagation();controls.pointerUp(pointer(e.clientX,e.clientY,0) as any);finish();};
      const lost=()=>cancel();
      window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);window.addEventListener('pointercancel',lost,true);
      dragOffs=[()=>window.removeEventListener('pointermove',move,true),()=>window.removeEventListener('pointerup',up,true),()=>window.removeEventListener('pointercancel',lost,true)];
      event.preventDefault();event.stopImmediatePropagation();
      return true;
    },
    active:()=>!!gesture,
    covers(list) {
      return list.length>0&&list.every(id=>!!roleOf(layer(id)));
    },
    cancel,
    dispose() {
      if(disposed)return;cancel();disposed=true;cancelAnimationFrame(frame);offState();
      for(const off of cleanup.splice(0))off();
      for(const outline of outlines.values())outline.dispose();
      controls.detach();controls.dispose();renderer.dispose();renderer.forceContextLoss();canvas.remove();
    }
  };
}
