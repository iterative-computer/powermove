import * as THREE from 'three';
import {layer3DRole,isModelGroup,type Layer3DRole} from './layers';
import {parent3D,world3D} from '../../legacy/core/space-3d';
import type {SceneRuntime} from './runtime';

/*
 * What a 3D transform acts on, and how a world-space delta becomes edits to
 * each real layer's own channels. Shared by the transform gizmo and Blender's
 * modal G/R/S so both write identical, undoable keyframe-aware edits.
 */

export type TransformMode='translate'|'rotate'|'scale';
export type TransformTarget={
  id:string;layer:any;role:Layer3DRole|'group';modes:TransformMode[];
  /** Objects: the runtime node. Lights and cameras are placed from their layer values. */
  object:THREE.Object3D|null;
  world:THREE.Matrix4;parentInverse:THREE.Matrix4;
  /** World origin (anchor) used as the individual pivot. */
  position:THREE.Vector3;
  /** World rotation used for Local orientation. */
  rotation:THREE.Quaternion;
  /** World aim point for lights and cameras. */
  target:THREE.Vector3|null;
  orientation:THREE.Vector3;
  /** Edit Mode points supply their own edits for a world delta. */
  edit?:(delta:THREE.Matrix4,mode:TransformMode)=>any[];
};

const vec=(values:number[])=>new THREE.Vector3(values[0],values[1],values[2]);
const tidy=(value:number)=>Math.round(value*1e9)/1e9;
const comp=(PM:any)=>PM.curComp?.()||PM.proj;
export const roleOf=(PM:any,layer:any):Layer3DRole|'group'|null=>layer3DRole(layer)||(isModelGroup(PM,layer)?'group':null);
export const editableLayer=(PM:any,layer:any)=>!!layer&&!layer.lock&&!(PM.groupAncestors?.(layer)||[]).some((g:any)=>g.lock);
const ev=(PM:any,l:any,prop:any,path:string,fallback=0)=>prop?Number(PM.evP(l,prop,PM.time,path)):fallback;
export const channel=(PM:any,l:any,path:string,fallback=0)=>ev(PM,l,l.p?.[path],path,fallback);

/** Which transforms apply: a sun only aims, a point light only moves, a camera never scales. */
export function transformModes(role:Layer3DRole|'group',lightType?:string):TransformMode[] {
  if(role==='object'||role==='group')return ['translate','rotate','scale'];
  if(role==='camera')return ['translate','rotate'];
  return lightType==='sun'?['rotate']:lightType==='point'?['translate']:['translate','rotate'];
}

export function aimPoint(PM:any,layer:any):THREE.Vector3|null {
  const role=layer3DRole(layer),content=role==='light'?layer.d?.data?.light?.p:role==='camera'?layer.d?.data?.camera?.p:null;
  if(!content)return null;
  return vec(['X','Y','Z'].map(axis=>ev(PM,layer,content[`target${axis}`],`${role}.target${axis}`))).applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,layer,PM.time)));
}

export function resolveTargets(PM:any,ids:readonly string[],runtime:SceneRuntime|null,options:{excludeCamera?:string|null}={}):TransformTarget[] {
  const out:TransformTarget[]=[];
  for(const id of ids){
    const l=PM.L?.(id),role=roleOf(PM,l);
    if(!role||!editableLayer(PM,l)||(role==='camera'&&id===options.excludeCamera))continue;
    if(PM.active&&!PM.active(l,PM.time))continue;
    const lightType=role==='light'?l.d?.data?.light?.type:undefined;
    const object=role==='object'?runtime?.objects.get(id)??null:null;
    if(role==='object'&&!object)continue;
    const position=role==='object'?object!.getWorldPosition(new THREE.Vector3())
      :new THREE.Vector3().setFromMatrixPosition(new THREE.Matrix4().fromArray(world3D(PM,l,PM.time)));
    const target=role==='light'||role==='camera'?aimPoint(PM,l):null;
    let world=new THREE.Matrix4();
    if(object){object.updateWorldMatrix(true,false);world=object.matrixWorld.clone();}
    else if(role==='group')world.fromArray(world3D(PM,l,PM.time));
    else {
      // A light or camera's frame looks from its position toward its target.
      const look=new THREE.Matrix4().lookAt(position,target&&target.distanceToSquared(position)>1e-12?target:position.clone().add(new THREE.Vector3(0,0,-1)),new THREE.Vector3(0,1,0));
      world.copy(look).setPosition(position);
    }
    if(role==='object'||role==='group')position.copy(vec(['x','y','z'].map(axis=>channel(PM,l,`anchor.${axis}`))).applyMatrix4(world));
    if(role==='group'&&!l.threeD){
      const box=new THREE.Box3();
      for(const child of comp(PM).layers)if((PM.groupAncestors?.(child)||[]).some((g:any)=>g.id===id)){
        const node=runtime?.objects.get(child.id);if(node)box.expandByObject(node);
      }
      if(!box.isEmpty())box.getCenter(position);
    }
    const rotation=new THREE.Quaternion();world.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
    out.push({id,layer:l,role,modes:transformModes(role,lightType),object,world,
      parentInverse:new THREE.Matrix4().fromArray(parent3D(PM,l,PM.time)).invert(),position,rotation,target,
      orientation:vec(['x','y','z'].map(axis=>channel(PM,l,`orientation.${axis}`)))});
  }
  // Selecting a parent and its child transforms the child once, through its parent.
  const roots=new Set((PM.transformRoots?.(out.map(t=>t.id))||out.map(t=>t.layer)).map((l:any)=>l.id));
  return out.filter(t=>roots.has(t.id));
}
export const allowedModes=(list:readonly TransformTarget[]):TransformMode[]=>list.length?(['translate','rotate','scale'] as TransformMode[]).filter(mode=>list.every(t=>t.modes.includes(mode))):[];

const setCommand=(PM:any,id:string,path:string,value:number)=>({type:'set_property',target:id,path,value:tidy(value),
  time:PM.time,mode:PM.autokey?'keyframe':'auto',preserveHandEdits:false,markIntent:'human'});

/**
 * Plain artwork groups use a 2D anchor. Before a 3D gesture, upgrade them to
 * 3D groups pivoting on their geometry, keeping every member where it is.
 */
export function upgradeGroupCommands(PM:any,targets:readonly TransformTarget[]):any[] {
  const commands:any[]=[];
  for(const t of targets.filter(t=>t.role==='group'&&!t.layer.threeD)){
    const anchor=t.position.clone().applyMatrix4(t.world.clone().invert());
    const old=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`anchor.${axis}`)));
    const local=t.parentInverse.clone().multiply(t.world),motion=anchor.clone().sub(old).applyMatrix3(new THREE.Matrix3().setFromMatrix4(local));
    const position=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`position.${axis}`))).add(motion);
    commands.push({type:'set_layer',target:t.id,patch:{threeD:true}});
    for(const [axis,value] of [['x',anchor.x],['y',anchor.y],['z',anchor.z]] as const)commands.push(setCommand(PM,t.id,`anchor.${axis}`,value));
    for(const [axis,value] of [['x',position.x],['y',position.y],['z',position.z]] as const)commands.push(setCommand(PM,t.id,`position.${axis}`,value));
  }
  return commands;
}

/**
 * Edits that place `t` at `delta × start world`. Channels are written as
 * absolute values from the gesture's start, so re-applying is idempotent.
 * `moveAim` carries a light or camera's aim point along with a move (Alt).
 */
export function deltaCommands(PM:any,t:TransformTarget,delta:THREE.Matrix4,mode:TransformMode,options:{moveAim?:boolean}={}):any[] {
  if(t.edit)return t.edit(delta,mode);
  const commands:any[]=[];
  if(t.role==='object'||t.role==='group'){
    const local=t.parentInverse.clone().multiply(delta.clone().multiply(t.world));
    const position=new THREE.Vector3(),quaternion=new THREE.Quaternion(),scale=new THREE.Vector3();local.decompose(position,quaternion,scale);
    const anchor=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`anchor.${axis}`)));
    position.add(anchor.applyMatrix3(new THREE.Matrix3().setFromMatrix4(local)));
    const orientation=new THREE.Quaternion().setFromEuler(new THREE.Euler(...t.orientation.toArray().map(THREE.MathUtils.degToRad) as [number,number,number],'ZYX'));
    quaternion.multiply(orientation.invert());
    const euler=new THREE.Euler().setFromQuaternion(quaternion,'ZYX'),deg=THREE.MathUtils.radToDeg;
    // The runtime adds native orientation to rotation; write back rotation alone.
    const values:[string,number][]=[['position.x',position.x],['position.y',position.y],['position.z',position.z],
      ['rotation.x',deg(euler.x)],['rotation.y',deg(euler.y)],['rotation',deg(euler.z)],
      ['scale.x',scale.x*100],['scale.y',scale.y*100],['scale.z',scale.z*100]];
    for(const [path,value] of values)
      if(path.startsWith('position.')||(mode==='rotate'?path.startsWith('rotation'):mode==='scale'?path.startsWith('scale.'):false))commands.push(setCommand(PM,t.id,path,value));
    return commands;
  }
  const prefix=t.role==='light'?'light':'camera',position=t.position.clone().applyMatrix4(delta);
  if(t.modes.includes('translate')&&position.distanceToSquared(t.position)>1e-18){
    const motion=position.clone().sub(t.position).applyMatrix3(new THREE.Matrix3().setFromMatrix4(t.parentInverse));
    const localPosition=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`position.${axis}`))).add(motion);
    for(const [axis,value] of [['x',localPosition.x],['y',localPosition.y],['z',localPosition.z]] as const)commands.push(setCommand(PM,t.id,`position.${axis}`,value));
  }
  if(t.target){
    // Moving keeps the aim point (Alt moves it along); rotating swings it around the position.
    const rotation=new THREE.Quaternion();delta.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
    const target=mode==='rotate'?position.clone().add(t.target.clone().sub(t.position).applyQuaternion(rotation))
      :options.moveAim?t.target.clone().add(position.clone().sub(t.position)):null;
    if(target){target.applyMatrix4(t.parentInverse);for(const [axis,value] of [['X',target.x],['Y',target.y],['Z',target.z]] as const)commands.push(setCommand(PM,t.id,`${prefix}.target${axis}`,value));}
  }
  return commands;
}

/** Absolute values for clearing transforms (Alt+G / Alt+R / Alt+S). */
export function clearCommands(PM:any,t:TransformTarget,mode:TransformMode):any[] {
  if(t.role==='light'||t.role==='camera'){
    if(mode!=='translate')return [];
    return ['x','y','z'].map(axis=>setCommand(PM,t.id,`position.${axis}`,0));
  }
  const paths=mode==='translate'?['position.x','position.y','position.z']:mode==='rotate'?['rotation.x','rotation.y','rotation']:['scale.x','scale.y','scale.z'];
  return paths.map(path=>setCommand(PM,t.id,path,mode==='scale'?100:0));
}
export const propertyCommand=setCommand;
