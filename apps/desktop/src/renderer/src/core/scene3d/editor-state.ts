import * as THREE from 'three';
import {layer3DRole,isModelGroup} from './layers';
import {touchViewport} from './viewport';

/*
 * Blender viewport settings that belong to the editor, not the project:
 * active tool, transform orientation, pivot point, snapping, overlays, panel
 * visibility, the 3D cursor and the active object. Preferences persist
 * locally; the cursor and active object are per composition and transient.
 */

export type ViewportTool='tweak'|'box'|'cursor'|'move'|'rotate'|'scale';
export type ViewportOrientation='global'|'local'|'view';
export type ViewportPivot='median'|'individual'|'cursor'|'active'|'bounds';
export type ViewportPreferences={tool:ViewportTool;orientation:ViewportOrientation;pivot:ViewportPivot;snap:boolean;overlays:boolean;toolbar:boolean;sidebar:boolean;lockCamera:boolean;xray:boolean};
export const VIEWPORT_TOOLS:ViewportTool[]=['tweak','box','cursor','move','rotate','scale'];
export const VIEWPORT_ORIENTATIONS:ViewportOrientation[]=['global','local','view'];
export const VIEWPORT_PIVOTS:ViewportPivot[]=['median','individual','cursor','active','bounds'];
const DEFAULTS:ViewportPreferences={tool:'box',orientation:'global',pivot:'median',snap:false,overlays:true,toolbar:true,sidebar:false,lockCamera:false,xray:false};
const STORAGE_KEY='powermove.viewport3d';

type EditorState={prefs:ViewportPreferences;cursors:Map<string,THREE.Vector3>;active:Map<string,string>;listeners:Set<()=>void>};
const states=new WeakMap<object,EditorState>();
function load():ViewportPreferences {
  try{
    const raw=JSON.parse(globalThis.localStorage?.getItem(STORAGE_KEY)||'{}');
    const prefs={...DEFAULTS};
    if(VIEWPORT_TOOLS.includes(raw.tool))prefs.tool=raw.tool;
    if(VIEWPORT_ORIENTATIONS.includes(raw.orientation))prefs.orientation=raw.orientation;
    if(VIEWPORT_PIVOTS.includes(raw.pivot))prefs.pivot=raw.pivot;
    for(const key of ['snap','overlays','toolbar','sidebar','lockCamera','xray'] as const)if(typeof raw[key]==='boolean')prefs[key]=raw[key];
    return prefs;
  }catch{return {...DEFAULTS};}
}
function editor(PM:any):EditorState {
  let entry=states.get(PM);
  if(!entry){entry={prefs:load(),cursors:new Map(),active:new Map(),listeners:new Set()};states.set(PM,entry);}
  return entry;
}
const compKey=(PM:any)=>{const comp=PM.curComp?.()||PM.proj;return `${PM.proj?.id}:${comp?.compId||comp?.id}`;};
export function notifyViewport(PM:any):void {for(const listener of [...editor(PM).listeners])listener();PM.invalidate?.('overlay');}
export function onViewportSettings(PM:any,listener:()=>void):()=>void {const set=editor(PM).listeners;set.add(listener);return()=>set.delete(listener);}
export const viewportPreferences=(PM:any):ViewportPreferences=>({...editor(PM).prefs});
export function setViewportPreferences(PM:any,patch:Partial<ViewportPreferences>):void {
  const entry=editor(PM),next={...entry.prefs};
  if(patch.tool!==undefined){if(!VIEWPORT_TOOLS.includes(patch.tool))throw new Error(`Unknown 3D tool "${String(patch.tool)}"`);next.tool=patch.tool;}
  if(patch.orientation!==undefined){if(!VIEWPORT_ORIENTATIONS.includes(patch.orientation))throw new Error(`Unknown transform orientation "${String(patch.orientation)}"`);next.orientation=patch.orientation;}
  if(patch.pivot!==undefined){if(!VIEWPORT_PIVOTS.includes(patch.pivot))throw new Error(`Unknown pivot point "${String(patch.pivot)}"`);next.pivot=patch.pivot;}
  for(const key of ['snap','overlays','toolbar','sidebar','lockCamera','xray'] as const)if(patch[key]!==undefined)next[key]=!!patch[key];
  const redraw=next.xray!==entry.prefs.xray;
  entry.prefs=next;
  try{globalThis.localStorage?.setItem(STORAGE_KEY,JSON.stringify(next));}catch{/* storage is optional */}
  notifyViewport(PM);
  // X-ray changes the composition preview itself, not just the overlays.
  if(redraw)touchViewport(PM);
}

/* ── 3D cursor ────────────────────────────────────────── */
export const cursorLocation=(PM:any):THREE.Vector3=>(editor(PM).cursors.get(compKey(PM))||new THREE.Vector3()).clone();
export function setCursorLocation(PM:any,point:THREE.Vector3|[number,number,number]):void {
  const value=Array.isArray(point)?new THREE.Vector3(...point):point.clone();
  if(![value.x,value.y,value.z].every(Number.isFinite))throw new Error('Enter a finite 3D cursor location');
  editor(PM).cursors.set(compKey(PM),value);notifyViewport(PM);
}

/* ── selection: Blender keeps an active object alongside the selection ── */
export const isViewportItem=(PM:any,layer:any)=>!!layer3DRole(layer)||isModelGroup(PM,layer);
export function selected3D(PM:any):string[] {
  return (PM.sel?.layers||[]).filter((id:string)=>isViewportItem(PM,PM.L?.(id)));
}
/** The active object: the last clicked item while it stays selected, otherwise the most recently selected. */
export function activeObject(PM:any):string|null {
  const ids=selected3D(PM),stored=editor(PM).active.get(compKey(PM));
  return stored&&ids.includes(stored)?stored:ids.at(-1)??null;
}
export function setActiveObject(PM:any,id:string|null):void {
  const entry=editor(PM),key=compKey(PM);
  if(id)entry.active.set(key,id);else entry.active.delete(key);notifyViewport(PM);
}
