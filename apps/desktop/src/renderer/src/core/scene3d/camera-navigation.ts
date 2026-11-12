import * as THREE from 'three';
import { parent3D } from '../../legacy/core/space-3d';
import { layer3DRole } from './layers';
import type { ViewPose } from './viewport';

/** Use the same first active camera as the composition renderer, even if locked. */
export function activeCameraLayer(PM:any,time=PM.time):any|null {
  return (PM.curComp?.() || PM.proj).layers.find((layer:any)=>layer3DRole(layer)==='camera' && PM.active(layer,time)) ?? null;
}

/** A camera drag writes real channels, in parent space, in one native Undo step. */
export function cameraNavigationEdit(PM:any,layer:any,start:ViewPose,label:string) {
  const composition=PM.curComp?.() || PM.proj,time=PM.time,autokey=PM.autokey;
  const parent=new THREE.Matrix4().fromArray(parent3D(PM,layer,time));
  if(Math.abs(parent.determinant())<1e-12)throw new Error('Increase the camera parent’s zero scale before moving the camera');
  const inverse=parent.clone().invert(),linear=new THREE.Matrix3().setFromMatrix4(inverse);
  const position=new THREE.Vector3(...['x','y','z'].map(axis=>PM.ev(layer,`position.${axis}`,time)) as [number,number,number]);
  // Camera anchors and their own rotation/scale can offset the matrix origin.
  // Translating by a world delta preserves all of those existing channels.
  const worldPosition=start.position.clone();
  let live=false,closed=false;
  function valid() {
    if(closed)throw new Error('Camera gesture has ended');
    if((PM.curComp?.() || PM.proj)!==composition || PM.time!==time || activeCameraLayer(PM,time)?.id!==layer.id)
      throw new Error('Camera navigation cancelled because the composition, camera or playhead changed');
    const current=PM.L(layer.id);
    if(!current || current.lock || (PM.groupAncestors?.(current) || []).some((group:any)=>group.lock))throw new Error('Unlock the active camera before moving it');
  }
  valid();
  return {
    update(next:ViewPose) {
      valid();
      const current=PM.L(layer.id),local=position.clone().add(next.position.clone().sub(worldPosition).applyMatrix3(linear));
      const target=next.target.clone().applyMatrix4(inverse);
      const values:Record<string,number>={'position.x':local.x,'position.y':local.y,'position.z':local.z,
        'camera.targetX':target.x,'camera.targetY':target.y,'camera.targetZ':target.z};
      if(current.d.data.camera.projection==='orthographic')values['camera.zoom']=next.zoom;
      const commands:any[]=[];
      for(const [path,value] of Object.entries(values)){
        const prop=path.startsWith('position.')?current.p[path]:current.d.data.camera.p[path.slice(7)];
        const evaluated=Number(PM.evP(current,prop,time,path));
        if(Math.abs(value-evaluated)<1e-8)continue;
        if(prop.expr)throw new Error(`Disable the expression on ${path} before moving the camera`);
        commands.push({type:'set_property',target:layer.id,path,value,time,mode:autokey?'keyframe':'auto',preserveHandEdits:false,markIntent:'human'});
      }
      if(!commands.length)return;
      if(!live){PM.Edit.begin(label,{origin:'canvas'});live=true;}
      const result=PM.Edit.apply(commands,{origin:'canvas',label});
      if(!result.ok)throw new Error(result.message);
    },
    commit() {
      valid();closed=true;
      if(live){const result=PM.Edit.commit(label);live=false;if(!result.ok)throw new Error(result.message);}
    },
    cancel() {if(live){PM.Edit.cancel();live=false;}closed=true;},
  };
}
