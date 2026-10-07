import * as THREE from 'three';
import type {ViewCamera} from './viewport';
import type {ViewportOrientation,ViewportPivot} from './editor-state';
import {deltaCommands,upgradeGroupCommands,type TransformTarget,type TransformMode} from './targets';

/*
 * Blender's modal transform operator (G, R, S, R R, and Tweak drags).
 * The pointer drives the transform until LMB/Enter/Space confirms or
 * RMB/Escape cancels. X/Y/Z constrain to an axis (press again for the other
 * orientation, a third time to clear), Shift+X/Y/Z to a plane, digits type an
 * exact value, Ctrl inverts snapping and Shift gives precision. The whole
 * gesture is one Undo; edits are re-applied from the start state each time.
 */

export type ModalMode='translate'|'rotate'|'scale'|'trackball';
export type Axis='x'|'y'|'z';
export type Constraint={axis:Axis;plane:boolean;orientation:ViewportOrientation};
export type Rect={x:number;y:number;width:number;height:number};
export type Point={x:number;y:number};

export const AXIS_COLORS:Record<Axis,string>={x:'#ff3352',y:'#8bdc00',z:'#2890ff'};
const AXES:Axis[]=['x','y','z'];
const UNIT:Record<Axis,THREE.Vector3>={x:new THREE.Vector3(1,0,0),y:new THREE.Vector3(0,1,0),z:new THREE.Vector3(0,0,1)};
const LABEL:Record<ModalMode,string>={translate:'Move',rotate:'Rotate',scale:'Resize',trackball:'Trackball'};

/** First press uses the header orientation, the second the other one (Local when Global), the third clears. */
export function cycleConstraint(current:Constraint|null,axis:Axis,plane:boolean,preferred:ViewportOrientation):Constraint|null {
  const alternate:ViewportOrientation=preferred==='global'?'local':'global';
  if(!current||current.axis!==axis||current.plane!==plane)return {axis,plane,orientation:preferred};
  if(current.orientation===preferred&&alternate!==preferred)return {axis,plane,orientation:alternate};
  return null;
}
/** Blender's numeric input: digits, one point, a leading minus; "-" anywhere toggles the sign. */
export function typeNumber(input:string,key:string):string {
  if(key==='Backspace')return input.slice(0,-1);
  if(key==='-')return input.startsWith('-')?input.slice(1):'-'+input;
  if(key==='.'&&!input.includes('.'))return input+'.';
  if(/^\d$/.test(key))return input+key;
  return input;
}
export function parseTyped(input:string):number|null {
  if(input===''||input==='-'||input==='.'||input==='-.')return null;
  const value=Number(input);return Number.isFinite(value)?value:null;
}
export const snapValue=(value:number,step:number)=>Math.round(value/step)*step;
export const SNAP_STEPS:Record<ModalMode,[number,number]>={translate:[1,.1],rotate:[5,1],scale:[.1,.01],trackball:[5,1]};

/* ── projection helpers ───────────────────────────────── */
/** A copy of the view camera matching the composition rectangle's aspect. */
export function fitCamera(source:ViewCamera,rect:Rect):ViewCamera {
  const camera=source.clone() as ViewCamera,aspect=rect.width/Math.max(1,rect.height);
  if(camera instanceof THREE.PerspectiveCamera)camera.aspect=aspect;
  else {camera.left=-3*aspect;camera.right=3*aspect;camera.top=3;camera.bottom=-3;}
  camera.updateProjectionMatrix();camera.updateMatrixWorld(true);return camera;
}
/** World point → stage-relative CSS pixels (z<1 means in front of the camera). */
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
/** Parameter along `axis` (through `origin`) closest to the pointer ray; falls back to screen projection when nearly parallel. */
function axisParameter(camera:THREE.Camera,rect:Rect,point:Point,origin:THREE.Vector3,axis:THREE.Vector3):number {
  const ray=rayAt(camera,rect,point),w0=origin.clone().sub(ray.origin);
  const b=axis.dot(ray.direction),d=axis.dot(w0),e=ray.direction.dot(w0),denominator=1-b*b;
  if(denominator>1e-3)return (b*e-d)/denominator;
  const a=toScreen(camera,rect,origin),c=toScreen(camera,rect,origin.clone().add(axis));
  const sx=c.x-a.x,sy=c.y-a.y,length=Math.max(1e-6,sx*sx+sy*sy);
  return ((point.x-a.x)*sx+(point.y-a.y)*sy)/length;
}
export function orientationFrame(orientation:ViewportOrientation,target:{rotation:THREE.Quaternion}|null,camera:THREE.Camera):THREE.Quaternion {
  if(orientation==='local'&&target)return target.rotation.clone();
  if(orientation==='view'){const q=new THREE.Quaternion();camera.matrixWorld.decompose(new THREE.Vector3(),q,new THREE.Vector3());return q;}
  return new THREE.Quaternion();
}
export const fmt=(value:number,digits=4)=>{const v=Math.abs(value)<5e-7?0:value;return Number(v.toFixed(digits)).toString();};

export interface ModalHost {
  PM:any;
  /** The current view camera, fitted to the composition rectangle. */
  camera():ViewCamera;
  rect():Rect;
  /** The 3D cursor, used by the 3D Cursor pivot. */
  cursor():THREE.Vector3;
  orientation:ViewportOrientation;
  pivot:ViewportPivot;
  snap:boolean;
  activeId:string|null;
  changed():void;
  finished(result:'confirm'|'cancel'):void;
}

/** A running G/R/S. Construct with `begin`, feed pointer and key events, then confirm or cancel. */
export class ModalTransform {
  mode:ModalMode;
  constraint:Constraint|null=null;
  inputs:string[]=['','',''];
  inputIndex=0;
  typing=false;
  readonly tweak:boolean;
  private start:Point;
  private virtual:Point;
  private last:Point;
  private precise=false;
  private snapping=false;
  private angle=0;
  private previousAngle:number|null=null;
  private label='';
  private targets:TransformTarget[];
  private pivot=new THREE.Vector3();
  private done=false;
  /** Read by overlays: world-space guide lines and the screen-space pivot link. */
  guides:{origin:THREE.Vector3;direction:THREE.Vector3;axis:Axis}[]=[];
  link:{from:Point;to:Point}|null=null;
  value={vector:new THREE.Vector3(),angle:0,trackball:[0,0] as [number,number],scale:new THREE.Vector3(1,1,1)};

  private constructor(private host:ModalHost,mode:ModalMode,targets:TransformTarget[],pointer:Point,tweak:boolean){
    this.mode=mode;this.targets=targets;this.start={...pointer};this.virtual={...pointer};this.last={...pointer};this.tweak=tweak;
  }

  /** `prefix` edits run first in the same Undo (Extrude); `constraint` starts constrained (along the extrusion normal). */
  static begin(host:ModalHost,mode:ModalMode,targets:TransformTarget[],pointer:Point,options:{tweak?:boolean;label?:string;prefix?:any[];constraint?:Constraint}={}):ModalTransform|null {
    const usable=targets.filter(t=>t.modes.includes(mode==='trackball'?'rotate':mode)||mode!=='translate');
    if(!usable.length)return null;
    const PM=host.PM,label=options.label||(usable.length===1?`${LABEL[mode]} 3D layer`:`${LABEL[mode]} 3D layers`);
    PM.Edit.begin(label,{origin:'canvas'});
    const upgrades=[...(options.prefix??[]),...upgradeGroupCommands(PM,usable)];
    if(upgrades.length){const result=PM.Edit.apply(upgrades,{origin:'canvas'});if(!result.ok){PM.Edit.cancel();throw new Error(result.message);}}
    const modal=new ModalTransform(host,mode,usable,pointer,!!options.tweak);
    modal.label=label;modal.placePivot();if(options.constraint)modal.constraint={...options.constraint};
    return modal;
  }
  get active(){return !this.done;}
  get title(){return this.label;}

  private placePivot():void {
    const pivot=this.host.pivot,list=this.targets;
    if(pivot==='cursor')this.pivot.copy(this.host.cursor());
    else if(pivot==='active'){const active=list.find(t=>t.id===this.host.activeId)??list.at(-1)!;this.pivot.copy(active.position);}
    else if(pivot==='bounds'){const box=new THREE.Box3();for(const t of list)box.expandByPoint(t.position);box.getCenter(this.pivot);}
    else {this.pivot.set(0,0,0);for(const t of list)this.pivot.add(t.position);this.pivot.divideScalar(list.length);}
  }
  private activeTarget(){return this.targets.find(t=>t.id===this.host.activeId)??this.targets.at(-1)??null;}
  private frame(target:TransformTarget|null,orientation=this.constraint?.orientation??'global'):THREE.Quaternion {
    return orientationFrame(orientation,target,this.host.camera());
  }
  private pivotFor(t:TransformTarget):THREE.Vector3 {return this.host.pivot==='individual'?t.position:this.pivot;}
  private snapOn(){return this.host.snap!==this.snapping;}

  /* ── input ──────────────────────────────────────────── */
  move(point:Point,modifiers:{shiftKey:boolean;ctrlKey:boolean;metaKey?:boolean}):void {
    if(this.done)return;
    this.precise=modifiers.shiftKey;this.snapping=modifiers.ctrlKey||!!modifiers.metaKey;
    const factor=this.precise?.1:1;
    this.virtual={x:this.virtual.x+(point.x-this.last.x)*factor,y:this.virtual.y+(point.y-this.last.y)*factor};
    this.last={...point};
    this.update();
  }
  modifiers(modifiers:{shiftKey:boolean;ctrlKey:boolean;metaKey?:boolean}):void {
    if(this.done)return;
    const snapping=modifiers.ctrlKey||!!modifiers.metaKey;
    if(snapping!==this.snapping){this.snapping=snapping;this.update();}
    this.precise=modifiers.shiftKey;
  }
  /** Returns true when the key belongs to the modal (it is consumed either way while running). */
  key(event:{key:string;code?:string;shiftKey:boolean;ctrlKey:boolean;altKey:boolean;metaKey:boolean}):boolean {
    if(this.done)return false;
    const key=event.key,lower=key.length===1?key.toLowerCase():key;
    if(key==='Escape'){this.cancel();return true;}
    if(key==='Enter'||key===' '||key==='NumpadEnter'){this.confirm();return true;}
    if(['x','y','z'].includes(lower)&&!event.ctrlKey&&!event.metaKey&&!event.altKey){
      const axis=lower as Axis;
      if(this.mode==='trackball')this.mode='rotate';
      // Rotation has no plane; Blender treats Shift+axis as the axis itself there.
      this.constraint=cycleConstraint(this.constraint,axis,event.shiftKey&&this.mode!=='rotate',this.host.orientation);
      if(this.typing&&this.constraint&&!this.constraint.plane)this.inputIndex=0;
      this.update();return true;
    }
    if(['g','r','s'].includes(lower)&&!event.ctrlKey&&!event.metaKey&&!event.altKey){
      const next:ModalMode=lower==='g'?'translate':lower==='s'?'scale':this.mode==='rotate'?'trackball':'rotate';
      if(next==='translate'&&!this.targets.some(t=>t.modes.includes('translate')))return true;
      this.mode=next;this.resetInput();this.angle=0;this.previousAngle=null;
      if(next==='trackball')this.constraint=null;
      this.update();return true;
    }
    if(key==='Tab'){
      if(!this.typing)return true;
      const count=this.components();this.inputIndex=(this.inputIndex+1)%count;this.update();return true;
    }
    if(/^\d$/.test(key)||key==='.'||key==='-'||key==='Backspace'){
      if(key==='Backspace'&&!this.typing)return true;
      this.typing=true;this.inputs[this.inputIndex]=typeNumber(this.inputs[this.inputIndex]!,key);
      if(key==='Backspace'&&this.inputs.every(v=>v===''))this.typing=false;
      this.update();return true;
    }
    if(key==='Shift'||key==='Control'||key==='Meta'){this.modifiers(event);return true;}
    return true;
  }
  /** MMB during a move/resize: constrain to the world axis that best matches the pointer's motion. */
  autoConstrain():void {
    if(this.done||this.mode==='trackball')return;
    const camera=this.host.camera(),rect=this.host.rect(),origin=toScreen(camera,rect,this.pivot);
    const dx=this.virtual.x-this.start.x,dy=this.virtual.y-this.start.y;
    if(Math.hypot(dx,dy)<4){this.constraint=null;this.update();return;}
    let best:Axis='x',score=-1;
    for(const axis of AXES){
      const end=toScreen(camera,rect,this.pivot.clone().add(UNIT[axis].clone().applyQuaternion(this.frame(this.activeTarget(),this.host.orientation))));
      const sx=end.x-origin.x,sy=end.y-origin.y,length=Math.hypot(sx,sy);if(length<1e-6)continue;
      const value=Math.abs((sx*dx+sy*dy)/(length*Math.hypot(dx,dy)));if(value>score){score=value;best=axis;}
    }
    this.constraint={axis:best,plane:false,orientation:this.host.orientation};this.update();
  }
  private resetInput(){this.inputs=['','',''];this.inputIndex=0;this.typing=false;}
  private components():number {
    if(this.mode==='rotate')return 1;
    if(this.mode==='trackball')return 2;
    if(this.constraint)return this.constraint.plane?2:1;
    return 3;
  }
  private typed(index:number):number|null {return parseTyped(this.inputs[index]!);}

  /* ── evaluation ─────────────────────────────────────── */
  private constraintAxes():Axis[] {
    if(!this.constraint)return AXES;
    return this.constraint.plane?AXES.filter(a=>a!==this.constraint!.axis):[this.constraint.axis];
  }
  private update():void {
    const PM=this.host.PM;
    try{
      const deltas=this.evaluate();
      const commands=this.targets.flatMap((t,i)=>deltaCommands(PM,t,deltas[i]!,this.mode==='trackball'?'rotate':this.mode as TransformMode));
      if(commands.length){const result=PM.Edit.apply(commands,{origin:'canvas',label:this.label});if(!result.ok)throw new Error(result.message);}
    }catch(error){PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:'scene3d-modal',error:true});this.cancel();return;}
    this.host.changed();
  }
  /** Per-target world deltas for the current pointer, constraint and typed input. */
  evaluate():THREE.Matrix4[] {
    const camera=this.host.camera(),rect=this.host.rect(),snap=this.snapOn(),[coarse,fine]=SNAP_STEPS[this.mode],step=this.precise?fine:coarse;
    const active=this.activeTarget(),activeFrame=this.frame(active);
    const pivotScreen=toScreen(camera,rect,this.pivot);
    this.guides=[];this.link=null;
    const frames=this.targets.map(t=>this.constraint?.orientation==='local'?t.rotation.clone():activeFrame.clone());
    if(this.constraint){
      const axes=this.constraintAxes(),origins=this.constraint.orientation==='local'?this.targets.map(t=>this.pivotFor(t)):[this.pivot];
      origins.forEach((origin,i)=>{for(const axis of axes)this.guides.push({origin:origin.clone(),direction:UNIT[axis].clone().applyQuaternion(this.constraint!.orientation==='local'?frames[i]!:activeFrame),axis});});
    }
    if(this.mode==='translate'){
      const vector=new THREE.Vector3();let world:THREE.Vector3|null=null;
      if(this.typing){
        const axes=this.constraint?this.constraintAxes():AXES;
        axes.forEach((axis,i)=>{vector[axis]=this.typed(i)??0;});
      }else if(!this.constraint){
        const normal=viewForward(camera).negate(),a=planeHit(camera,rect,this.start,this.pivot,normal),b=planeHit(camera,rect,this.virtual,this.pivot,normal);
        world=a&&b?b.sub(a):new THREE.Vector3();
        vector.copy(world);
        if(snap){vector.set(snapValue(vector.x,step),snapValue(vector.y,step),snapValue(vector.z,step));world=vector.clone();}
      }else if(!this.constraint.plane){
        const direction=UNIT[this.constraint.axis].clone().applyQuaternion(activeFrame);
        let amount=axisParameter(camera,rect,this.virtual,this.pivot,direction)-axisParameter(camera,rect,this.start,this.pivot,direction);
        if(snap)amount=snapValue(amount,step);
        vector[this.constraint.axis]=amount;
      }else {
        const normal=UNIT[this.constraint.axis].clone().applyQuaternion(activeFrame);
        const a=planeHit(camera,rect,this.start,this.pivot,normal),b=planeHit(camera,rect,this.virtual,this.pivot,normal);
        if(a&&b)vector.copy(b.sub(a).applyQuaternion(activeFrame.clone().invert()));
        vector[this.constraint.axis]=0;
        if(snap)vector.set(snapValue(vector.x,step),snapValue(vector.y,step),snapValue(vector.z,step));
      }
      this.value.vector.copy(vector);
      return this.targets.map((_t,i)=>new THREE.Matrix4().makeTranslation(world&&!this.typing?world:vector.clone().applyQuaternion(this.typing&&!this.constraint?new THREE.Quaternion():frames[i]!)));
    }
    if(this.mode==='rotate'){
      let angle:number;
      if(this.typing)angle=this.typed(0)??0;
      else {
        const current=Math.atan2(this.virtual.y-pivotScreen.y,this.virtual.x-pivotScreen.x);
        if(this.previousAngle===null)this.previousAngle=Math.atan2(this.start.y-pivotScreen.y,this.start.x-pivotScreen.x);
        let delta=current-this.previousAngle;while(delta>Math.PI)delta-=2*Math.PI;while(delta<-Math.PI)delta+=2*Math.PI;
        this.angle+=delta;this.previousAngle=current;
        // Screen y points down: clockwise on screen is negative about the axis facing the viewer.
        angle=-THREE.MathUtils.radToDeg(this.angle);
        if(snap)angle=snapValue(angle,step);
      }
      this.value.angle=angle;
      this.link={from:{x:pivotScreen.x,y:pivotScreen.y},to:{...this.last}};
      const toViewer=viewForward(camera).negate();
      return this.targets.map((t,i)=>{
        let axis=toViewer.clone(),radians=THREE.MathUtils.degToRad(angle);
        if(this.constraint){
          axis=UNIT[this.constraint.axis].clone().applyQuaternion(frames[i]!);
          // Typed values follow the right-hand rule; pointer motion follows the screen.
          if(!this.typing&&axis.dot(toViewer)<0)radians=-radians;
        }
        const pivot=this.pivotFor(t);
        return new THREE.Matrix4().makeTranslation(pivot).multiply(new THREE.Matrix4().makeRotationAxis(axis.normalize(),radians)).multiply(new THREE.Matrix4().makeTranslation(pivot.clone().negate()));
      });
    }
    if(this.mode==='trackball'){
      let ax:number,ay:number;
      if(this.typing){ax=this.typed(0)??0;ay=this.typed(1)??0;}
      else {ax=(this.virtual.x-this.start.x)*.5;ay=(this.virtual.y-this.start.y)*.5;if(snap){ax=snapValue(ax,step);ay=snapValue(ay,step);}}
      this.value.trackball=[ax,ay];
      const q=new THREE.Quaternion();camera.matrixWorld.decompose(new THREE.Vector3(),q,new THREE.Vector3());
      const up=new THREE.Vector3(0,1,0).applyQuaternion(q),right=new THREE.Vector3(1,0,0).applyQuaternion(q);
      const rotation=new THREE.Matrix4().makeRotationAxis(up,THREE.MathUtils.degToRad(ax)).multiply(new THREE.Matrix4().makeRotationAxis(right,THREE.MathUtils.degToRad(ay)));
      return this.targets.map(t=>{const pivot=this.pivotFor(t);return new THREE.Matrix4().makeTranslation(pivot).multiply(rotation).multiply(new THREE.Matrix4().makeTranslation(pivot.clone().negate()));});
    }
    // Resize: the pointer's distance from the pivot relative to where it started.
    let factor:number;
    const scale=new THREE.Vector3(1,1,1);
    if(this.typing){
      const axes=this.constraint?this.constraintAxes():AXES;
      if(this.constraint)axes.forEach((axis,i)=>{scale[axis]=this.typed(i)??1;});
      else {const first=this.typed(0);scale.set(first??1,this.typed(1)??first??1,this.typed(2)??first??1);}
    }else {
      const a=Math.hypot(this.start.x-pivotScreen.x,this.start.y-pivotScreen.y),b=Math.hypot(this.virtual.x-pivotScreen.x,this.virtual.y-pivotScreen.y);
      const sign=Math.sign((this.start.x-pivotScreen.x)*(this.virtual.x-pivotScreen.x)+(this.start.y-pivotScreen.y)*(this.virtual.y-pivotScreen.y))||1;
      factor=a<1?1:sign*b/a;
      if(snap)factor=snapValue(factor,step);
      for(const axis of this.constraintAxes())scale[axis]=factor;
    }
    for(const axis of AXES)if(Math.abs(scale[axis])<1e-4)scale[axis]=scale[axis]<0?-1e-4:1e-4;
    this.value.scale.copy(scale);
    this.link={from:{x:pivotScreen.x,y:pivotScreen.y},to:{...this.last}};
    return this.targets.map((t,i)=>{
      const pivot=this.pivotFor(t),frame=new THREE.Matrix4().makeRotationFromQuaternion(frames[i]!);
      return new THREE.Matrix4().makeTranslation(pivot).multiply(frame).multiply(new THREE.Matrix4().makeScale(scale.x,scale.y,scale.z))
        .multiply(frame.clone().invert()).multiply(new THREE.Matrix4().makeTranslation(pivot.clone().negate()));
    });
  }

  /* ── presentation ───────────────────────────────────── */
  private field(index:number,value:number,unit=''):string {
    if(!this.typing)return `${fmt(value)}${unit}`;
    const text=this.inputs[index]||'';
    return index===this.inputIndex?`[${text}|]${unit}`:`${text||fmt(value)}${unit}`;
  }
  private along():string {
    if(!this.constraint)return '';
    const name={global:'global',local:'local',view:'view'}[this.constraint.orientation];
    return this.constraint.plane?` locking ${name} ${this.constraint.axis.toUpperCase()}`:` along ${name} ${this.constraint.axis.toUpperCase()}`;
  }
  /** Blender's header text, e.g. "D: 1.5 (1.5) along global X". */
  header():string {
    const {vector,angle,trackball,scale}=this.value;
    if(this.mode==='translate'){
      if(this.constraint&&!this.constraint.plane){const value=vector[this.constraint.axis];return `D: ${this.field(0,value)} (${fmt(Math.abs(value))})${this.along()}`;}
      const axes=this.constraint?this.constraintAxes():AXES;
      return `${axes.map((axis,i)=>`D${axis}: ${this.field(i,vector[axis])}`).join('  ')} (${fmt(vector.length())})${this.along()}`;
    }
    if(this.mode==='rotate')return `Rot: ${this.field(0,angle,'°')}${this.along()}`;
    if(this.mode==='trackball')return `Trackball: ${this.field(0,trackball[0],'°')} ${this.field(1,trackball[1],'°')}`;
    if(this.constraint&&!this.constraint.plane)return `Scale: ${this.field(0,scale[this.constraint.axis])}${this.along()}`;
    const axes=this.constraint?this.constraintAxes():AXES;
    return `Scale ${axes.map((axis,i)=>`${axis.toUpperCase()}: ${this.field(i,scale[axis])}`).join('  ')}${this.along()}`;
  }
  hints():[string,string][] {
    return [['LMB','Confirm'],['RMB','Cancel'],['X Y Z','Axis'],...(this.mode==='rotate'?[]:[['⇧ X Y Z','Plane']] as [string,string][]),
      ['Ctrl','Snap'],['⇧','Precision'],['G R S','Switch'],['0-9','Value']];
  }

  confirm():void {
    if(this.done)return;this.done=true;
    try{this.host.PM.Edit.commit(this.label);}catch(error){this.host.PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:'scene3d-modal',error:true});}
    this.guides=[];this.link=null;this.host.finished('confirm');
  }
  cancel():void {
    if(this.done)return;this.done=true;
    try{this.host.PM.Edit.cancel();}catch{/* already closed */}
    this.guides=[];this.link=null;this.host.finished('cancel');
  }
}
