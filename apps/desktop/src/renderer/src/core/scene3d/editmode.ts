import * as THREE from 'three';
import {compositionRuntime} from './service';
import {layer3DRole} from './layers';
import {activeObject,notifyViewport} from './editor-state';
import {editableLayer,transformModes,type TransformTarget} from './targets';
import {meshFromGeometry,weldMesh,topologyKey,transformPoints,deletePoints,mergePoints,extrudeRegion,meshEdges,pointPosition,type EditMesh,type Weld} from './editmesh';

/*
 * Blender's Edit Mode (Tab) for a model layer. Points are selected and
 * transformed in the viewport; the first edit turns a primitive or profile
 * into an indexed mesh source. Every edit is an ordinary undoable content
 * edit of the real layer, so Undo, saving and agents all see the same mesh.
 */

type EditState={id:string;selected:Set<number>;weld:Weld;mesh:EditMesh};
const states=new WeakMap<object,EditState>();

export const editModeLayer=(PM:any):string|null=>{const state=states.get(PM);return state&&PM.L?.(state.id)?state.id:null;};
function refuse(message:string):never {throw new Error(message);}
/** The mesh currently shown for a model: its own indexed mesh, or its generated geometry. */
export function currentMesh(PM:any,layer:any):EditMesh {
  const source=layer.d.data.object.source;
  if(source.mesh)return {positions:[...source.mesh.positions],indices:source.mesh.indices?[...source.mesh.indices]:Array.from({length:source.mesh.positions.length/3},(_,i)=>i),...(source.mesh.uvs?{uvs:[...source.mesh.uvs]}:{})};
  const object=compositionRuntime(PM).objects.get(layer.id) as THREE.Mesh|undefined;
  if(!object?.isMesh)refuse('This model has no editable mesh');
  return meshFromGeometry(object.geometry);
}
export function canEdit(PM:any,layer:any):string|null {
  if(layer3DRole(layer)!=='object')return 'Select a model to edit';
  if(!editableLayer(PM,layer))return 'Unlock the model first';
  const object=layer.d.data.object;
  if('assetId' in object.source||object.blender)return 'Imported models are edited in their source application';
  if(object.slots?.length)return 'Models with several material slots cannot be edited here yet';
  return null;
}
/** Tab: toggle Edit Mode for the active model. */
export function toggleEditMode(PM:any):boolean {
  if(editModeLayer(PM)){exitEditMode(PM);return true;}
  const id=activeObject(PM),layer=id?PM.L(id):null;
  if(!layer||layer3DRole(layer)!=='object')return false;
  const problem=canEdit(PM,layer);if(problem)refuse(problem);
  const mesh=currentMesh(PM,layer);
  states.set(PM,{id:layer.id,selected:new Set(),weld:weldMesh(mesh),mesh});
  PM.selectLayers?.([layer.id]);notifyViewport(PM);return true;
}
export function exitEditMode(PM:any):void {if(states.delete(PM))notifyViewport(PM);}
/** The live edit state; re-reads the mesh after Undo or another edit changed it. */
export function editState(PM:any):EditState|null {
  const state=states.get(PM);if(!state)return null;
  const layer=PM.L?.(state.id);
  if(!layer||!(PM.sel?.layers||[]).includes(state.id)||layer3DRole(layer)!=='object'){states.delete(PM);return null;}
  const mesh=currentMesh(PM,layer);
  if(topologyKey(mesh)!==state.weld.key){state.weld=weldMesh(mesh);state.selected.clear();}
  state.mesh=mesh;return state;
}
function meshCommand(PM:any,id:string,mesh:EditMesh) {
  const data=JSON.parse(JSON.stringify(PM.L(id).d.data));
  // An edited mesh is its own source: it no longer follows a recipe's regeneration.
  delete data.object.generation;delete data.object.blender;
  data.object.source={mesh:{positions:mesh.positions,indices:mesh.indices,...(mesh.uvs?{uvs:mesh.uvs}:{})}};
  return {type:'set_content',target:id,patch:{data}};
}
function commit(PM:any,state:EditState,mesh:EditMesh,label:string):void {
  const result=PM.Edit.apply(meshCommand(PM,state.id,mesh),{origin:'canvas',label});
  if(!result.ok)refuse(result.message);
  state.mesh=mesh;state.weld=weldMesh(mesh);notifyViewport(PM);
}

/* ── selection ───────────────────────────────────────── */
export function selectPoints(PM:any,points:Iterable<number>,mode:'set'|'add'|'sub'|'toggle'):void {
  const state=editState(PM);if(!state)return;
  const list=[...points];
  if(mode==='set')state.selected=new Set(list);
  else for(const point of list){
    if(mode==='add')state.selected.add(point);else if(mode==='sub')state.selected.delete(point);
    else if(state.selected.has(point))state.selected.delete(point);else state.selected.add(point);
  }
  notifyViewport(PM);
}
export function selectAllPoints(PM:any,action:'select'|'deselect'|'invert'|'toggle'):boolean {
  const state=editState(PM);if(!state)return false;
  const all=state.weld.points.map((_,i)=>i),resolved=action==='toggle'?(state.selected.size?'deselect':'select'):action;
  state.selected=resolved==='select'?new Set(all):resolved==='deselect'?new Set():new Set(all.filter(p=>!state.selected.has(p)));
  notifyViewport(PM);return true;
}

/* ── topology ────────────────────────────────────────── */
export function deleteSelectedPoints(PM:any):boolean {
  const state=editState(PM);if(!state||!state.selected.size)return false;
  const mesh=deletePoints(state.mesh,state.weld,state.selected);
  if(mesh.indices.length<3)refuse('A model needs at least one face; delete the layer instead');
  commit(PM,state,mesh,'Delete vertices');state.selected.clear();return true;
}
export function mergeSelectedPoints(PM:any):boolean {
  const state=editState(PM);if(!state||state.selected.size<2)return false;
  const center=new THREE.Vector3();for(const point of state.selected)center.add(pointPosition(state.mesh,state.weld,point));center.divideScalar(state.selected.size);
  const mesh=mergePoints(state.mesh,state.weld,state.selected);
  if(mesh.indices.length<3)refuse('Merging would remove every face');
  commit(PM,state,mesh,'Merge vertices');
  // Keep the merged point selected, like Blender.
  const merged=state.weld.points.findIndex((_,point)=>pointPosition(mesh,state.weld,point).distanceTo(center)<1e-5);
  state.selected=new Set(merged>=0?[merged]:[]);notifyViewport(PM);
  return true;
}
/**
 * E: plan an extrusion. The caller applies `command` inside the following
 * move's transaction, so Extrude and Move are one Undo and cancelling removes both.
 */
export function planExtrude(PM:any):{command:any;target:TransformTarget;adopt:()=>void}|null {
  const state=editState(PM);if(!state||!state.selected.size)return null;
  const result=extrudeRegion(state.mesh,state.weld,state.selected);if(!result)return null;
  const weld=result.weld,selected=new Set(result.vertices.map(v=>weld.vertexPoint[v]!));
  const object=compositionRuntime(PM).objects.get(state.id);object?.updateWorldMatrix(true,false);
  const normal=result.normal.clone().transformDirection(object?.matrixWorld??new THREE.Matrix4());
  const target=pointsTarget(PM,normal,{mesh:result.mesh,weld,selected});if(!target)return null;
  return {command:meshCommand(PM,state.id,result.mesh),target,adopt:()=>{state.mesh=result.mesh;state.weld=weld;state.selected=selected;notifyViewport(PM);}};
}

/* ── transforms ──────────────────────────────────────── */
/** The selected points as one transform target; its Local frame is the model's, or `normal` (extrude). */
export function pointsTarget(PM:any,normal?:THREE.Vector3,plan?:{mesh:EditMesh;weld:Weld;selected:Set<number>}):TransformTarget|null {
  const state=editState(PM);if(!state)return null;
  if(!(plan?.selected??state.selected).size)return null;
  const layer=PM.L(state.id),object=compositionRuntime(PM).objects.get(state.id);if(!object)return null;
  object.updateWorldMatrix(true,false);
  const world=object.matrixWorld.clone(),inverse=world.clone().invert(),base=plan?.mesh??state.mesh,weld=plan?.weld??state.weld,selected=new Set(plan?.selected??state.selected);
  const position=new THREE.Vector3();for(const point of selected)position.add(pointPosition(base,weld,point).applyMatrix4(world));position.divideScalar(selected.size);
  const rotation=new THREE.Quaternion();world.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
  if(normal)rotation.setFromUnitVectors(new THREE.Vector3(0,0,1),normal.clone().normalize());
  return {id:state.id,layer,role:'object',modes:transformModes('object'),object:null,world,parentInverse:new THREE.Matrix4(),position,rotation,target:null,orientation:new THREE.Vector3(),
    edit:(delta)=>[meshCommand(PM,state.id,transformPoints(base,weld,selected,inverse.clone().multiply(delta).multiply(world)))]};
}
/** World positions of the points, their selection and edges, for the overlay. */
export function editOverlay(PM:any):{points:THREE.Vector3[];selected:boolean[];edges:[number,number][]}|null {
  const state=editState(PM);if(!state)return null;
  const object=compositionRuntime(PM).objects.get(state.id);if(!object)return null;
  object.updateWorldMatrix(true,false);
  const points=state.weld.points.map((_,p)=>pointPosition(state.mesh,state.weld,p).applyMatrix4(object.matrixWorld));
  return {points,selected:points.map((_,p)=>state.selected.has(p)),edges:meshEdges(state.mesh,state.weld)};
}
export function editSummary(PM:any):{layer:string;selected:number;total:number}|null {
  const state=editState(PM);return state?{layer:state.id,selected:state.selected.size,total:state.weld.points.length}:null;
}
