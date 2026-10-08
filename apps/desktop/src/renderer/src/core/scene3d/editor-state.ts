import {roleOf} from './targets';
import {touchViewport} from './viewport';

/*
 * Editor-only 3D settings, never part of the project: whether the editor
 * draws its guides (floor grid, light and camera wires) and whether navigating
 * in camera view moves the camera itself. Both persist locally.
 */

export type ViewportPreferences={overlays:boolean;lockCamera:boolean};
const DEFAULTS:ViewportPreferences={overlays:true,lockCamera:false};
const STORAGE_KEY='powermove.viewport3d';

type EditorState={prefs:ViewportPreferences;listeners:Set<()=>void>};
const states=new WeakMap<object,EditorState>();
function load():ViewportPreferences {
  try{
    const raw=JSON.parse(globalThis.localStorage?.getItem(STORAGE_KEY)||'{}'),prefs={...DEFAULTS};
    for(const key of ['overlays','lockCamera'] as const)if(typeof raw[key]==='boolean')prefs[key]=raw[key];
    return prefs;
  }catch{return {...DEFAULTS};}
}
function editor(PM:any):EditorState {
  let entry=states.get(PM);
  if(!entry){entry={prefs:load(),listeners:new Set()};states.set(PM,entry);}
  return entry;
}
export function notifyViewport(PM:any):void {for(const listener of [...editor(PM).listeners])listener();PM.invalidate?.('overlay');}
export function onViewportSettings(PM:any,listener:()=>void):()=>void {const set=editor(PM).listeners;set.add(listener);return()=>set.delete(listener);}
export const viewportPreferences=(PM:any):ViewportPreferences=>({...editor(PM).prefs});
export function setViewportPreferences(PM:any,patch:Partial<ViewportPreferences>):void {
  const entry=editor(PM),next={...entry.prefs};
  for(const key of ['overlays','lockCamera'] as const)if(patch[key]!==undefined)next[key]=!!patch[key];
  const redraw=next.overlays!==entry.prefs.overlays;
  entry.prefs=next;
  try{globalThis.localStorage?.setItem(STORAGE_KEY,JSON.stringify(next));}catch{/* storage is optional */}
  notifyViewport(PM);
  if(redraw)touchViewport(PM);
}

/** Selected layers that live in 3D: models, lights, cameras, model groups and 3D-enabled 2D layers. */
export const isViewportItem=(PM:any,layer:any)=>!!roleOf(PM,layer);
export function selected3D(PM:any):string[] {
  return (PM.sel?.layers||[]).filter((id:string)=>isViewportItem(PM,PM.L?.(id)));
}
