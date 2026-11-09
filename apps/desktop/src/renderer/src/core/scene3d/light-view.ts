import * as THREE from 'three';
import { parent3D, world3D } from '../../legacy/core/space-3d';
import { layer3DRole,isModelNull,isSceneNull } from './layers';
import {nativeSceneBridge} from './native-transform';
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

/** Model-control pivots use scene units and the rendered scene camera. */
export function nullViewerPosition(PM:any,layerId:string,time:number,rect:Scene3DViewerRect):Point2D|null {
  return nullViewerState(PM,layerId,time,rect)?.viewerPosition??null;
}

/** The controller's own plane, independent of selection and render geometry. */
export function nullViewerState(PM:any,layerId:string,time:number,rect:Scene3DViewerRect):{viewerPosition:Point2D;corners:Point2D[]}|null {
  const layer=PM.L(layerId);
  if(!isSceneNull(PM,layer)||!Number.isFinite(time)||!validRect(rect)||!PM.active(layer,time))return null;
  const camera=cameraFor(PM,time,'viewer'),comp=PM.curComp?.()||PM.proj,model=isModelNull(PM,layer);
  const world=new THREE.Matrix4().fromArray(world3D(PM,layer,time));if(!model)world.premultiply(nativeSceneBridge(comp));
  const anchor=new THREE.Vector3(...['x','y','z'].map(axis=>Number(PM.ev(layer,`anchor.${axis}`,time))||0) as [number,number,number]);
  const project=(position:THREE.Vector3):Point2D|null=>{
    position.applyMatrix4(world);const local=position.clone().applyMatrix4(camera.matrixWorldInverse),ndc=position.project(camera);
    if(local.z>=0||![ndc.x,ndc.y,ndc.z].every(Number.isFinite)||Math.abs(ndc.z)>1)return null;
    return {x:rect.x+(ndc.x+1)*rect.width/2,y:rect.y+(1-ndc.y)*rect.height/2};
  };
  const viewerPosition=project(anchor.clone());if(!viewerPosition)return null;
  const unit=model?6/Math.max(1,comp.h):1,w=(Number(layer.d?.w)||100)*unit/2,h=(Number(layer.d?.h)||100)*unit/2;
  const corners=[[-1,-1],[1,-1],[1,1],[-1,1]].map(([x,y])=>project(anchor.clone().add(new THREE.Vector3(x!*w,y!*h,0))));
  return {viewerPosition,corners:corners.every(Boolean)?corners as Point2D[]:[]};
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

export interface LightViewerGuides {
  layerId: string;
  /** Spot only: outer cone rim at the aim distance, projected; null where a rim point is behind the camera. */
  rim: (Point2D | null)[] | null;
  /** Spot only: the full-intensity inner rim (cone angle reduced by penumbra). */
  innerRim: (Point2D | null)[] | null;
  /** Spot/sun aim point, projected; null for point lights or behind the camera. */
  aim: Point2D | null;
  /** Point/spot light with a finite Range: projected sphere silhouette radius around the light. */
  range: { center: Point2D; radius: number } | null;
}

const RIM_SEGMENTS=48;
/** Read-only drawing guides for the viewer overlay: spot cone and point range. */
export function lightViewerGuides(PM: any, layerId: string, time: number, rect: Scene3DViewerRect, view: LightViewCamera='viewer'): LightViewerGuides | null {
  const state=lightViewerState(PM,layerId,time,rect,view);
  if(!state)return null;
  const layer=PM.L(layerId),camera=cameraFor(PM,time,view);
  const project=(world: THREE.Vector3): Point2D | null => {
    if(world.clone().applyMatrix4(camera.matrixWorldInverse).z>=-1e-6)return null;
    const ndc=world.clone().project(camera);
    if(![ndc.x,ndc.y].every(Number.isFinite))return null;
    return {x:rect.x+(ndc.x+1)*rect.width/2,y:rect.y+(1-ndc.y)*rect.height/2};
  };
  const origin=new THREE.Vector3(state.worldPosition.x,state.worldPosition.y,state.worldPosition.z);
  const target=new THREE.Vector3(...['X','Y','Z'].map(axis=>channel(PM,layer,`target${axis}`,time)) as [number,number,number])
    .applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,layer,time)));
  const out: LightViewerGuides={layerId,rim:null,innerRim:null,aim:state.type==='point' ? null : project(target),range:null};
  if(state.type==='spot' && state.direction && state.coneAngleDegrees!=null){
    const direction=new THREE.Vector3(state.direction.x,state.direction.y,state.direction.z);
    const length=Math.min(25,Math.max(.25,origin.distanceTo(target)));
    const helper=Math.abs(direction.y)<.99 ? new THREE.Vector3(0,1,0) : new THREE.Vector3(1,0,0);
    const u=new THREE.Vector3().crossVectors(direction,helper).normalize(),v=new THREE.Vector3().crossVectors(direction,u).normalize();
    const center=origin.clone().addScaledVector(direction,length);
    const penumbra=Math.min(1,Math.max(0,Number(channel(PM,layer,'penumbra',time)) || 0));
    const ring=(degrees: number)=>{
      const radius=length*Math.tan(THREE.MathUtils.degToRad(degrees));
      return Array.from({length:RIM_SEGMENTS},(_,i)=>{
        const a=i/RIM_SEGMENTS*Math.PI*2;
        return project(center.clone().addScaledVector(u,Math.cos(a)*radius).addScaledVector(v,Math.sin(a)*radius));
      });
    };
    out.rim=ring(state.coneAngleDegrees);
    out.innerRim=penumbra>.02 ? ring(state.coneAngleDegrees*(1-penumbra)) : null;
  }
  const range=Number(channel(PM,layer,'distance',time));
  if(state.type!=='sun' && range>0 && state.inFront && state.viewerPosition){
    const right=new THREE.Vector3().setFromMatrixColumn(camera.matrixWorld,0).normalize();
    const edge=project(origin.clone().addScaledVector(right,range));
    if(edge)out.range={center:state.viewerPosition,radius:Math.hypot(edge.x-state.viewerPosition.x,edge.y-state.viewerPosition.y)};
  }
  return out;
}
