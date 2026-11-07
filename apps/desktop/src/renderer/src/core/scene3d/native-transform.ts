import * as THREE from 'three';
import {poseValues3D} from '../../legacy/core/space-3d';

/** The same basis is used by mesh rendering and editor manipulation. */
export function nativeSceneBridge(comp:{w:number;h:number}):THREE.Matrix4 {
  const unit=6/Math.max(1,comp.h);
  return new THREE.Matrix4().makeTranslation(-comp.w*unit/2,3,0).multiply(new THREE.Matrix4().makeScale(unit,-unit,-unit));
}

/** Convert a transformed native local matrix back to editable channels. Retain
 * Orientation, and resolve XY skew rather than losing it during decomposition.
 * The layer format cannot represent XZ/YZ shear: refuse that pose atomically. */
export function nativeTransformValues(PM:any,layer:any,local:THREE.Matrix4,time:number):Record<string,number> {
  const values=poseValues3D(PM,layer,local.elements,time,new THREE.Matrix4().elements);
  const deg=THREE.MathUtils.degToRad,rad=THREE.MathUtils.radToDeg;
  const rotation=new THREE.Quaternion().setFromEuler(new THREE.Euler(deg(values['rotation.x']!),deg(values['rotation.y']!),deg(values.rotation!),'ZYX'));
  const orientation=new THREE.Quaternion().setFromEuler(new THREE.Euler(...['x','y','z'].map(axis=>deg(Number(PM.ev(layer,`orientation.${axis}`,time) || 0))) as [number,number,number],'ZYX'));
  const euler=new THREE.Euler().setFromQuaternion(rotation.multiply(orientation.invert()),'ZYX');
  values['rotation.x']=rad(euler.x);values['rotation.y']=rad(euler.y);values.rotation=rad(euler.z);
  for(const axis of ['x','y','z'])delete values[`orientation.${axis}`];
  return values;
}
