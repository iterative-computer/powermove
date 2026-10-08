import * as THREE from 'three';
import {layer3DRole,isModelGroup,type Layer3DRole} from './layers';
import {parent3D,world3D} from '../../legacy/core/space-3d';
import {sceneFromComp,GEOMETRY_TO_COMP} from './space';
import type {SceneRuntime} from './runtime';

/*
 * What a 3D transform acts on, and how a delta becomes edits to each real
 * layer's own channels. Models, lights, cameras, model groups and 3D-enabled
 * 2D layers are all targets. Layer channels live in composition pixels; the
 * camera, gizmo and pointer work in scene units, so a target carries both:
 * `world`/`pivot` in pixels for writing, `position`/`rotation` in scene units
 * for interaction. Every gesture writes identical, undoable, keyframe-aware edits.
 */

export type TransformMode='translate'|'rotate'|'scale';
export type TargetRole=Layer3DRole|'group'|'plane';
export type TransformTarget={
  id:string;layer:any;role:TargetRole;modes:TransformMode[];
  /** Models: the runtime node, used for bounds. */
  object:THREE.Object3D|null;
  /** Pixel-space world matrix and parent inverse, for writing channels. */
  world:THREE.Matrix4;parentInverse:THREE.Matrix4;
  /** Pixel-space pivot (the layer's anchor in the world). */
  pivot:THREE.Vector3;
  /** Scene-space pivot and orientation, for the camera, gizmo and pointer. */
  position:THREE.Vector3;
  rotation:THREE.Quaternion;
  /** Scene-space aim point for lights and cameras. */
  target:THREE.Vector3|null;
  orientation:THREE.Vector3;
  /** A target may supply its own edits for a scene-space delta. */
  edit?:(delta:THREE.Matrix4,mode:TransformMode)=>any[];
};

const vec=(values:number[])=>new THREE.Vector3(values[0],values[1],values[2]);
const tidy=(value:number)=>Math.round(value*1e6)/1e6;
const comp=(PM:any)=>PM.curComp?.()||PM.proj;
/** Every 3D layer is a transform target: models, lights, cameras, model groups and 3D-enabled 2D layers or groups. */
export function roleOf(PM:any,layer:any):TargetRole|null {
  const role=layer3DRole(layer);if(role)return role;
  if(isModelGroup(PM,layer)||(layer?.type==='group'&&layer.threeD))return 'group';
  return layer?.threeD&&layer.type!=='audio'&&layer.type!=='adjustment'?'plane':null;
}
export const editableLayer=(PM:any,layer:any)=>!!layer&&!layer.lock&&!(PM.groupAncestors?.(layer)||[]).some((g:any)=>g.lock);
const ev=(PM:any,l:any,prop:any,path:string,fallback=0)=>prop?Number(PM.evP(l,prop,PM.time,path)):fallback;
export const channel=(PM:any,l:any,path:string,fallback=0)=>ev(PM,l,l.p?.[path],path,fallback);

/** Which transforms apply: a sun only aims, a point light only moves, a camera never scales. */
export function transformModes(role:TargetRole,lightType?:string):TransformMode[] {
  if(role==='object'||role==='group'||role==='plane')return ['translate','rotate','scale'];
  if(role==='camera')return ['translate','rotate'];
  return lightType==='sun'?['rotate']:lightType==='point'?['translate']:['translate','rotate'];
}

/** A light's or camera's aim point, in composition pixels. */
export function aimPointPx(PM:any,layer:any):THREE.Vector3|null {
  const role=layer3DRole(layer),content=role==='light'?layer.d?.data?.light?.p:role==='camera'?layer.d?.data?.camera?.p:null;
  if(!content)return null;
  return vec(['X','Y','Z'].map(axis=>ev(PM,layer,content[`target${axis}`],`${role}.target${axis}`))).applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,layer,PM.time)));
}
/** A light's or camera's aim point, in scene units. */
export function aimPoint(PM:any,layer:any):THREE.Vector3|null {
  const point=aimPointPx(PM,layer);return point?point.applyMatrix4(sceneFromComp(comp(PM))):null;
}

export function resolveTargets(PM:any,ids:readonly string[],runtime:SceneRuntime|null,options:{excludeCamera?:string|null}={}):TransformTarget[] {
  const out:TransformTarget[]=[],toScene=sceneFromComp(comp(PM));
  for(const id of ids){
    const l=PM.L?.(id),role=roleOf(PM,l);
    if(!role||!editableLayer(PM,l)||(role==='camera'&&id===options.excludeCamera))continue;
    if(PM.active&&!PM.active(l,PM.time))continue;
    const lightType=role==='light'?l.d?.data?.light?.type:undefined;
    const object=role==='object'?runtime?.objects.get(id)??null:null;
    const world=new THREE.Matrix4().fromArray(world3D(PM,l,PM.time));
    const pivot=role==='light'||role==='camera'?new THREE.Vector3().setFromMatrixPosition(world)
      :vec(['x','y','z'].map(axis=>channel(PM,l,`anchor.${axis}`))).applyMatrix4(world);
    let aimPx=role==='light'||role==='camera'?aimPointPx(PM,l):null;
    if(role==='light'||role==='camera'){
      // A light or camera's frame looks from its position toward its aim point.
      const look=new THREE.Matrix4().lookAt(pivot,aimPx&&aimPx.distanceToSquared(pivot)>1e-9?aimPx:pivot.clone().add(new THREE.Vector3(0,0,1)),new THREE.Vector3(0,-1,0));
      world.copy(look).setPosition(pivot);
    }
    if(role==='group'&&!l.threeD){
      // A plain artwork group pivots on the centre of its models.
      const box=new THREE.Box3();
      for(const child of comp(PM).layers)if((PM.groupAncestors?.(child)||[]).some((g:any)=>g.id===id)){
        const node=runtime?.objects.get(child.id);if(node)box.expandByObject(node);
      }
      if(!box.isEmpty())pivot.copy(box.getCenter(new THREE.Vector3()).applyMatrix4(toScene.clone().invert()));
    }
    const rotation=new THREE.Quaternion();
    toScene.clone().multiply(world).multiply(role==='object'?GEOMETRY_TO_COMP:new THREE.Matrix4()).decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
    out.push({id,layer:l,role,modes:transformModes(role,lightType),object,world,
      parentInverse:new THREE.Matrix4().fromArray(parent3D(PM,l,PM.time)).invert(),pivot,position:pivot.clone().applyMatrix4(toScene),rotation,
      target:aimPx?aimPx.applyMatrix4(toScene):null,orientation:vec(['x','y','z'].map(axis=>channel(PM,l,`orientation.${axis}`)))});
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
    const anchor=t.pivot.clone().applyMatrix4(t.world.clone().invert());
    const old=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`anchor.${axis}`)));
    const local=t.parentInverse.clone().multiply(t.world),motion=anchor.clone().sub(old).applyMatrix3(new THREE.Matrix3().setFromMatrix4(local));
    const position=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`position.${axis}`))).add(motion);
    commands.push({type:'set_layer',target:t.id,patch:{threeD:true}});
    for(const [axis,value] of [['x',anchor.x],['y',anchor.y],['z',anchor.z]] as const)commands.push(setCommand(PM,t.id,`anchor.${axis}`,value));
    for(const [axis,value] of [['x',position.x],['y',position.y],['z',position.z]] as const)commands.push(setCommand(PM,t.id,`position.${axis}`,value));
  }
  return commands;
}

/** A scene-space delta expressed in composition pixels. */
export function deltaInPixels(PM:any,delta:THREE.Matrix4):THREE.Matrix4 {
  const toScene=sceneFromComp(comp(PM));return toScene.clone().invert().multiply(delta).multiply(toScene);
}
/**
 * Edits that place `t` at `delta × start world` (`delta` in scene units).
 * Channels are written as absolute values from the gesture's start, so
 * re-applying is idempotent. `moveAim` carries a light's or camera's aim point
 * along with a move (Alt).
 */
export function deltaCommands(PM:any,t:TransformTarget,delta:THREE.Matrix4,mode:TransformMode,options:{moveAim?:boolean}={}):any[] {
  if(t.edit)return t.edit(delta,mode);
  const px=deltaInPixels(PM,delta),commands:any[]=[];
  if(t.role!=='light'&&t.role!=='camera'){
    const local=t.parentInverse.clone().multiply(px.clone().multiply(t.world));
    const position=new THREE.Vector3(),quaternion=new THREE.Quaternion(),scale=new THREE.Vector3();local.decompose(position,quaternion,scale);
    const anchor=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`anchor.${axis}`)));
    position.add(anchor.applyMatrix3(new THREE.Matrix3().setFromMatrix4(local)));
    const orientation=new THREE.Quaternion().setFromEuler(new THREE.Euler(...t.orientation.toArray().map(THREE.MathUtils.degToRad) as [number,number,number],'ZYX'));
    quaternion.multiply(orientation.invert());
    const euler=new THREE.Euler().setFromQuaternion(quaternion,'ZYX'),deg=THREE.MathUtils.radToDeg;
    // Orientation is applied separately; write back rotation alone.
    const values:[string,number][]=[['position.x',position.x],['position.y',position.y],['position.z',position.z],
      ['rotation.x',deg(euler.x)],['rotation.y',deg(euler.y)],['rotation',deg(euler.z)],
      ['scale.x',scale.x*100],['scale.y',scale.y*100],['scale.z',scale.z*100]];
    for(const [path,value] of values)
      if(t.layer.p?.[path]&&(path.startsWith('position.')||(mode==='rotate'?path.startsWith('rotation'):mode==='scale'?path.startsWith('scale.'):false)))commands.push(setCommand(PM,t.id,path,value));
    return commands;
  }
  const prefix=t.role==='light'?'light':'camera',position=t.pivot.clone().applyMatrix4(px);
  if(t.modes.includes('translate')&&position.distanceToSquared(t.pivot)>1e-12){
    const motion=position.clone().sub(t.pivot).applyMatrix3(new THREE.Matrix3().setFromMatrix4(t.parentInverse));
    const localPosition=vec(['x','y','z'].map(axis=>channel(PM,t.layer,`position.${axis}`))).add(motion);
    for(const [axis,value] of [['x',localPosition.x],['y',localPosition.y],['z',localPosition.z]] as const)commands.push(setCommand(PM,t.id,`position.${axis}`,value));
  }
  const aim=aimPointPx(PM,t.layer);
  if(aim){
    // Moving keeps the aim point (Alt moves it along); rotating swings it around the position.
    const rotation=new THREE.Quaternion();px.decompose(new THREE.Vector3(),rotation,new THREE.Vector3());
    const target=mode==='rotate'?position.clone().add(aim.clone().sub(t.pivot).applyQuaternion(rotation))
      :options.moveAim?aim.clone().add(position.clone().sub(t.pivot)):null;
    if(target){target.applyMatrix4(t.parentInverse);for(const [axis,value] of [['X',target.x],['Y',target.y],['Z',target.z]] as const)commands.push(setCommand(PM,t.id,`${prefix}.target${axis}`,value));}
  }
  return commands;
}
export const propertyCommand=setCommand;
