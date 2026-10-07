import * as THREE from 'three';
import {compositionRuntime} from './service';
import {layer3DRole} from './layers';
import {editScene} from './operations';
import {activeObject,setActiveObject,selected3D,isViewportItem,cursorLocation,setCursorLocation} from './editor-state';
import {resolveTargets,deltaCommands,clearCommands,editableLayer,propertyCommand,type TransformMode} from './targets';
import {getViewportMode,activeCameraLayer,localViewIds} from './viewport';

/*
 * Blender object-mode operators over ordinary timeline layers. Each returns
 * false when it has nothing to act on, so the key that invoked it can fall
 * through to the editor's own shortcut.
 */

const comp=(PM:any)=>PM.curComp?.()||PM.proj;
/** Selectable items: active 3D layers, limited to the models of Local View when it is on. */
const items=(PM:any)=>{const local=localViewIds(PM);return comp(PM).layers.filter((l:any)=>isViewportItem(PM,l)&&(!PM.active||PM.active(l,PM.time))&&(!local||layer3DRole(l)!=='object'||local.has(l.id)));};
const fail=(message:string)=>{throw new Error(message);};
const select=(PM:any,ids:string[],active?:string|null)=>{
  PM.selectLayers?.(ids);setActiveObject(PM,active===undefined?ids.at(-1)??null:active);PM.bus?.emit?.('scene3d:selection');PM.invalidate?.();
};
const transformTargets=(PM:any,ids=selected3D(PM))=>resolveTargets(PM,ids,compositionRuntime(PM),{excludeCamera:getViewportMode(PM)==='camera'?activeCameraLayer(PM)?.id:null});

/* ── selection ───────────────────────────────────────── */
export type SelectAction='select'|'deselect'|'invert'|'toggle';
export function selectAll(PM:any,action:SelectAction='select'):boolean {
  const all=items(PM).filter((l:any)=>layer3DRole(l)).map((l:any)=>l.id),current=new Set(selected3D(PM));
  if(!all.length&&action!=='deselect')return false;
  const resolved=action==='toggle'?(current.size?'deselect':'select'):action;
  const keep=(PM.sel?.layers||[]).filter((id:string)=>!isViewportItem(PM,PM.L?.(id)));
  if(resolved==='deselect'){if(!current.size)return false;select(PM,keep,null);return true;}
  if(resolved==='invert'){select(PM,[...keep,...all.filter((id:string)=>!current.has(id))]);return true;}
  select(PM,[...keep,...all],activeObject(PM)??all.at(-1));return true;
}
export function selectByType(PM:any,role:'object'|'light'|'camera',extend=true):boolean {
  const ids=items(PM).filter((l:any)=>layer3DRole(l)===role).map((l:any)=>l.id);if(!ids.length)return false;
  select(PM,extend?[...new Set([...(PM.sel?.layers||[]),...ids])]:ids);return true;
}
/**
 * Blender click selection. Plain click selects only the item. Shift adds an
 * unselected item and makes it active, makes a selected item active, and
 * deselects the item that is already active.
 */
export function clickSelect(PM:any,id:string|null,extend=false):void {
  const current:string[]=[...(PM.sel?.layers||[])];
  if(!id){if(!extend)select(PM,[],null);return;}
  if(!extend){select(PM,[id],id);return;}
  if(!current.includes(id)){select(PM,[...current,id],id);return;}
  if(activeObject(PM)!==id){setActiveObject(PM,id);PM.invalidate?.();return;}
  const next=current.filter(x=>x!==id);select(PM,next,next.filter(x=>isViewportItem(PM,PM.L?.(x))).at(-1)??null);
}
/** Box select: set, extend (Shift) or subtract (Ctrl). */
export function boxSelect(PM:any,ids:string[],mode:'set'|'add'|'sub'):void {
  const local=localViewIds(PM);if(local)ids=ids.filter(id=>layer3DRole(PM.L?.(id))!=='object'||local.has(id));
  const current:string[]=[...(PM.sel?.layers||[])];
  if(mode==='set')select(PM,ids,activeObject(PM)&&ids.includes(activeObject(PM)!)?activeObject(PM):ids.at(-1)??null);
  else if(mode==='add')select(PM,[...new Set([...current,...ids])],activeObject(PM)??ids.at(-1)??null);
  else {const next=current.filter(id=>!ids.includes(id));select(PM,next,next.includes(activeObject(PM)!)?activeObject(PM):null);}
}

/* ── object ──────────────────────────────────────────── */
function transaction<T>(PM:any,label:string,run:()=>T):T {
  PM.Edit.begin(label,{origin:'command'});
  try{const result=run();const committed=PM.Edit.commit(label);if(!committed.ok)fail(committed.message);return result;}
  catch(error){PM.Edit.cancel();throw error;}
}
export function deleteSelected(PM:any):boolean {
  const ids=selected3D(PM).filter(id=>editableLayer(PM,PM.L(id)));if(!ids.length)return false;
  transaction(PM,ids.length===1?'Delete 3D layer':'Delete 3D layers',()=>{
    for(const id of ids)if(PM.L(id)){const result=editScene(PM,{operation:'remove',target:id},{origin:'command'});if(!result.ok)fail(result.message);}
  });
  select(PM,[],null);return true;
}
/** Shift+D: duplicates are selected so the following grab moves them. */
export function duplicateSelected(PM:any):string[] {
  const ids=selected3D(PM).filter(id=>layer3DRole(PM.L(id)));if(!ids.length)return [];
  const copies=transaction(PM,ids.length===1?'Duplicate 3D layer':'Duplicate 3D layers',()=>ids.map(id=>{
    const result=editScene(PM,{operation:'duplicate',target:id},{origin:'command'});if(!result.ok)fail(result.message);
    return (result as any).data?.result?.id as string;
  }).filter(Boolean));
  select(PM,copies);return copies;
}
/** H / Shift+H / Alt+H use the layer's visibility, which the timeline shows. */
export function hideSelected(PM:any,unselected=false):boolean {
  const chosen=new Set(selected3D(PM));
  const targets=items(PM).filter((l:any)=>layer3DRole(l)&&l.on!==false&&(unselected?!chosen.has(l.id):chosen.has(l.id)));
  if(!targets.length)return false;
  const result=PM.Edit.apply(targets.map((l:any)=>({type:'set_layer',target:l.id,patch:{visible:false}})),{origin:'command',label:unselected?'Hide unselected':'Hide selected'});
  if(!result.ok)fail(result.message);
  if(!unselected)select(PM,(PM.sel?.layers||[]).filter((id:string)=>!chosen.has(id)),null);
  return true;
}
export function revealHidden(PM:any):boolean {
  const hidden=comp(PM).layers.filter((l:any)=>layer3DRole(l)&&l.on===false);if(!hidden.length)return false;
  const result=PM.Edit.apply(hidden.map((l:any)=>({type:'set_layer',target:l.id,patch:{visible:true}})),{origin:'command',label:'Reveal hidden'});
  if(!result.ok)fail(result.message);
  select(PM,hidden.map((l:any)=>l.id));return true;
}
/** Alt+G / Alt+R / Alt+S. */
export function clearTransform(PM:any,mode:TransformMode):boolean {
  const targets=transformTargets(PM);if(!targets.length)return false;
  const commands=targets.flatMap(t=>clearCommands(PM,t,mode));if(!commands.length)return false;
  const result=PM.Edit.apply(commands,{origin:'command',label:mode==='translate'?'Clear location':mode==='rotate'?'Clear rotation':'Clear scale'});
  if(!result.ok)fail(result.message);return true;
}
const KEY_CHANNELS:Record<string,string[]>={
  object:['position.x','position.y','position.z','rotation.x','rotation.y','rotation','scale.x','scale.y','scale.z'],
  group:['position.x','position.y','position.z','rotation.x','rotation.y','rotation','scale.x','scale.y','scale.z'],
  light:['position.x','position.y','position.z','light.targetX','light.targetY','light.targetZ'],
  camera:['position.x','position.y','position.z','camera.targetX','camera.targetY','camera.targetZ']
};
const channelProp=(layer:any,path:string)=>{
  const [head,key]=path.split('.') as [string,string|undefined];
  if((head==='light'||head==='camera')&&key)return layer.d?.data?.[head]?.p?.[key];
  return layer.p?.[path];
};
/** I: key Location, Rotation and Scale (lights and cameras key position and aim). */
export function insertKeyframes(PM:any):boolean {
  const layers=selected3D(PM).map(id=>PM.L(id)).filter((l:any)=>editableLayer(PM,l));if(!layers.length)return false;
  const commands=layers.flatMap((l:any)=>{
    const role=layer3DRole(l)||'group';
    return KEY_CHANNELS[role]!.filter(path=>channelProp(l,path)).map(path=>({...propertyCommand(PM,l.id,path,Number(PM.evP(l,channelProp(l,path),PM.time,path))),mode:'keyframe'}));
  });
  const result=PM.Edit.apply(commands,{origin:'command',label:'Insert keyframes'});if(!result.ok)fail(result.message);
  return true;
}
/** Alt+I: remove the transform keys at the playhead. */
export function deleteKeyframes(PM:any):boolean {
  const layers=selected3D(PM).map(id=>PM.L(id)).filter((l:any)=>editableLayer(PM,l));if(!layers.length)return false;
  const fps=PM.proj.fps||30;let removed=0;
  const result=PM.Edit.mutate('Delete keyframes',()=>{
    for(const l of layers)for(const path of KEY_CHANNELS[layer3DRole(l)||'group']!){
      const prop=channelProp(l,path),local=PM.time-l.from;
      const key=prop?.kf?.find((k:any)=>Math.abs(k.t-local)<.5/fps);if(key){PM.removeKey(prop,key);removed++;}
    }
  },{origin:'command'});
  if(!result.ok)fail(result.message);
  return removed>0;
}
/** Ctrl+P: parent the other selected items to the active one, keeping their world pose. */
export function parentToActive(PM:any):boolean {
  const active=activeObject(PM),children=selected3D(PM).filter(id=>id!==active);
  if(!active||!children.length)return false;
  const result=PM.Edit.apply(children.map(id=>({type:'set_layer',target:id,patch:{parent:active}})),{origin:'command',label:'Parent to active'});
  if(!result.ok)fail(result.message);return true;
}
/** Alt+P: clear parent, keeping the world pose. */
export function clearParent(PM:any):boolean {
  const ids=selected3D(PM).filter(id=>PM.L(id)?.parent);if(!ids.length)return false;
  const result=PM.Edit.apply(ids.map(id=>({type:'set_layer',target:id,patch:{parent:null}})),{origin:'command',label:'Clear parent'});
  if(!result.ok)fail(result.message);return true;
}

/* ── snapping and the 3D cursor (Shift+S) ─────────────── */
export type SnapOperation='cursor-to-selected'|'cursor-to-active'|'cursor-to-origin'|'cursor-to-grid'|'selection-to-cursor'|'selection-to-cursor-offset'|'selection-to-active'|'selection-to-grid';
const median=(points:THREE.Vector3[])=>points.reduce((sum,p)=>sum.add(p),new THREE.Vector3()).divideScalar(Math.max(1,points.length));
const grid=(p:THREE.Vector3)=>new THREE.Vector3(Math.round(p.x),Math.round(p.y),Math.round(p.z));
function moveTargets(PM:any,label:string,destination:(t:{position:THREE.Vector3})=>THREE.Vector3,ids?:string[]):boolean {
  const targets=transformTargets(PM,ids).filter(t=>t.modes.includes('translate'));if(!targets.length)return false;
  const commands=targets.flatMap(t=>deltaCommands(PM,t,new THREE.Matrix4().makeTranslation(destination(t).sub(t.position)),'translate'));
  const result=PM.Edit.apply(commands,{origin:'command',label});if(!result.ok)fail(result.message);return true;
}
export function snap(PM:any,operation:SnapOperation):boolean {
  const targets=transformTargets(PM),active=targets.find(t=>t.id===activeObject(PM))??targets.at(-1);
  switch(operation){
    case 'cursor-to-origin':setCursorLocation(PM,new THREE.Vector3());return true;
    case 'cursor-to-grid':setCursorLocation(PM,grid(cursorLocation(PM)));return true;
    case 'cursor-to-selected':if(!targets.length)return false;setCursorLocation(PM,median(targets.map(t=>t.position)));return true;
    case 'cursor-to-active':if(!active)return false;setCursorLocation(PM,active.position);return true;
    case 'selection-to-cursor':return moveTargets(PM,'Selection to cursor',()=>cursorLocation(PM));
    case 'selection-to-cursor-offset':{
      if(!targets.length)return false;const offset=cursorLocation(PM).sub(median(targets.map(t=>t.position)));
      return moveTargets(PM,'Selection to cursor',t=>t.position.clone().add(offset));
    }
    case 'selection-to-active':{
      if(!active)return false;const destination=active.position.clone();
      return moveTargets(PM,'Selection to active',()=>destination.clone(),targets.filter(t=>t!==active).map(t=>t.id));
    }
    case 'selection-to-grid':return moveTargets(PM,'Selection to grid',t=>grid(t.position));
  }
  return false;
}
/** Shift+right click: place the cursor on the surface under the pointer, else on the view plane through it. */
export function placeCursor(PM:any,ray:THREE.Ray,viewNormal:THREE.Vector3):void {
  const world=compositionRuntime(PM),caster=new THREE.Raycaster(ray.origin,ray.direction);
  const hit=caster.intersectObjects([...world.objects.values()].filter(o=>!(o as any).isLight),true).find(h=>h.object.visible&&(h.object as any).isMesh);
  const point=hit?.point??ray.intersectPlane(new THREE.Plane().setFromNormalAndCoplanarPoint(viewNormal,cursorLocation(PM)),new THREE.Vector3());
  if(point)setCursorLocation(PM,point);
}
