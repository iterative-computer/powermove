import * as THREE from 'three';
import {compositionRuntime} from './service';
import {layer3DRole,isModelGroup} from './layers';
import {viewportCamera,viewportState,viewLabel,getViewportMode,activeCameraLayer,viewportPose,setViewAxis,setViewportMode,toggleViewProjection,onViewportChange,applyViewportPose,localViewIds,OPPOSITE_VIEW,type ViewAxis} from './viewport';
import {viewportPreferences,onViewportSettings,selected3D,activeObject,cursorLocation,setCursorLocation,isViewportItem,type ViewportTool} from './editor-state';
import {resolveTargets,aimPoint,type TransformMode} from './targets';
import {ModalTransform,fitCamera,toScreen,rayAt,viewForward,type ModalMode,type Rect,type Point} from './modal';
import {ViewportOverlay,type Glyph} from './overlay';
import {ViewportChrome} from './chrome';
import {createTransformGizmo,type TransformGizmo} from './gizmo';
import {createSceneNavigation,lockedCamera,writeCameraPose,type SceneNavigation} from './navigation';
import {clickSelect,placeCursor} from './operators';
import {onRenderState} from './rendering';
import {startWalk,type WalkSession} from './walk';
import {editModeLayer,editOverlay,pointsTarget,planExtrude,selectPoints} from './editmode';
import {world3D} from '../../legacy/core/space-3d';

/*
 * One Blender 3D viewport over the composition viewer. It appears only when
 * the composition contains 3D layers. Blender keys apply while the pointer is
 * over the viewer and no 2D layers are selected (see `inViewportContext`).
 */

export interface ViewportHost {
  panComposition(dx:number,dy:number):void;
  zoomComposition(factor:number,clientX:number,clientY:number):void;
}
export interface ViewportController {
  /** Follow the composition rectangle (stage-relative CSS px). */
  update(rect:Rect,selection?:readonly string[]):void;
  /** Capture-phase press: modal confirm/cancel, navigation, gizmo, light/camera wires, cursor tool. */
  pointerDown(event:PointerEvent):boolean;
  /** A press on a model the viewer picked: Blender click selection, and Tweak drags. */
  objectPress(event:PointerEvent,layerId:string):boolean;
  /** True over a gizmo handle (highlights it). */
  hover(clientX:number,clientY:number):boolean;
  /** The stage cursor the viewport wants at this point, or null. */
  cursorAt(clientX:number,clientY:number):string|null;
  wheel(event:WheelEvent):boolean;
  contextMenu(event:MouseEvent):boolean;
  startTransform(mode:ModalMode):boolean;
  /** B: the next left drag box-selects, even when it starts on a model. */
  armBoxSelect():boolean;
  /** Shift+B: zoom the view to a dragged rectangle. */
  armZoomBorder():boolean;
  /** Shift+`: Blender's Walk Navigation. */
  startWalk():boolean;
  /** Edit Mode E: extrude the selection and move it along its normal. */
  extrude():boolean;
  takeBoxSelect():boolean;
  modalActive():boolean;
  /** Last pointer position over the viewer, in client coordinates. */
  pointer():{clientX:number;clientY:number}|null;
  covers(ids:readonly string[]):boolean;
  /** Light and camera wires inside a composition-space box (box select). */
  glyphsIn(box:{x0:number;y0:number;x1:number;y1:number}):string[];
  active():boolean;
  cancel():void;
  dispose():void;
}

const controllers=new WeakMap<object,ViewportController&{over():boolean}>();
export const viewportController=(PM:any)=>controllers.get(PM)??null;
const comp=(PM:any)=>PM.curComp?.()||PM.proj;
export const hasScene3D=(PM:any)=>!!comp(PM)?.layers?.some((l:any)=>layer3DRole(l));
/** Blender's keymap applies to the 3D viewport under the pointer, never to 2D work. */
export function inViewportContext(PM:any):boolean {
  const controller=controllers.get(PM);
  if(!controller||!controller.over()||!hasScene3D(PM))return false;
  const focused=document.activeElement as HTMLElement|null;
  if(focused?.matches?.('input,textarea,select,[contenteditable="true"]'))return false;
  return (PM.sel?.layers||[]).every((id:string)=>isViewportItem(PM,PM.L?.(id)));
}
const TOOL_MODES:Partial<Record<ViewportTool,TransformMode>>={move:'translate',rotate:'rotate',scale:'scale'};
const DRAG_THRESHOLD=3;

export function attachViewport(PM:any,stage:HTMLElement,host:ViewportHost):ViewportController {
  const overlay=new ViewportOverlay(stage);
  let rect:Rect={x:0,y:0,width:1,height:1},over=false,pointer:Point|null=null,client:{clientX:number;clientY:number}|null=null;
  let modal:ModalTransform|null=null,modalOffs:(()=>void)[]=[],frame=0,disposed=false,suppressContextUntil=0;
  let glyphHits:{id:string;x:number;y:number}[]=[],boxArmed:(()=>void)|null=null,walk:WalkSession|null=null;
  let zoomArmed:(()=>void)|null=null,border:{x:number;y:number;width:number;height:number}|null=null,editHits:{x:number;y:number}[]=[];
  const offs:(()=>void)[]=[];
  const listen=(target:EventTarget,type:string,handler:any,options?:any)=>{target.addEventListener(type,handler,options);offs.push(()=>target.removeEventListener(type,handler,options));};
  const warn=(error:unknown)=>PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:'scene3d-viewport',error:true});
  const stagePoint=(clientX:number,clientY:number):Point=>{const bounds=stage.getBoundingClientRect();return {x:clientX-bounds.left,y:clientY-bounds.top};};
  const available=()=>!disposed&&hasScene3D(PM);
  const viewCamera=()=>fitCamera(viewportCamera(PM,compositionRuntime(PM).camera),rect);
  const excluded=()=>getViewportMode(PM)==='camera'?activeCameraLayer(PM)?.id??null:null;
  /** In Edit Mode transforms act on the selected points; otherwise on the selected layers. */
  const targets=()=>editModeLayer(PM)?[pointsTarget(PM)].filter(t=>!!t) as ReturnType<typeof resolveTargets>:resolveTargets(PM,selected3D(PM),compositionRuntime(PM),{excludeCamera:excluded()});

  const navigation:SceneNavigation=createSceneNavigation(PM,stage,{
    height:()=>rect.height,panComposition:host.panComposition,zoomComposition:host.zoomComposition
  },available);
  const gizmo:TransformGizmo=createTransformGizmo(PM,stage,overlay.canvas,()=>schedule(),targets);
  overlay.scene.add(gizmo.helper);
  const chrome=new ViewportChrome(stage,{
    drag:(event,kind)=>navigation.drag(event,kind),
    toggleCamera:()=>{try{setViewportMode(PM,getViewportMode(PM)==='camera'?'editor':'camera');}catch(error){warn(error);}},
    toggleProjection:()=>{try{toggleViewProjection(PM);}catch(error){warn(error);}},
    align:(axis:ViewAxis)=>{try{const view=viewportState(PM);setViewAxis(PM,view.mode==='editor'&&view.axis===axis?OPPOSITE_VIEW[axis]:axis);}catch(error){warn(error);}}
  });

  /* ── drawing ─────────────────────────────────────────── */
  const schedule=()=>{if(!frame&&!disposed)frame=requestAnimationFrame(render);};
  /** Hit tests must see the latest tool, selection and view, even before the next frame draws. */
  const flush=()=>{if(frame){cancelAnimationFrame(frame);render();}};
  function expand(ids:Iterable<string>):Set<string> {
    const out=new Set<string>();
    for(const id of ids){
      const layer=PM.L?.(id);
      if(isModelGroup(PM,layer)){for(const child of comp(PM).layers)if(layer3DRole(child)==='object'&&(PM.groupAncestors?.(child)||[]).some((g:any)=>g.id===id))out.add(child.id);}
      else if(layer3DRole(layer)==='object')out.add(id);
    }
    return out;
  }
  function glyphs(camera:THREE.Camera,selected:Set<string>,active:string|null):Glyph[] {
    const list:Glyph[]=[],hidden=excluded(),local=localViewIds(PM);glyphHits=[];
    for(const layer of comp(PM).layers){
      const role=layer3DRole(layer);if((role!=='light'&&role!=='camera')||!PM.active(layer,PM.time)||layer.id===hidden||(local&&!local.has(layer.id)))continue;
      const world=new THREE.Matrix4().fromArray(world3D(PM,layer,PM.time)),position=new THREE.Vector3().setFromMatrixPosition(world);
      const state=layer.id===active?'active':selected.has(layer.id)?'selected':'none';
      const evaluate=(key:string)=>{const prop=layer.d.data[role].p?.[key];return prop?Number(PM.evP(layer,prop,PM.time,`${role}.${key}`)):undefined;};
      if(role==='camera')list.push({id:layer.id,kind:'camera',position,target:aimPoint(PM,layer),up:new THREE.Vector3(0,1,0).transformDirection(world),fov:evaluate('fov')??50,aspect:(comp(PM).w||16)/Math.max(1,comp(PM).h||9),state});
      else {const type=layer.d.data.light.type;list.push({id:layer.id,kind:type==='sun'||type==='spot'||type==='area'?type:'point',position,target:aimPoint(PM,layer),angle:evaluate('angle'),size:[evaluate('width')??1,evaluate('height')??1],state});}
      const screen=toScreen(camera,rect,position);if(screen.z<1)glyphHits.push({id:layer.id,x:screen.x,y:screen.y});
    }
    return list;
  }
  function render(){
    frame=0;if(disposed)return;
    try{
      const prefs=viewportPreferences(PM),visible=available();
      const view=viewportState(PM);
      if(!visible){
        gizmo.sync({targets:[],mode:null,orientation:prefs.orientation,pivot:prefs.pivot,activeId:null,cursor:new THREE.Vector3(),snap:prefs.snap,camera:new THREE.PerspectiveCamera()});
        overlay.draw({camera:new THREE.PerspectiveCamera(),rect,runtime:null,userView:false,axis:null,distance:1,fov:50,overlays:false,selected:new Set(),active:new Set(),glyphs:[],guides:[],gizmo:null});
        chrome.update({visible:false,rotation:new THREE.Quaternion(),axis:null,camera:true,orthographic:false,label:'',detail:'',leftInset:0,rightInset:0,cursor:null,modal:null});
        return;
      }
      const runtime=compositionRuntime(PM),camera=viewCamera(),ids=selected3D(PM),active=activeObject(PM);
      const stageCamera=camera.clone() as typeof camera,width=Math.max(2,stage.clientWidth),height=Math.max(2,stage.clientHeight);
      stageCamera.setViewOffset(rect.width,rect.height,-rect.x,-rect.y,width,height);stageCamera.updateProjectionMatrix();stageCamera.updateMatrixWorld(true);
      const editing=editModeLayer(PM),edit=editing?editOverlay(PM):null,mode=modal||editing?null:TOOL_MODES[prefs.tool]??null;
      editHits=edit?edit.points.map(point=>{const s=toScreen(camera,rect,point);return {x:s.x,y:s.y};}):[];
      gizmo.sync({targets:mode&&!navigation.active()?targets():[],mode,orientation:prefs.orientation,pivot:prefs.pivot,activeId:active,cursor:cursorLocation(PM),snap:prefs.snap,camera:stageCamera});
      const selected=new Set(ids),pose=viewportPose(PM);
      overlay.draw({camera,rect,runtime,userView:view.mode==='editor',axis:view.axis,distance:pose.distance,fov:view.fov,overlays:prefs.overlays,
        edit,xray:prefs.xray,selected:editing?new Set():expand(ids),active:editing?new Set():expand(active?[active]:[]),glyphs:prefs.overlays?glyphs(camera,selected,active):[],guides:modal?.guides??[],gizmo:gizmo.attached()?gizmo.helper:null});
      const cursor=toScreen(camera,rect,cursorLocation(PM)),rotation=new THREE.Quaternion();camera.matrixWorld.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
      const fps=PM.proj?.fps||30,activeLayer=active?PM.L?.(active):null;
      chrome.update({visible:true,rotation,axis:view.axis,camera:view.mode==='camera',orthographic:view.projection==='orthographic',
        label:viewLabel(PM),detail:`(${Math.round((PM.time||0)*fps)})${activeLayer?` ${activeLayer.name}`:''}`,leftInset:prefs.toolbar?46:0,rightInset:prefs.sidebar?236:0,
        cursor:prefs.overlays&&cursor.z<1&&cursor.z>-1?{x:cursor.x,y:cursor.y}:null,box:border,
        modal:modal?{header:modal.header(),hints:modal.hints(),link:modal.link}:walk?{header:walk.header(),hints:walk.hints(),link:null}:zoomArmed?{header:'Zoom Border',hints:[['LMB drag','Zoom to region'],['Esc','Cancel']],link:null}:boxArmed?{header:'Box Select',hints:[['LMB drag','Select'],['⇧','Extend'],['Ctrl','Subtract'],['Esc','Cancel']],link:null}:null});
    }catch(error){console.error('[scene3d] viewport overlay failed',error);}
  }

  /* ── modal transforms ────────────────────────────────── */
  function endModal(){for(const off of modalOffs.splice(0))off();modal=null;schedule();PM.bus?.emit?.('scene3d:viewport');}
  function startModal(mode:ModalMode,start:Point,options:{tweak?:boolean;label?:string;prefix?:any[];constraint?:import('./modal').Constraint;targets?:ReturnType<typeof resolveTargets>}={}):boolean {
    if(modal||gizmo.active()||navigation.active())return false;
    const list=options.targets??targets();if(!list.length)return false;
    const prefs=viewportPreferences(PM);
    try{
      modal=ModalTransform.begin({PM,camera:viewCamera,rect:()=>rect,cursor:()=>cursorLocation(PM),orientation:prefs.orientation,pivot:prefs.pivot,snap:prefs.snap,activeId:activeObject(PM),
        changed:()=>{schedule();PM.bus?.emit?.('scene3d:viewport');},finished:()=>endModal()},mode,list,start,options);
    }catch(error){warn(error);return false;}
    if(!modal)return false;
    const current=modal;
    const capture=(type:string,handler:any)=>{window.addEventListener(type,handler,true);modalOffs.push(()=>window.removeEventListener(type,handler,true));};
    capture('pointermove',(e:PointerEvent)=>{const p=stagePoint(e.clientX,e.clientY);pointer=p;client={clientX:e.clientX,clientY:e.clientY};current.move(p,e);});
    capture('pointerdown',(e:PointerEvent)=>{
      e.preventDefault();e.stopImmediatePropagation();
      // The right click that cancels is followed by a contextmenu event; swallow only that one.
      if(e.button===0)current.confirm();else if(e.button===2){suppressContextUntil=performance.now()+600;current.cancel();}else if(e.button===1)current.autoConstrain();
    });
    if(options.tweak)capture('pointerup',(e:PointerEvent)=>{if(e.button!==0)return;e.preventDefault();e.stopImmediatePropagation();current.confirm();});
    // Like Blender, a running transform owns the keyboard until it is confirmed or cancelled.
    capture('keydown',(e:KeyboardEvent)=>{if(current.key(e)){e.preventDefault();e.stopImmediatePropagation();}});
    capture('keyup',(e:KeyboardEvent)=>{current.modifiers(e);if(['Shift','Control','Meta'].includes(e.key)){e.preventDefault();e.stopImmediatePropagation();}});
    capture('contextmenu',(e:MouseEvent)=>{e.preventDefault();e.stopImmediatePropagation();});
    capture('wheel',(e:WheelEvent)=>{e.preventDefault();e.stopImmediatePropagation();});
    capture('blur',()=>current.cancel());
    current.move(start,{shiftKey:false,ctrlKey:false});
    schedule();PM.bus?.emit?.('scene3d:viewport');
    return true;
  }
  /** Shift+B: drag a rectangle; the view zooms so it fills the composition frame. */
  function zoomBorder(event:PointerEvent){
    const start=stagePoint(event.clientX,event.clientY);
    const move=(e:PointerEvent)=>{e.preventDefault();e.stopImmediatePropagation();const p=stagePoint(e.clientX,e.clientY);
      border={x:Math.min(start.x,p.x),y:Math.min(start.y,p.y),width:Math.abs(p.x-start.x),height:Math.abs(p.y-start.y)};schedule();};
    const up=(e:PointerEvent)=>{
      e.preventDefault();e.stopImmediatePropagation();window.removeEventListener('pointermove',move,true);window.removeEventListener('pointerup',up,true);
      const box=border;zoomArmed?.();
      if(!box||box.width<4||box.height<4)return;
      const scale=Math.max(box.width/rect.width,box.height/rect.height),center={x:box.x+box.width/2,y:box.y+box.height/2};
      try{
        if(getViewportMode(PM)==='camera'&&!lockedCamera(PM)){
          const bounds=stage.getBoundingClientRect();host.zoomComposition(1/scale,bounds.left+center.x,bounds.top+center.y);return;
        }
        const camera=viewCamera(),pose=viewportPose(PM),forward=viewForward(camera);
        const hit=rayAt(camera,rect,center).intersectPlane(new THREE.Plane().setFromNormalAndCoplanarPoint(forward,pose.target),new THREE.Vector3())??pose.target;
        const next={target:hit,rotation:pose.rotation,distance:Math.max(.01,pose.distance*scale)};
        if(lockedCamera(PM))writeCameraPose(PM,next,'View camera');else applyViewportPose(PM,next,{axis:viewportState(PM).axis});
      }catch(error){warn(error);}
    };
    window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);
    event.preventDefault();event.stopImmediatePropagation();
  }
  /** Edit Mode presses: click selects a point (Shift toggles), Tweak drags grab, other drags box-select. */
  function editPress(event:PointerEvent){
    const p=stagePoint(event.clientX,event.clientY);
    const hit=editHits.map((h,i)=>({i,d:Math.hypot(h.x-p.x,h.y-p.y)})).filter(h=>h.d<=10).sort((a,b)=>a.d-b.d)[0]?.i;
    const tweak=viewportPreferences(PM).tool==='tweak'&&hit!==undefined;
    if(hit!==undefined&&!event.shiftKey&&tweak)selectPoints(PM,[hit],'set');
    press(event,(start,e)=>{
      if(tweak){startModal('translate',start,{tweak:true})&&modal?.move(stagePoint(e.clientX,e.clientY),e);return;}
      const move=(m:PointerEvent)=>{const q=stagePoint(m.clientX,m.clientY);border={x:Math.min(start.x,q.x),y:Math.min(start.y,q.y),width:Math.abs(q.x-start.x),height:Math.abs(q.y-start.y)};schedule();};
      const up=(u:PointerEvent)=>{
        window.removeEventListener('pointermove',move,true);window.removeEventListener('pointerup',up,true);
        const box=border;border=null;schedule();if(!box)return;
        const inside=editHits.map((h,i)=>({h,i})).filter(({h})=>h.x>=box.x&&h.x<=box.x+box.width&&h.y>=box.y&&h.y<=box.y+box.height).map(({i})=>i);
        selectPoints(PM,inside,u.ctrlKey||u.metaKey?'sub':u.shiftKey?'add':'set');
      };
      window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);move(e);
    },()=>{
      if(hit===undefined){if(!event.shiftKey)selectPoints(PM,[],'set');return;}
      selectPoints(PM,[hit],event.shiftKey?'toggle':'set');
    });
  }
  function transformBlocker(mode:ModalMode,editing:string|null,ids:string[]):string {
    if(editing)return 'Select points to transform';
    const layers=ids.map(id=>PM.L?.(id)).filter(Boolean);
    if(layers.some((l:any)=>l.lock||(PM.groupAncestors?.(l)||[]).some((g:any)=>g.lock)))return 'Unlock the selected layer first';
    if(ids.includes(excluded()??''))return 'This is the camera you are looking through. Press 0 for a user view, or turn on Lock Camera to View, to move it';
    if(mode==='translate'&&layers.some((l:any)=>l.d?.data?.light?.type==='sun'))return 'Sun lights only rotate; their position does not change the light';
    if(layers.some((l:any)=>!PM.active(l,PM.time)))return 'Move the playhead into the layer’s time range to transform it';
    return 'The selection cannot be transformed this way';
  }
  /** Arm a press: a drag past the threshold runs `drag`, a release without one runs `click`. */
  function press(event:PointerEvent,drag:(start:Point,e:PointerEvent)=>void,click:()=>void){
    const start=stagePoint(event.clientX,event.clientY);let done=false;
    const cleanup=()=>{done=true;window.removeEventListener('pointermove',move,true);window.removeEventListener('pointerup',up,true);window.removeEventListener('pointercancel',cleanup,true);};
    const move=(e:PointerEvent)=>{if(done)return;const p=stagePoint(e.clientX,e.clientY);if(Math.hypot(p.x-start.x,p.y-start.y)<DRAG_THRESHOLD)return;cleanup();drag(start,e);};
    const up=()=>{if(done)return;cleanup();click();};
    window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);window.addEventListener('pointercancel',cleanup,true);
    event.preventDefault();event.stopImmediatePropagation();
  }
  function cursorTool(event:PointerEvent){
    const place=(e:{clientX:number;clientY:number})=>{
      const camera=viewCamera(),p=stagePoint(e.clientX,e.clientY);
      try{placeCursor(PM,rayAt(camera,rect,p),viewForward(camera).negate());}catch(error){warn(error);}
    };
    place(event);
    const move=(e:PointerEvent)=>{e.preventDefault();place(e);};
    const up=()=>{window.removeEventListener('pointermove',move,true);window.removeEventListener('pointerup',up,true);};
    window.addEventListener('pointermove',move,true);window.addEventListener('pointerup',up,true);
    event.preventDefault();event.stopImmediatePropagation();
  }
  /** Blender's Tweak: press selects, dragging grabs; Select Box and the transform tools box-select on drag instead. */
  function selectPress(event:PointerEvent,id:string):boolean {
    const extend=event.shiftKey,wasSelected=(PM.sel?.layers||[]).includes(id);
    if(viewportPreferences(PM).tool!=='tweak')return false;
    if(!wasSelected||extend)clickSelect(PM,id,extend);
    press(event,(start,e)=>{if(!extend)startModal('translate',start,{tweak:true})&&modal?.move(stagePoint(e.clientX,e.clientY),e);},
      ()=>{if(wasSelected&&!extend)clickSelect(PM,id,false);});
    return true;
  }

  /* ── stage events ────────────────────────────────────── */
  listen(stage,'pointerenter',()=>{over=true;});
  listen(stage,'pointerleave',()=>{over=false;});
  listen(stage,'pointermove',(e:PointerEvent)=>{over=true;pointer=stagePoint(e.clientX,e.clientY);client={clientX:e.clientX,clientY:e.clientY};});
  const offViewport=onViewportChange(PM,()=>schedule()),offSettings=onViewportSettings(PM,()=>schedule()),offRender=onRenderState(PM,()=>schedule());
  offs.push(offViewport,offSettings,offRender);
  for(const event of ['selection','sel','time','project','layers','composition'])
    {const off=PM.bus?.on?.(event,()=>schedule());if(typeof off==='function')offs.push(off);}

  const controller:ViewportController&{over():boolean}={
    over:()=>over,
    update(next){rect={x:next.x,y:next.y,width:Math.max(1,next.width),height:Math.max(1,next.height)};schedule();},
    pointerDown(event){
      if(modal)return true;
      if(!available())return false;
      flush();
      if(zoomArmed&&event.button===0){zoomBorder(event);return true;}
      if(editModeLayer(PM)&&event.button===0&&!event.altKey){editPress(event);return true;}
      if(navigation.pointerDown(event))return true;
      if(event.button!==0||event.altKey||boxArmed)return false;
      if(gizmo.pointerDown(event))return true;
      const prefs=viewportPreferences(PM),p=stagePoint(event.clientX,event.clientY);
      if(prefs.tool==='cursor'){cursorTool(event);return true;}
      const glyph=prefs.overlays?glyphHits.filter(g=>Math.hypot(g.x-p.x,g.y-p.y)<=12).sort((a,b)=>Math.hypot(a.x-p.x,a.y-p.y)-Math.hypot(b.x-p.x,b.y-p.y))[0]:null;
      if(glyph){
        if(selectPress(event,glyph.id))return true;
        clickSelect(PM,glyph.id,event.shiftKey);event.preventDefault();event.stopImmediatePropagation();return true;
      }
      return false;
    },
    objectPress(event,id){return available()&&selectPress(event,id);},
    hover:(clientX,clientY)=>{flush();return gizmo.hover(clientX,clientY);},
    cursorAt(clientX,clientY){
      if(!available())return null;
      if(modal)return 'move';
      if(boxArmed||zoomArmed)return 'crosshair';
      flush();
      if(gizmo.hover(clientX,clientY))return 'pointer';
      return viewportPreferences(PM).tool==='cursor'?'crosshair':null;
    },
    wheel(event){if(modal){event.preventDefault();return true;}return navigation.wheel(event);},
    contextMenu(event){
      if(modal||performance.now()<suppressContextUntil){suppressContextUntil=0;event.preventDefault();return true;}
      if(!available()||!event.shiftKey)return false;
      // Shift+right click places the 3D cursor.
      event.preventDefault();
      const camera=viewCamera();
      try{placeCursor(PM,rayAt(camera,rect,stagePoint(event.clientX,event.clientY)),viewForward(camera).negate());}catch(error){warn(error);}
      return true;
    },
    startTransform(mode){
      // Keys only reach here over the viewer; before the first pointer move, start from the frame's centre.
      if(startModal(mode,pointer??{x:rect.x+rect.width/2,y:rect.y+rect.height/2}))return true;
      if(modal||gizmo.active()||navigation.active())return true;
      // With something selected, explain instead of letting G/R/S fall through to an unrelated 2D tool.
      const editing=editModeLayer(PM),ids=selected3D(PM);
      if(!editing&&!ids.length)return false;
      throw new Error(transformBlocker(mode,editing,ids));
    },
    armBoxSelect(){
      if(!available()||modal||boxArmed)return !!boxArmed;
      const key=(e:KeyboardEvent)=>{if(e.key==='Escape'){e.preventDefault();e.stopImmediatePropagation();disarm();}};
      const contextmenu=(e:MouseEvent)=>{e.preventDefault();e.stopImmediatePropagation();disarm();};
      const disarm=()=>{window.removeEventListener('keydown',key,true);window.removeEventListener('contextmenu',contextmenu,true);boxArmed=null;schedule();};
      window.addEventListener('keydown',key,true);window.addEventListener('contextmenu',contextmenu,true);
      boxArmed=disarm;schedule();return true;
    },
    extrude(){
      if(!pointer||!editModeLayer(PM))return false;
      const plan=planExtrude(PM);if(!plan)return false;
      if(!startModal('translate',pointer,{label:'Extrude',prefix:[plan.command],targets:[plan.target],constraint:{axis:'z',plane:false,orientation:'local'}}))return false;
      plan.adopt();return true;
    },
    startWalk(){
      if(!available()||modal||walk||navigation.active())return false;
      try{walk=startWalk(PM,stage,()=>schedule(),()=>{walk=null;schedule();PM.bus?.emit?.('scene3d:viewport');});}catch(error){warn(error);return true;}
      schedule();return !!walk;
    },
    armZoomBorder(){
      if(!available()||modal||zoomArmed)return !!zoomArmed;
      const key=(e:KeyboardEvent)=>{if(e.key==='Escape'){e.preventDefault();e.stopImmediatePropagation();disarm();}};
      const disarm=()=>{window.removeEventListener('keydown',key,true);zoomArmed=null;border=null;schedule();};
      window.addEventListener('keydown',key,true);zoomArmed=disarm;schedule();return true;
    },
    takeBoxSelect(){if(!boxArmed)return false;boxArmed();return true;},
    modalActive:()=>!!modal||!!walk,
    pointer:()=>over?client:null,
    covers:ids=>ids.length>0&&ids.every(id=>isViewportItem(PM,PM.L?.(id))),
    glyphsIn(box){
      const c=comp(PM),sx=(x:number)=>rect.x+x/Math.max(1,c.w)*rect.width,sy=(y:number)=>rect.y+y/Math.max(1,c.h)*rect.height;
      const x0=Math.min(sx(box.x0),sx(box.x1)),x1=Math.max(sx(box.x0),sx(box.x1)),y0=Math.min(sy(box.y0),sy(box.y1)),y1=Math.max(sy(box.y0),sy(box.y1));
      return viewportPreferences(PM).overlays?glyphHits.filter(g=>g.x>=x0&&g.x<=x1&&g.y>=y0&&g.y<=y1).map(g=>g.id):[];
    },
    active:()=>!!modal||gizmo.active()||navigation.active(),
    cancel(){walk?.cancel();modal?.cancel();gizmo.cancel();navigation.cancel();},
    dispose(){
      if(disposed)return;walk?.cancel();modal?.cancel();disposed=true;cancelAnimationFrame(frame);
      for(const off of offs.splice(0))off();
      gizmo.dispose();navigation.dispose();chrome.dispose();overlay.dispose();
      if(controllers.get(PM)===controller)controllers.delete(PM);
    }
  };
  controllers.set(PM,controller);
  schedule();
  return controller;
}
/** Exposed for agents and tests: set the 3D cursor without a pointer. */
export {setCursorLocation};
