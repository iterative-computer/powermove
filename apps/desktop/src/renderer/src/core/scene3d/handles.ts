import * as THREE from 'three';
import {world3D} from '../../legacy/core/space-3d';
import {layer3DRole} from './layers';
import {sceneFromComp} from './space';
import {toScreen,SCENE_AXIS,AXIS_COLORS,type Axis,type Rect,type Point} from './modal';
import type {TransformTarget} from './targets';
import type {SceneRuntime} from './runtime';

/*
 * The 3D selection drawn like a 2D selection: the selection's bounding box on
 * screen with corner handles (scale), the rotate stem (turn in the view), the
 * anchor, and three short axis spokes (move along x, y or z). With the Rotate
 * tool, rings replace the spokes (turn about x, y or z). Lights and cameras
 * add an aim handle at the point they face. All geometry is in stage pixels.
 */

export type GizmoHandle={kind:'corner';index:number}|{kind:'rotate'}|{kind:'axis';axis:Axis}|{kind:'ring';axis:Axis}|{kind:'aim';id:string};
export type GizmoGeometry={
  box:Point[]|null;
  stem:{from:Point;to:Point}|null;
  pivot:Point;
  pivotScene:THREE.Vector3;
  axes:{axis:Axis;tip:Point;color:string}[];
  rings:{axis:Axis;points:Point[];color:string}[];
  aims:{id:string;from:Point;at:Point}[];
};
const SPOKE=54,RING=46,STEM=22;

/** Scene units per screen pixel at `point`, so handles keep a constant size. */
function perPixel(camera:THREE.Camera,rect:Rect,point:THREE.Vector3):number {
  if((camera as THREE.OrthographicCamera).isOrthographicCamera){const c=camera as THREE.OrthographicCamera;return (c.top-c.bottom)/c.zoom/Math.max(1,rect.height);}
  const c=camera as THREE.PerspectiveCamera,depth=Math.max(1e-4,-point.clone().applyMatrix4(c.matrixWorldInverse).z);
  return 2*depth*Math.tan(THREE.MathUtils.degToRad(c.fov)/2)/Math.max(1,rect.height);
}

export function computeGizmo(PM:any,targets:readonly TransformTarget[],camera:THREE.Camera,rect:Rect,runtime:SceneRuntime,tool:string):GizmoGeometry|null {
  if(!targets.length)return null;
  const comp=PM.curComp?.()||PM.proj,toScene=sceneFromComp(comp),corners:THREE.Vector3[]=[];
  const members=(id:string)=>comp.layers.filter((l:any)=>(PM.groupAncestors?.(l)||[]).some((g:any)=>g.id===id));
  const addObject=(object:THREE.Object3D|undefined)=>{
    if(!object)return;const box=new THREE.Box3().setFromObject(object);if(box.isEmpty())return;
    for(const x of [box.min.x,box.max.x])for(const y of [box.min.y,box.max.y])for(const z of [box.min.z,box.max.z])corners.push(new THREE.Vector3(x,y,z));
  };
  const addPlane=(layer:any)=>{
    const b=PM.GL?.bounds?.(layer,PM.time);if(!b)return;
    const matrix=toScene.clone().multiply(new THREE.Matrix4().fromArray(world3D(PM,layer,PM.time)));
    for(const [x,y] of [[b.x0,b.y0],[b.x1,b.y0],[b.x1,b.y1],[b.x0,b.y1]])corners.push(new THREE.Vector3(x,y,0).applyMatrix4(matrix));
  };
  for(const t of targets){
    if(t.role==='object')addObject(runtime.objects.get(t.id));
    else if(t.role==='plane')addPlane(t.layer);
    else if(t.role==='group')for(const child of members(t.id)){const object=runtime.objects.get(child.id);if(object)addObject(object);else if(child.type!=='group'&&child.type!=='audio'&&!layer3DRole(child))addPlane(child);}
  }
  const pivotScene=new THREE.Vector3();for(const t of targets)pivotScene.add(t.position);pivotScene.divideScalar(targets.length);
  const pivotScreen=toScreen(camera,rect,pivotScene);
  if(pivotScreen.z>=1||pivotScreen.z<=-1)return null;
  const pivot={x:pivotScreen.x,y:pivotScreen.y};
  let box:Point[]|null=null,stem:GizmoGeometry['stem']=null;
  const projected=corners.map(c=>toScreen(camera,rect,c)).filter(p=>p.z<1&&p.z>-1);
  if(projected.length){
    const x0=Math.min(...projected.map(p=>p.x)),x1=Math.max(...projected.map(p=>p.x)),y0=Math.min(...projected.map(p=>p.y)),y1=Math.max(...projected.map(p=>p.y));
    box=[{x:x0,y:y0},{x:x1,y:y0},{x:x1,y:y1},{x:x0,y:y1}];
    if(targets.some(t=>t.modes.includes('rotate')))stem={from:{x:(x0+x1)/2,y:y0},to:{x:(x0+x1)/2,y:y0-STEM}};
  }
  const unit=perPixel(camera,rect,pivotScene),axes:GizmoGeometry['axes']=[],rings:GizmoGeometry['rings']=[];
  const canMove=targets.every(t=>t.modes.includes('translate')),canRotate=targets.every(t=>t.modes.includes('rotate'));
  for(const axis of ['x','y','z'] as Axis[]){
    const direction=SCENE_AXIS[axis];
    if(tool==='rotate'){
      if(!canRotate)continue;
      // A ring in the plane perpendicular to the axis.
      const u=new THREE.Vector3().crossVectors(direction,Math.abs(direction.y)>.9?new THREE.Vector3(1,0,0):new THREE.Vector3(0,1,0)).normalize(),v=new THREE.Vector3().crossVectors(direction,u);
      const points:Point[]=[];
      for(let i=0;i<=48;i++){const a=i/48*Math.PI*2,p=toScreen(camera,rect,pivotScene.clone().addScaledVector(u,Math.cos(a)*RING*unit).addScaledVector(v,Math.sin(a)*RING*unit));points.push({x:p.x,y:p.y});}
      rings.push({axis,points,color:AXIS_COLORS[axis]});
    }else if(canMove){
      const tip=toScreen(camera,rect,pivotScene.clone().addScaledVector(direction,SPOKE*unit));
      // An axis seen end-on has no useful spoke.
      if(Math.hypot(tip.x-pivot.x,tip.y-pivot.y)>8)axes.push({axis,tip:{x:tip.x,y:tip.y},color:AXIS_COLORS[axis]});
    }
  }
  const aims:GizmoGeometry['aims']=[];
  for(const t of targets)if(t.target){
    const from=toScreen(camera,rect,t.position),at=toScreen(camera,rect,t.target);
    if(at.z<1&&at.z>-1)aims.push({id:t.id,from:{x:from.x,y:from.y},at:{x:at.x,y:at.y}});
  }
  return {box,stem,pivot,pivotScene,axes,rings,aims};
}

const distanceToSegment=(p:Point,a:Point,b:Point)=>{
  const dx=b.x-a.x,dy=b.y-a.y,length=dx*dx+dy*dy,t=length?Math.max(0,Math.min(1,((p.x-a.x)*dx+(p.y-a.y)*dy)/length)):0;
  return Math.hypot(p.x-a.x-t*dx,p.y-a.y-t*dy);
};
export function hitGizmo(geometry:GizmoGeometry|null,point:Point):GizmoHandle|null {
  if(!geometry)return null;
  const near=(p:Point,radius:number)=>Math.hypot(point.x-p.x,point.y-p.y)<=radius;
  if(geometry.stem&&near(geometry.stem.to,8))return {kind:'rotate'};
  if(geometry.box)for(let index=0;index<4;index++)if(near(geometry.box[index]!,8))return {kind:'corner',index};
  for(const aim of geometry.aims)if(near(aim.at,9))return {kind:'aim',id:aim.id};
  for(const axis of geometry.axes)if(near(axis.tip,9)||distanceToSegment(point,geometry.pivot,axis.tip)<=4&&Math.hypot(point.x-geometry.pivot.x,point.y-geometry.pivot.y)>10)return {kind:'axis',axis:axis.axis};
  for(const ring of geometry.rings)for(let i=1;i<ring.points.length;i++)if(distanceToSegment(point,ring.points[i-1]!,ring.points[i]!)<=5)return {kind:'ring',axis:ring.axis};
  return null;
}
export const CURSORS:Record<GizmoHandle['kind'],string>={corner:'nwse-resize',rotate:'grab',axis:'pointer',ring:'grab',aim:'crosshair'};
