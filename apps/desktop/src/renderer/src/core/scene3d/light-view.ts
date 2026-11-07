import * as THREE from 'three';
import { parent3D, world3D } from '../../legacy/core/space-3d';
import { layer3DRole } from './layers';
import { compositionRuntime } from './service';
import { validateSceneChannel } from './schema';
import { viewportCamera } from './viewport';

export type Point2D = { x: number; y: number };
export type Point3D = Point2D & { z: number };
/** Displayed composition image rectangle, in the same coordinates as the pointer. */
export type Scene3DViewerRect = Point2D & { width: number; height: number };
export type LightViewCamera = 'viewer' | 'camera';
export interface LightViewerState {
  layerId: string;
  worldPosition: Point3D;
  viewerPosition: Point2D | null;
  /** In front of the camera, independent of layer activity and viewport clipping. */
  inFront: boolean;
  /** Active light in front of the camera. An offscreen marker can still be visible. */
  visible: boolean;
  inFrame: boolean;
  color: string;
  intensity: number;
  type: 'sun' | 'point' | 'spot';
  /** Normalized world direction toward the aim target; null for point/coincident aim. */
  direction: Point3D | null;
  /** Three.js spot half-angle, in degrees. Full cone opening is twice this. */
  coneAngleDegrees: number | null;
}
const point = (v: THREE.Vector3): Point3D => ({ x: v.x, y: v.y, z: v.z });
const validRect = (r: Scene3DViewerRect) => [r.x,r.y,r.width,r.height].every(Number.isFinite) && r.width>0 && r.height>0;

function channel(PM: any, layer: any, key: string, time: number): any {
  const prop=layer.d.data.light.p[key],path=`light.${key}`;
  try {
    const value=PM.evP ? PM.evP(layer,prop,time,path) : prop.v;
    validateSceneChannel(path,value);return value;
  } catch { return prop.v; }
}
function cameraFor(PM: any, time: number, view: LightViewCamera) {
  const world=compositionRuntime(PM,time),comp=PM.curComp?.() || PM.proj;
  const camera=(view==='camera' ? world.camera : viewportCamera(PM,world.camera)).clone();
  world.resizeCamera(comp.w,comp.h,camera);
  return camera;
}

/** Read-only marker data. Uses the same evaluated hierarchy/camera as rendering. */
export function lightViewerState(PM: any, layerId: string, time: number, rect: Scene3DViewerRect, view: LightViewCamera='viewer'): LightViewerState | null {
  const layer=PM.L(layerId);
  if(layer3DRole(layer)!=='light' || !Number.isFinite(time) || !validRect(rect))return null;
  const camera=cameraFor(PM,time,view),position=new THREE.Vector3().setFromMatrixPosition(new THREE.Matrix4().fromArray(world3D(PM,layer,time)));
  const local=position.clone().applyMatrix4(camera.matrixWorldInverse),ndc=position.clone().project(camera);
  const inFront=local.z<0,projectable=[ndc.x,ndc.y,ndc.z].every(Number.isFinite) && Math.abs(local.z)>1e-10;
  const target=new THREE.Vector3(...['X','Y','Z'].map(axis=>channel(PM,layer,`target${axis}`,time)) as [number,number,number])
    .applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,layer,time)));
  const direction=target.sub(position),type=layer.d.data.light.type;
  return {
    layerId,worldPosition:point(position),viewerPosition:projectable ? {x:rect.x+(ndc.x+1)*rect.width/2,y:rect.y+(1-ndc.y)*rect.height/2} : null,
    inFront,visible:inFront && (PM.active ? PM.active(layer,time) : layer.on!==false),
    inFrame:inFront && projectable && Math.abs(ndc.x)<=1 && Math.abs(ndc.y)<=1 && Math.abs(ndc.z)<=1,
    color:channel(PM,layer,'color',time),intensity:channel(PM,layer,'intensity',time),type,
    direction:type!=='point' && direction.lengthSq()>1e-20 ? point(direction.normalize()) : null,
    coneAngleDegrees:type==='spot' ? channel(PM,layer,'angle',time) : null,
  };
}

/** Unproject onto the camera-facing plane through the light at time t.
 * Returns native parent-space Position values, including anchor compensation.
 * Does not mutate data: apply the result with normal guarded edit commands.
 */
export function lightPositionFromViewer(PM: any, layerId: string, time: number, pointer: Point2D, rect: Scene3DViewerRect, view: LightViewCamera='viewer'): Point3D | null {
  if(![pointer.x,pointer.y].every(Number.isFinite))return null;
  const state=lightViewerState(PM,layerId,time,rect,view);
  if(!state?.inFront)return null;
  const layer=PM.L(layerId),camera=cameraFor(PM,time,view),origin=new THREE.Vector3(state.worldPosition.x,state.worldPosition.y,state.worldPosition.z);
  const parent=new THREE.Matrix4().fromArray(parent3D(PM,layer,time));
  if(Math.abs(parent.determinant())<1e-12)return null;
  const ray=new THREE.Raycaster();
  ray.setFromCamera(new THREE.Vector2((pointer.x-rect.x)/rect.width*2-1,1-(pointer.y-rect.y)/rect.height*2),camera);
  const plane=new THREE.Plane().setFromNormalAndCoplanarPoint(camera.getWorldDirection(new THREE.Vector3()),origin);
  const hit=ray.ray.intersectPlane(plane,new THREE.Vector3());if(!hit)return null;
  parent.invert();
  const delta=hit.applyMatrix4(parent).sub(origin.applyMatrix4(parent));
  const position=new THREE.Vector3(...['x','y','z'].map(axis=>Number(PM.ev(layer,`position.${axis}`,time))) as [number,number,number]).add(delta);
  return position.toArray().every(Number.isFinite) ? point(position) : null;
}
