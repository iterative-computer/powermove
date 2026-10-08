import * as THREE from 'three';
import {compositionRuntime} from './service';
import {layer3DRole} from './layers';
import {viewportCamera,viewportState,getViewportMode,activeCameraLayer,viewportPose,setViewAxis,setViewportMode,onViewportChange,OPPOSITE_VIEW,type ViewAxis} from './viewport';
import {viewportPreferences,setViewportPreferences,onViewportSettings,selected3D,isViewportItem} from './editor-state';
import {resolveTargets,aimPoint,editableLayer,propertyCommand,roleOf} from './targets';
import {ModalTransform,fitCamera,toScreen,rayAt,viewForward,type ModalMode,type Rect,type Point,type Axis} from './modal';
import {ViewportOverlay,type Glyph} from './overlay';
import {ViewportChrome} from './chrome';
import {computeGizmo,hitGizmo,CURSORS,type GizmoGeometry,type GizmoHandle} from './handles';
import {createSceneNavigation,frameComposition,alignCameraToView,type SceneNavigation} from './navigation';
import {onRenderState} from './rendering';
import {sceneFromComp,compFromScene,lengthToScene,PX_PER_UNIT} from './space';
import {world3D,parent3D} from '../../legacy/core/space-3d';

/*
 * The 3D view over the composition viewer. It appears for any composition
 * with 3D layers (models, lights, cameras, or 2D layers with 3D on) and works
 * like 2D: click selects, dragging moves, the selection box scales and turns,
 * and the axis handles move along one axis. Navigation is a small cluster in
 * the corner, so the preview keeps its size.
 */

export interface ViewportHost {
  panComposition(dx:number,dy:number):void;
  zoomComposition(factor:number,clientX:number,clientY:number):void;
  /** The active editor tool ('select', 'rotate', …): Rotate shows rotation rings. */
  tool():string;
}
export interface ViewportController {
  /** Follow the composition rectangle (stage-relative CSS px). */
  update(rect:Rect,selection?:readonly string[]):void;
  /** Capture-phase press: navigation, the selection gizmo, light and camera wires. */
  pointerDown(event:PointerEvent):boolean;
  /** A press on a 3D layer the viewer picked: select it like 2D, and drag to move. */
  objectPress(event:PointerEvent,layerId:string):boolean;
  /** A press inside the 3D selection box (over no layer): drag moves the selection, as in 2D. */
  selectionPress(event:PointerEvent):boolean;
  /** True over a gizmo handle (highlights it). */
  hover(clientX:number,clientY:number):boolean;
  /** The stage cursor the 3D view wants at this point, or null. */
  cursorAt(clientX:number,clientY:number):string|null;
  wheel(event:WheelEvent):boolean;
  /** Last pointer position over the viewer, in client coordinates. */
  pointer():{clientX:number;clientY:number}|null;
  covers(ids:readonly string[]):boolean;
  /** Light and camera wires inside a composition-space box (box select). */
  glyphsIn(box:{x0:number;y0:number;x1:number;y1:number}):string[];
  active():boolean;
  cancel():void;
  dispose():void;
}

const controllers=new WeakMap<object,ViewportController>();
export const viewportController=(PM:any)=>controllers.get(PM)??null;
const comp=(PM:any)=>PM.curComp?.()||PM.proj;
/** Any 3D layer turns on the 3D view: models, lights, cameras and 2D layers with 3D on. */
export const hasScene3D=(PM:any)=>!!comp(PM)?.layers?.some((l:any)=>!!roleOf(PM,l));
/** The 2D selection ink: the inverse of the composition background. */
export function selectionInk(project:any):string {
  const fill=project?.backgroundFill,color=fill?.type!=='none'&&fill?.stops?.[0]?.color||project?.bg||'#000000';
  const hex=/^#[0-9a-f]{6}(?:[0-9a-f]{2})?$/i.test(color)?color.slice(1,7):'000000';
  return '#'+[0,2,4].map(i=>(255-parseInt(hex.slice(i,i+2),16)).toString(16).padStart(2,'0')).join('');
}
const DRAG_THRESHOLD=3;

export function attachViewport(PM:any,stage:HTMLElement,host:ViewportHost):ViewportController {
  const overlay=new ViewportOverlay(stage);
  let rect:Rect={x:0,y:0,width:1,height:1},pointer:Point|null=null,client:{clientX:number;clientY:number}|null=null;
  let modal:ModalTransform|null=null,modalOffs:(()=>void)[]=[],aiming=false,frame=0,disposed=false;
  let glyphHits:{id:string;x:number;y:number}[]=[],gizmo:GizmoGeometry|null=null,hot:GizmoHandle|null=null;
  const offs:(()=>void)[]=[];
  const listen=(target:EventTarget,type:string,handler:any,options?:any)=>{target.addEventListener(type,handler,options);offs.push(()=>target.removeEventListener(type,handler,options));};
  const warn=(error:unknown)=>PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:'scene3d-viewport',error:true});
  const stagePoint=(clientX:number,clientY:number):Point=>{const bounds=stage.getBoundingClientRect();return {x:clientX-bounds.left,y:clientY-bounds.top};};
  const available=()=>!disposed&&hasScene3D(PM);
  const viewCamera=()=>fitCamera(viewportCamera(PM,compositionRuntime(PM).camera),rect);
  /** The camera you look through is the view itself; move it from a free view or with "Camera follows view". */
  const excluded=()=>getViewportMode(PM)==='camera'?activeCameraLayer(PM)?.id??null:null;
  const targets=()=>resolveTargets(PM,selected3D(PM),compositionRuntime(PM),{excludeCamera:excluded()});

  const navigation:SceneNavigation=createSceneNavigation(PM,stage,{
    height:()=>rect.height,panComposition:host.panComposition,zoomComposition:host.zoomComposition
  },available);
  const chrome=new ViewportChrome(stage,{
    drag:(event,kind)=>navigation.drag(event,kind),
    toggleCamera:()=>{try{setViewportMode(PM,getViewportMode(PM)==='camera'?'editor':'camera');}catch(error){warn(error);}},
    align:(axis:ViewAxis)=>{try{const view=viewportState(PM);setViewAxis(PM,view.mode==='editor'&&view.axis===axis?OPPOSITE_VIEW[axis]:axis);}catch(error){warn(error);}},
    menu:(anchor)=>PM.menu?.(anchor,viewMenu())
  });
  function viewMenu(){
    const view=viewportState(PM),prefs=viewportPreferences(PM),hasCamera=!!activeCameraLayer(PM);
    const run=(fn:()=>unknown)=>()=>{try{fn();}catch(error){warn(error);}};
    const look=(axis:ViewAxis,label:string)=>({label,on:view.mode==='editor'&&view.axis===axis,run:run(()=>setViewAxis(PM,axis))});
    return [
      {label:'Camera view',on:view.mode==='camera',run:run(()=>setViewportMode(PM,'camera'))},
      {label:'Free view',on:view.mode==='editor'&&!view.axis,run:run(()=>setViewportMode(PM,'editor'))},
      '-',look('front','Front'),look('top','Top'),look('right','Side'),
      '-',
      {label:'Frame selection',disabled:!selected3D(PM).length,run:run(()=>frameComposition(PM,true))},
      {label:'Frame all',run:run(()=>frameComposition(PM,false))},
      '-',
      {label:'Move camera to this view',disabled:!hasCamera||view.mode==='camera',run:run(()=>alignCameraToView(PM))},
      {label:'Camera follows view',on:prefs.lockCamera,disabled:!hasCamera,run:run(()=>setViewportPreferences(PM,{lockCamera:!prefs.lockCamera}))},
      {label:'Show guides',on:prefs.overlays,run:run(()=>setViewportPreferences(PM,{overlays:!prefs.overlays}))}
    ];
  }

  /* ── drawing ─────────────────────────────────────────── */
  const schedule=()=>{if(!frame&&!disposed)frame=requestAnimationFrame(render);};
  /** Hit tests must see the latest tool, selection and view, even before the next frame draws. */
  const flush=()=>{if(frame){cancelAnimationFrame(frame);render();}};
  function glyphs(camera:THREE.Camera):Glyph[] {
    const list:Glyph[]=[],hidden=excluded(),selected=new Set(PM.sel?.layers||[]),toScene=sceneFromComp(comp(PM));glyphHits=[];
    for(const layer of comp(PM).layers){
      const role=layer3DRole(layer);if((role!=='light'&&role!=='camera')||!PM.active(layer,PM.time)||layer.id===hidden)continue;
      const world=toScene.clone().multiply(new THREE.Matrix4().fromArray(world3D(PM,layer,PM.time))),position=new THREE.Vector3().setFromMatrixPosition(world);
      const state=selected.has(layer.id)?'selected':'none';
      const evaluate=(key:string)=>{const prop=layer.d.data[role].p?.[key];return prop?Number(PM.evP(layer,prop,PM.time,`${role}.${key}`)):undefined;};
      if(role==='camera')list.push({id:layer.id,kind:'camera',position,target:aimPoint(PM,layer),up:new THREE.Vector3(0,-1,0).transformDirection(world),fov:evaluate('fov')??50,aspect:(comp(PM).w||16)/Math.max(1,comp(PM).h||9),state});
      else {const type=layer.d.data.light.type;list.push({id:layer.id,kind:type==='sun'||type==='spot'||type==='area'?type:'point',position,target:aimPoint(PM,layer),angle:evaluate('angle'),
        size:[lengthToScene(evaluate('width')??PX_PER_UNIT),lengthToScene(evaluate('height')??PX_PER_UNIT)],state});}
      const screen=toScreen(camera,rect,position);if(screen.z<1)glyphHits.push({id:layer.id,x:screen.x,y:screen.y});
    }
    return list;
  }
  function render(){
    frame=0;if(disposed)return;
    try{
      const prefs=viewportPreferences(PM),visible=available(),ink=selectionInk(PM.proj);
      if(!visible){
        gizmo=null;
        overlay.draw({camera:new THREE.PerspectiveCamera(),rect,runtime:null,userView:false,axis:null,distance:1,fov:50,overlays:false,ink,frame:[],floor:0,glyphs:[],guides:[]});
        chrome.update({visible:false,rotation:new THREE.Quaternion(),axis:null,camera:true,ink,gizmo:null,hover:null,readout:null});
        return;
      }
      const view=viewportState(PM),runtime=compositionRuntime(PM),camera=viewCamera(),pose=viewportPose(PM),c=comp(PM),toScene=sceneFromComp(c);
      const corners=[[0,0],[c.w,0],[c.w,c.h],[0,c.h]].map(([x,y])=>new THREE.Vector3(x,y,0).applyMatrix4(toScene));
      overlay.draw({camera,rect,runtime,userView:view.mode==='editor',axis:view.axis,distance:pose.distance,fov:view.fov,overlays:prefs.overlays,ink,
        frame:corners,floor:corners[2]!.y,glyphs:prefs.overlays?glyphs(camera):[],guides:modal?.guides??[]});
      // The selection gizmo appears whenever only 3D layers are selected, as with 2D selection boxes.
      const ids=PM.sel?.layers||[];
      gizmo=!aiming&&ids.length&&ids.every((id:string)=>isViewportItem(PM,PM.L?.(id)))&&!navigation.active()?computeGizmo(PM,targets(),camera,rect,runtime,host.tool()):null;
      const rotation=new THREE.Quaternion();camera.matrixWorld.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
      chrome.update({visible:true,rotation,axis:view.axis,camera:view.mode==='camera',ink,gizmo:modal?{...gizmo!,axes:[],rings:[],aims:[],stem:null}:gizmo,hover:hot,
        readout:modal&&pointer?{text:modal.readout(),at:pointer}:null});
    }catch(error){console.error('[scene3d] 3D view overlay failed',error);}
  }

  /* ── drags ───────────────────────────────────────────── */
  function endModal(){for(const off of modalOffs.splice(0))off();modal=null;schedule();PM.bus?.emit?.('scene3d:viewport');}
  /** Begin a drag that started at `start`; the release confirms it. */
  function startDrag(mode:ModalMode,start:Point,options:{constraint?:{axis:Axis}}={}):boolean {
    if(modal||navigation.active())return false;
    const list=targets();if(!list.length)return false;
    try{
      modal=ModalTransform.begin({PM,camera:viewCamera,rect:()=>rect,changed:()=>{schedule();PM.bus?.emit?.('scene3d:viewport');},finished:()=>endModal()},mode,list,start,options);
    }catch(error){warn(error);return false;}
    if(!modal)return false;
    const current=modal;
    const capture=(type:string,handler:any)=>{window.addEventListener(type,handler,true);modalOffs.push(()=>window.removeEventListener(type,handler,true));};
    capture('pointermove',(e:PointerEvent)=>{const p=stagePoint(e.clientX,e.clientY);pointer=p;client={clientX:e.clientX,clientY:e.clientY};current.move(p,e);});
    capture('pointerup',(e:PointerEvent)=>{if(e.button!==0)return;e.preventDefault();e.stopImmediatePropagation();current.confirm();});
    capture('pointerdown',(e:PointerEvent)=>{e.preventDefault();e.stopImmediatePropagation();if(e.button===2)current.cancel();});
    capture('keydown',(e:KeyboardEvent)=>{if(current.key(e)){e.preventDefault();e.stopImmediatePropagation();}else current.modifiers(e);});
    capture('keyup',(e:KeyboardEvent)=>current.modifiers(e));
    capture('contextmenu',(e:MouseEvent)=>{e.preventDefault();e.stopImmediatePropagation();});
    capture('blur',()=>current.cancel());
    schedule();PM.bus?.emit?.('scene3d:viewport');
    return true;
  }
  /** Arm a press: a drag past the threshold runs `drag`, a release without one runs `click`. */
  function press(event:PointerEvent,drag:(start:Point,e:PointerEvent)=>void,click:()=>void=()=>{}){
    const start=stagePoint(event.clientX,event.clientY);let done=false;
    const cleanup=()=>{done=true;window.removeEventListener('pointermove',move,true);window.removeEventListener('pointerup',up,true);window.removeEventListener('pointercancel',cleanup,true);};
    const move=(e:PointerEvent)=>{if(done)return;const p=stagePoint(e.clientX,e.clientY);if(Math.hypot(p.x-start.x,p.y-start.y)<DRAG_THRESHOLD)return;cleanup();drag(start,e);};
    const up=()=>{if(done)return;cleanup();click();};
    window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);window.addEventListener('pointercancel',cleanup,true);
    event.preventDefault();event.stopImmediatePropagation();
  }
  /** Drag a light's or camera's aim point across the plane facing the view. */
  function aimDrag(event:PointerEvent,id:string){
    const layer=PM.L(id),role=layer3DRole(layer);
    if((role!=='light'&&role!=='camera')||!editableLayer(PM,layer))return;
    const camera=viewCamera(),origin=aimPoint(PM,layer);if(!origin)return;
    const normal=viewForward(camera).negate(),label=role==='light'?'Aim light':'Aim camera';
    const fromScene=compFromScene(comp(PM)),parentInverse=new THREE.Matrix4().fromArray(parent3D(PM,layer,PM.time)).invert();
    try{PM.Edit.begin(label,{origin:'canvas'});}catch(error){warn(error);return;}
    aiming=true;
    const move=(e:PointerEvent)=>{
      e.preventDefault();e.stopImmediatePropagation();
      const hit=rayAt(camera,rect,stagePoint(e.clientX,e.clientY)).intersectPlane(new THREE.Plane().setFromNormalAndCoplanarPoint(normal,origin),new THREE.Vector3());
      if(!hit)return;
      const local=hit.applyMatrix4(fromScene).applyMatrix4(parentInverse);
      const result=PM.Edit.apply((['X','Y','Z'] as const).map(axis=>propertyCommand(PM,id,`${role}.target${axis}`,local[axis.toLowerCase() as 'x'|'y'|'z'])),{origin:'canvas',label});
      if(!result.ok){warn(result.message);finish(false);}
      schedule();
    };
    const finish=(commit:boolean)=>{
      window.removeEventListener('pointermove',move,true);window.removeEventListener('pointerup',up,true);window.removeEventListener('keydown',key,true);
      aiming=false;try{commit?PM.Edit.commit(label):PM.Edit.cancel();}catch(error){warn(error);}schedule();
    };
    const up=(e:PointerEvent)=>{e.preventDefault();e.stopImmediatePropagation();finish(true);};
    const key=(e:KeyboardEvent)=>{if(e.key==='Escape'){e.preventDefault();e.stopImmediatePropagation();finish(false);}};
    window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);window.addEventListener('keydown',key,true);
    event.preventDefault();event.stopImmediatePropagation();
  }
  function handleDrag(event:PointerEvent,handle:GizmoHandle){
    if(handle.kind==='aim'){aimDrag(event,handle.id);return;}
    press(event,start=>{
      if(handle.kind==='corner')startDrag('scale',start);
      else if(handle.kind==='rotate')startDrag('rotate',start);
      else if(handle.kind==='axis')startDrag('translate',start,{constraint:{axis:handle.axis}});
      else startDrag('rotate',start,{constraint:{axis:handle.axis}});
    });
  }
  /** 2D selection semantics: click selects (Shift toggles); dragging moves the selection. */
  const inside=(box:Point[],p:Point)=>{let hit=false;for(let i=0,j=box.length-1;i<box.length;j=i++){const a=box[i]!,b=box[j]!;if((a.y>p.y)!==(b.y>p.y)&&p.x<(b.x-a.x)*(p.y-a.y)/(b.y-a.y)+a.x)hit=!hit;}return hit;};
  function selectPress(event:PointerEvent,id:string){
    const current:string[]=[...(PM.sel?.layers||[])],selected=current.includes(id);
    if(event.shiftKey&&selected){PM.selectLayers?.(current.filter(x=>x!==id));event.preventDefault();event.stopImmediatePropagation();return;}
    if(!selected)PM.selectLayers?.(event.shiftKey?[...current,id]:[id]);
    flush();
    // With the Rotate tool, dragging turns the selection in the view; otherwise it moves.
    press(event,(start,e)=>{if(startDrag(host.tool()==='rotate'?'rotate':'translate',start))modal?.move(stagePoint(e.clientX,e.clientY),e);},
      ()=>{if(selected&&!event.shiftKey&&current.length>1)PM.selectLayers?.([id]);});
  }

  /* ── stage events ────────────────────────────────────── */
  listen(stage,'pointermove',(e:PointerEvent)=>{pointer=stagePoint(e.clientX,e.clientY);client={clientX:e.clientX,clientY:e.clientY};});
  listen(stage,'pointerleave',()=>{client=null;if(hot){hot=null;schedule();}});
  offs.push(onViewportChange(PM,()=>schedule()),onViewportSettings(PM,()=>schedule()),onRenderState(PM,()=>schedule()));
  for(const event of ['selection','sel','time','project','layers','composition','tool']){const off=PM.bus?.on?.(event,()=>schedule());if(typeof off==='function')offs.push(off);}

  const controller:ViewportController={
    update(next){rect={x:next.x,y:next.y,width:Math.max(1,next.width),height:Math.max(1,next.height)};schedule();},
    pointerDown(event){
      if(modal||aiming)return true;
      if(!available())return false;
      if(navigation.pointerDown(event))return true;
      if(event.button!==0)return false;
      flush();
      const p=stagePoint(event.clientX,event.clientY),handle=hitGizmo(gizmo,p);
      if(handle){handleDrag(event,handle);return true;}
      const glyph=viewportPreferences(PM).overlays?glyphHits.filter(g=>Math.hypot(g.x-p.x,g.y-p.y)<=12).sort((a,b)=>Math.hypot(a.x-p.x,a.y-p.y)-Math.hypot(b.x-p.x,b.y-p.y))[0]:null;
      if(glyph){selectPress(event,glyph.id);return true;}
      return false;
    },
    objectPress(event,id){if(!available()||event.button!==0)return false;selectPress(event,id);return true;},
    selectionPress(event){
      if(!available()||event.button!==0||modal||aiming)return false;
      flush();const p=stagePoint(event.clientX,event.clientY);
      if(!gizmo?.box||!inside(gizmo.box,p))return false;
      press(event,(start,e)=>{if(startDrag(host.tool()==='rotate'?'rotate':'translate',start))modal?.move(stagePoint(e.clientX,e.clientY),e);});
      return true;
    },
    hover(clientX,clientY){flush();const handle=hitGizmo(gizmo,stagePoint(clientX,clientY));if(JSON.stringify(handle)!==JSON.stringify(hot)){hot=handle;schedule();}return !!handle;},
    cursorAt(clientX,clientY){
      if(!available())return null;
      if(modal)return modal.mode==='translate'?'move':'grabbing';
      return this.hover(clientX,clientY)?CURSORS[hot!.kind]:null;
    },
    wheel(event){if(modal||aiming){event.preventDefault();return true;}return navigation.wheel(event);},
    pointer:()=>client,
    covers:ids=>ids.length>0&&ids.every(id=>isViewportItem(PM,PM.L?.(id))),
    glyphsIn(box){
      const c=comp(PM),sx=(x:number)=>rect.x+x/Math.max(1,c.w)*rect.width,sy=(y:number)=>rect.y+y/Math.max(1,c.h)*rect.height;
      const x0=Math.min(sx(box.x0),sx(box.x1)),x1=Math.max(sx(box.x0),sx(box.x1)),y0=Math.min(sy(box.y0),sy(box.y1)),y1=Math.max(sy(box.y0),sy(box.y1));
      return viewportPreferences(PM).overlays?glyphHits.filter(g=>g.x>=x0&&g.x<=x1&&g.y>=y0&&g.y<=y1).map(g=>g.id):[];
    },
    active:()=>!!modal||aiming||navigation.active(),
    cancel(){modal?.cancel();navigation.cancel();},
    dispose(){
      if(disposed)return;modal?.cancel();disposed=true;cancelAnimationFrame(frame);
      for(const off of offs.splice(0))off();
      navigation.dispose();chrome.dispose();overlay.dispose();
      if(controllers.get(PM)===controller)controllers.delete(PM);
    }
  };
  controllers.set(PM,controller);
  schedule();
  return controller;
}
