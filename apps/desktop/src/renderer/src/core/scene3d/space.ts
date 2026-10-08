import * as THREE from 'three';

/*
 * One 3D space for every layer. Like 2D layers, 3D models, lights, cameras
 * and 3D-enabled 2D layers keep their channels in composition pixels: origin
 * at the top-left, +x right, +y down, +z away from the viewer. The renderer
 * and Blender work in scene units (y up, origin at the composition centre);
 * this module is the only conversion between the two.
 */

/** Unit geometry (a 1-unit cube, a model normalised to a unit sphere) is this many pixels at 100% scale. */
export const PX_PER_UNIT=200;
/** The default lens matches the 2D layers' 3D switch: 50 mm on a 36 mm film gate, centred on the composition. */
export const DEFAULT_LENS_MM=50;

type Size={w:number;h:number};
const size=(comp:any):Size=>({w:Math.max(1,Number(comp?.w)||1920),h:Math.max(1,Number(comp?.h)||1080)});

/** Composition pixels → scene units. A 180° turn about x plus a uniform scale, so it keeps handedness. */
export function sceneFromComp(comp:any):THREE.Matrix4 {
  const {w,h}=size(comp),k=1/PX_PER_UNIT;
  return new THREE.Matrix4().makeScale(k,-k,-k).multiply(new THREE.Matrix4().makeTranslation(-w/2,-h/2,0));
}
export const compFromScene=(comp:any)=>sceneFromComp(comp).invert();
/** A model's own unit geometry (y up) expressed in the composition's pixel frame (y down). */
export const GEOMETRY_TO_COMP=new THREE.Matrix4().makeScale(PX_PER_UNIT,-PX_PER_UNIT,-PX_PER_UNIT);

export const toScene=(comp:any,point:THREE.Vector3)=>point.clone().applyMatrix4(sceneFromComp(comp));
export const toComp=(comp:any,point:THREE.Vector3)=>point.clone().applyMatrix4(compFromScene(comp));
/** Pixel lengths (light range, area size, clip planes) to scene units and back. */
export const lengthToScene=(px:number)=>px/PX_PER_UNIT;
export const lengthToComp=(units:number)=>units*PX_PER_UNIT;

/** The default camera: centred on the composition, framing the z=0 plane at exactly 1:1. */
export function defaultLens(comp:any):{distance:number;fov:number} {
  const {w,h}=size(comp),distance=w*DEFAULT_LENS_MM/36;
  return {distance,fov:THREE.MathUtils.radToDeg(2*Math.atan(h/2/distance))};
}
export function defaultCameraChannels(comp:any):{x:number;y:number;z:number;targetX:number;targetY:number;targetZ:number;fov:number;near:number;far:number} {
  const {w,h}=size(comp),{distance,fov}=defaultLens(comp);
  return {x:w/2,y:h/2,z:-distance,targetX:w/2,targetY:h/2,targetZ:0,fov:Math.round(fov*1000)/1000,near:1,far:100000};
}
/** A key light above, left of and in front of the centre, aimed at the centre. */
export function defaultLightChannels(comp:any):{x:number;y:number;z:number;targetX:number;targetY:number;targetZ:number} {
  const {w,h}=size(comp),k=PX_PER_UNIT;
  return {x:w/2-3*k,y:h/2-5*k,z:-4*k,targetX:w/2,targetY:h/2,targetZ:0};
}
export const compCenter=(comp:any)=>{const {w,h}=size(comp);return {x:w/2,y:h/2,z:0};};
/** New models keep the same size relative to the frame: 1 at 1080p, smaller in smaller compositions. */
export const modelScale=(comp:any)=>{const {w,h}=size(comp);return Math.round(Math.min(w,h)/1080*1000)/1000;};

/** Configure a scene camera as the default composition camera. */
export function applyDefaultCamera(camera:THREE.PerspectiveCamera|THREE.OrthographicCamera,comp:any):void {
  const {distance,fov}=defaultLens(comp);
  camera.position.set(0,0,distance/PX_PER_UNIT);camera.up.set(0,1,0);camera.lookAt(0,0,0);
  camera.near=.01;camera.far=Math.max(1000,distance/PX_PER_UNIT*50);
  if(camera instanceof THREE.PerspectiveCamera)camera.fov=fov;
  camera.updateProjectionMatrix();camera.updateMatrixWorld(true);
}

/*
 * Converting stored transforms from the old scene-unit convention (y up,
 * origin at the centre) to composition pixels. Absolute positions move to the
 * composition frame; positions relative to a converted parent only scale and flip.
 */
const flip=(v:number)=>v===0?0:-v;
export type ChannelConverter=(value:number)=>number;
export function unitChannelConverters(comp:any,relative:boolean):Record<string,ChannelConverter> {
  const {w,h}=size(comp),k=PX_PER_UNIT;
  const x:ChannelConverter=relative?v=>v*k:v=>w/2+v*k,y:ChannelConverter=relative?v=>-v*k:v=>h/2-v*k,z:ChannelConverter=v=>flip(v*k);
  return {
    'position.x':x,'position.y':y,'position.z':z,
    'anchor.x':v=>v*k,'anchor.y':v=>flip(v*k),'anchor.z':v=>flip(v*k),
    'rotation.y':flip,rotation:flip,'orientation.y':flip,'orientation.z':flip,
    targetX:x,targetY:y,targetZ:z,
    distance:v=>v*k,width:v=>v*k,height:v=>v*k,near:v=>v*k,far:v=>v*k
  };
}
