import * as THREE from 'three';
import type {ViewCamera} from './viewport';
import {deltaCommands,upgradeGroupCommands,type TransformTarget,type TransformMode} from './targets';
import {PX_PER_UNIT} from './space';

/*
 * A 3D drag: moving, rotating or scaling the selection while the pointer is
 * held, exactly like a 2D drag. Moving follows the pointer across the screen,
 * or along one axis from an axis handle; Shift locks a free move to its
 * dominant axis and Ctrl snaps (10 px, 5°, 10%). Escape or right-click
 * cancels. The gesture is one Undo; edits are re-applied from its start.
 */

export type ModalMode='translate'|'rotate'|'scale';
export type Axis='x'|'y'|'z';
/** Axes are the composition's: x right, y down, z away from the viewer. */
export type Constraint={axis:Axis};
export type Rect={x:number;y:number;width:number;height:number};
export type Point={x:number;y:number};

export const AXIS_COLORS:Record<Axis,string>={x:'#f0525c',y:'#5cc46a',z:'#4c9bf5'};
const AXES:Axis[]=['x','y','z'];
/** Composition axes in scene units (scene y is up and z toward the viewer). */
export const SCENE_AXIS:Record<Axis,THREE.Vector3>={x:new THREE.Vector3(1,0,0),y:new THREE.Vector3(0,-1,0),z:new THREE.Vector3(0,0,-1)};
const SNAP={translate:10/PX_PER_UNIT,rotate:5,scale:.1};
export const snapValue=(value:number,step:number)=>Math.round(value/step)*step;
const LABEL:Record<ModalMode,string>={translate:'Move',rotate:'Rotate',scale:'Scale'};

/* ── projection helpers ───────────────────────────────── */
/** A copy of the view camera matching the composition rectangle's aspect. */
export function fitCamera(source:ViewCamera,rect:Rect):ViewCamera {
  const camera=source.clone() as ViewCamera,aspect=rect.width/Math.max(1,rect.height);
  if(camera instanceof THREE.PerspectiveCamera)camera.aspect=aspect;
  else {camera.left=-3*aspect;camera.right=3*aspect;camera.top=3;camera.bottom=-3;}
  camera.updateProjectionMatrix();camera.updateMatrixWorld(true);return camera;
}
/** Scene point → rectangle pixels (z<1 means in front of the camera). */
export function toScreen(camera:THREE.Camera,rect:Rect,world:THREE.Vector3):{x:number;y:number;z:number} {
  const v=world.clone().project(camera);
  return {x:rect.x+(v.x+1)/2*rect.width,y:rect.y+(1-v.y)/2*rect.height,z:v.z};
}
export function rayAt(camera:THREE.Camera,rect:Rect,point:Point):THREE.Ray {
  const caster=new THREE.Raycaster();
  caster.setFromCamera(new THREE.Vector2((point.x-rect.x)/rect.width*2-1,1-(point.y-rect.y)/rect.height*2),camera);
  return caster.ray;
}
export const viewForward=(camera:THREE.Camera)=>new THREE.Vector3(0,0,-1).transformDirection(camera.matrixWorld);
function planeHit(camera:THREE.Camera,rect:Rect,point:Point,origin:THREE.Vector3,normal:THREE.Vector3):THREE.Vector3|null {
  return rayAt(camera,rect,point).intersectPlane(new THREE.Plane().setFromNormalAndCoplanarPoint(normal,origin),new THREE.Vector3());
}
/** Parameter along `axis` (through `origin`) closest to the pointer ray; screen projection when nearly parallel. */
function axisParameter(camera:THREE.Camera,rect:Rect,point:Point,origin:THREE.Vector3,axis:THREE.Vector3):number {
  const ray=rayAt(camera,rect,point),w0=origin.clone().sub(ray.origin);
  const b=axis.dot(ray.direction),d=axis.dot(w0),e=ray.direction.dot(w0),denominator=1-b*b;
  if(denominator>1e-3)return (b*e-d)/denominator;
  const a=toScreen(camera,rect,origin),c=toScreen(camera,rect,origin.clone().add(axis));
  const sx=c.x-a.x,sy=c.y-a.y,length=Math.max(1e-6,sx*sx+sy*sy);
  return ((point.x-a.x)*sx+(point.y-a.y)*sy)/length;
}
const fmt=(value:number)=>{const v=Math.abs(value)<.05?0:value;return String(Math.round(v*10)/10).replace('-','−');};

export interface ModalHost {
  PM:any;
  /** The current view camera, fitted to the composition rectangle. */
  camera():ViewCamera;
  rect():Rect;
  changed():void;
  finished(result:'confirm'|'cancel'):void;
}

/** A running drag. Construct with `begin`, feed pointer moves, then confirm or cancel. */
export class ModalTransform {
  mode:ModalMode;
  constraint:Constraint|null=null;
  private start:Point;
  private last:Point;
  private snapping=false;
  private locked=false;
  private angle=0;
  private previousAngle:number|null=null;
  private label='';
  private targets:TransformTarget[];
  private pivot=new THREE.Vector3();
  private fixedConstraint:boolean;
  private done=false;
  /** Read by overlays: the guide line of an axis-constrained move. */
  guides:{origin:THREE.Vector3;direction:THREE.Vector3;axis:Axis}[]=[];
  value={vector:new THREE.Vector3(),angle:0,scale:1};

  private constructor(private host:ModalHost,mode:ModalMode,targets:TransformTarget[],pointer:Point,constraint:Constraint|null){
    this.mode=mode;this.targets=targets;this.start={...pointer};this.last={...pointer};
    this.constraint=constraint;this.fixedConstraint=!!constraint;
  }

  static begin(host:ModalHost,mode:ModalMode,targets:TransformTarget[],pointer:Point,options:{label?:string;prefix?:any[];constraint?:Constraint;pivot?:THREE.Vector3}={}):ModalTransform|null {
    const usable=targets.filter(t=>t.modes.includes(mode)||mode!=='translate');
    if(!usable.length)return null;
    const PM=host.PM,label=options.label||(usable.length===1?`${LABEL[mode]} 3D layer`:`${LABEL[mode]} 3D layers`);
    PM.Edit.begin(label,{origin:'canvas'});
    const upgrades=[...(options.prefix??[]),...upgradeGroupCommands(PM,usable)];
    if(upgrades.length){const result=PM.Edit.apply(upgrades,{origin:'canvas'});if(!result.ok){PM.Edit.cancel();throw new Error(result.message);}}
    const modal=new ModalTransform(host,mode,usable,pointer,options.constraint??null);
    modal.label=label;
    if(options.pivot)modal.pivot.copy(options.pivot);
    else{for(const t of usable)modal.pivot.add(t.position);modal.pivot.divideScalar(usable.length);}
    return modal;
  }
  get active(){return !this.done;}
  get title(){return this.label;}

  /* ── input ──────────────────────────────────────────── */
  move(point:Point,modifiers:{shiftKey:boolean;ctrlKey:boolean;metaKey?:boolean}):void {
    if(this.done)return;
    this.last={...point};this.modifiers(modifiers,false);this.update();
  }
  modifiers(modifiers:{shiftKey:boolean;ctrlKey:boolean;metaKey?:boolean},update=true):void {
    if(this.done)return;
    const snapping=modifiers.ctrlKey||!!modifiers.metaKey,locked=modifiers.shiftKey&&this.mode==='translate'&&!this.fixedConstraint;
    const changed=snapping!==this.snapping||locked!==this.locked;
    this.snapping=snapping;this.locked=locked;
    if(!locked&&!this.fixedConstraint)this.constraint=null;
    if(changed&&update)this.update();
  }
  /** Escape cancels; every other key is ignored while dragging. */
  key(event:{key:string}):boolean {
    if(this.done)return false;
    if(event.key==='Escape')this.cancel();
    return true;
  }
  /** Shift on a free move: lock to the axis that best matches the pointer's motion so far. */
  private dominantAxis():Constraint|null {
    const camera=this.host.camera(),rect=this.host.rect(),origin=toScreen(camera,rect,this.pivot);
    const dx=this.last.x-this.start.x,dy=this.last.y-this.start.y;if(Math.hypot(dx,dy)<4)return null;
    let best:Axis='x',score=-1;
    for(const axis of AXES){
      const end=toScreen(camera,rect,this.pivot.clone().add(SCENE_AXIS[axis]));
      const sx=end.x-origin.x,sy=end.y-origin.y,length=Math.hypot(sx,sy);if(length<1e-6)continue;
      const value=Math.abs((sx*dx+sy*dy)/(length*Math.hypot(dx,dy)));if(value>score){score=value;best=axis;}
    }
    return {axis:best};
  }

  /* ── evaluation ─────────────────────────────────────── */
  private update():void {
    const PM=this.host.PM;
    if(this.locked&&!this.fixedConstraint)this.constraint=this.dominantAxis();
    try{
      const deltas=this.evaluate();
      const commands=this.targets.flatMap((t,i)=>deltaCommands(PM,t,deltas[i]!,this.mode as TransformMode));
      if(commands.length){const result=PM.Edit.apply(commands,{origin:'canvas',label:this.label});if(!result.ok)throw new Error(result.message);}
    }catch(error){PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:'scene3d-modal',error:true});this.cancel();return;}
    this.host.changed();
  }
  /** Per-target scene-space deltas for the current pointer and constraint. */
  evaluate():THREE.Matrix4[] {
    const camera=this.host.camera(),rect=this.host.rect(),pivotScreen=toScreen(camera,rect,this.pivot);
    this.guides=this.constraint?[{origin:this.pivot.clone(),direction:SCENE_AXIS[this.constraint.axis].clone(),axis:this.constraint.axis}]:[];
    if(this.mode==='translate'){
      const vector=new THREE.Vector3();
      if(this.constraint){
        const direction=SCENE_AXIS[this.constraint.axis];
        let amount=axisParameter(camera,rect,this.last,this.pivot,direction)-axisParameter(camera,rect,this.start,this.pivot,direction);
        if(this.snapping)amount=snapValue(amount,SNAP.translate);
        vector.copy(direction).multiplyScalar(amount);
      }else{
        // A free move stays in the plane facing the camera, under the pointer.
        const normal=viewForward(camera).negate(),a=planeHit(camera,rect,this.start,this.pivot,normal),b=planeHit(camera,rect,this.last,this.pivot,normal);
        if(a&&b)vector.copy(b.sub(a));
        if(this.snapping)vector.set(snapValue(vector.x,SNAP.translate),snapValue(vector.y,SNAP.translate),snapValue(vector.z,SNAP.translate));
      }
      this.value.vector.copy(vector);
      return this.targets.map(()=>new THREE.Matrix4().makeTranslation(vector));
    }
    if(this.mode==='rotate'){
      const current=Math.atan2(this.last.y-pivotScreen.y,this.last.x-pivotScreen.x);
      if(this.previousAngle===null)this.previousAngle=Math.atan2(this.start.y-pivotScreen.y,this.start.x-pivotScreen.x);
      let delta=current-this.previousAngle;while(delta>Math.PI)delta-=2*Math.PI;while(delta<-Math.PI)delta+=2*Math.PI;
      this.angle+=delta;this.previousAngle=current;
      // Screen y points down: clockwise on screen is negative about the axis facing the viewer.
      let angle=-THREE.MathUtils.radToDeg(this.angle);if(this.snapping)angle=snapValue(angle,SNAP.rotate);
      this.value.angle=angle;
      const toViewer=viewForward(camera).negate();
      let axis=toViewer.clone(),radians=THREE.MathUtils.degToRad(angle);
      if(this.constraint){axis=SCENE_AXIS[this.constraint.axis].clone();if(axis.dot(toViewer)<0)radians=-radians;}
      const rotation=new THREE.Matrix4().makeTranslation(this.pivot).multiply(new THREE.Matrix4().makeRotationAxis(axis.normalize(),radians)).multiply(new THREE.Matrix4().makeTranslation(this.pivot.clone().negate()));
      return this.targets.map(()=>rotation.clone());
    }
    // Scale: the pointer's distance from the pivot relative to where it started.
    const a=Math.hypot(this.start.x-pivotScreen.x,this.start.y-pivotScreen.y),b=Math.hypot(this.last.x-pivotScreen.x,this.last.y-pivotScreen.y);
    let factor=a<1?1:b/a;if(this.snapping)factor=snapValue(factor,SNAP.scale);factor=Math.max(.001,factor);
    this.value.scale=factor;
    const scale=new THREE.Matrix4().makeTranslation(this.pivot).multiply(new THREE.Matrix4().makeScale(factor,factor,factor)).multiply(new THREE.Matrix4().makeTranslation(this.pivot.clone().negate()));
    return this.targets.map(()=>scale.clone());
  }

  /** A short readout for the pointer: the move in composition pixels, the angle, or the scale. */
  readout():string {
    // Clockwise on screen reads positive, like the 2D rotate handle.
    if(this.mode==='rotate')return `${fmt(-this.value.angle)}°`;
    if(this.mode==='scale')return `${Math.round(this.value.scale*100)}%`;
    const v=this.value.vector,px={x:v.x*PX_PER_UNIT,y:-v.y*PX_PER_UNIT,z:-v.z*PX_PER_UNIT};
    if(this.constraint)return `${this.constraint.axis.toUpperCase()} ${fmt(px[this.constraint.axis])}`;
    return `X ${fmt(px.x)}  Y ${fmt(px.y)}  Z ${fmt(px.z)}`;
  }

  confirm():void {
    if(this.done)return;this.done=true;
    try{this.host.PM.Edit.commit(this.label);}catch(error){this.host.PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:'scene3d-modal',error:true});}
    this.guides=[];this.host.finished('confirm');
  }
  cancel():void {
    if(this.done)return;this.done=true;
    try{this.host.PM.Edit.cancel();}catch{/* already closed */}
    this.guides=[];this.host.finished('cancel');
  }
}
