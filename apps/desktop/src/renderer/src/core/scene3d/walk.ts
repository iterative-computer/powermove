import * as THREE from 'three';
import {viewportPose,applyViewportPose,getViewportMode,activeCameraLayer,viewPosition,type ViewPose} from './viewport';
import {cameraPoseCommands} from './navigation';
import {editableLayer} from './targets';

/*
 * Blender's Walk Navigation (Shift+`). The mouse looks around, WASD moves,
 * Q/E move down/up, Shift is faster and Alt slower, the wheel changes speed.
 * LMB, Enter or Space confirms; RMB or Escape returns to where it started.
 * In camera view it flies the real camera (one Undo); otherwise the user view.
 */

const LOOK=.0025,KEYS=new Set(['w','a','s','d','q','e','arrowup','arrowdown','arrowleft','arrowright']);
export function walkStep(pose:ViewPose,position:THREE.Vector3,move:THREE.Vector3,look:{x:number;y:number}):{pose:ViewPose;position:THREE.Vector3} {
  let rotation=new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(0,1,0),-look.x*LOOK).multiply(pose.rotation);
  const pitched=rotation.clone().multiply(new THREE.Quaternion().setFromAxisAngle(new THREE.Vector3(1,0,0),-look.y*LOOK));
  // Never pitch past straight up or down.
  if(Math.abs(new THREE.Vector3(0,0,-1).applyQuaternion(pitched).y)<.995)rotation=pitched;
  rotation.normalize();
  const forward=new THREE.Vector3(0,0,-1).applyQuaternion(rotation),right=new THREE.Vector3(1,0,0).applyQuaternion(rotation);
  const flat=new THREE.Vector3(forward.x,0,forward.z);if(flat.lengthSq()<1e-9)flat.set(0,0,-1);flat.normalize();
  const next=position.clone().addScaledVector(flat,move.z).addScaledVector(right,move.x).add(new THREE.Vector3(0,move.y,0));
  return {position:next,pose:{rotation,distance:pose.distance,target:next.clone().addScaledVector(forward,pose.distance)}};
}

export interface WalkSession {header():string;hints():[string,string][];confirm():void;cancel():void}
export function startWalk(PM:any,stage:HTMLElement,changed:()=>void,finished:()=>void):WalkSession|null {
  const inCamera=getViewportMode(PM)==='camera',camera=inCamera?activeCameraLayer(PM):null;
  if(inCamera&&(!camera||!editableLayer(PM,camera)))throw new Error(camera?'Unlock the camera first':'Add a camera first');
  const start=viewportPose(PM);let pose=start,position=viewPosition(start);
  let speed=Math.max(.5,start.distance*.6),look={x:0,y:0},last=performance.now(),frame=0,done=false;
  const held=new Set<string>(),offs:(()=>void)[]=[];
  if(camera)PM.Edit.begin('Walk camera',{origin:'canvas'});
  const apply=()=>{
    if(camera){const result=PM.Edit.apply(cameraPoseCommands(PM,camera,pose),{origin:'canvas',label:'Walk camera'});if(!result.ok)throw new Error(result.message);}
    else applyViewportPose(PM,pose);
  };
  const tick=(now:number)=>{
    frame=0;if(done)return;
    const dt=Math.min(.1,(now-last)/1000);last=now;
    const fast=held.has('shift')?3:held.has('alt')?.25:1,step=speed*fast*dt;
    const move=new THREE.Vector3((held.has('d')||held.has('arrowright')?1:0)-(held.has('a')||held.has('arrowleft')?1:0),
      (held.has('e')?1:0)-(held.has('q')?1:0),(held.has('w')||held.has('arrowup')?1:0)-(held.has('s')||held.has('arrowdown')?1:0)).multiplyScalar(step);
    if(move.lengthSq()||look.x||look.y){
      ({pose,position}=walkStep(pose,position,move,look));look={x:0,y:0};
      try{apply();}catch(error){PM.toast?.(error instanceof Error?error.message:String(error),2200,{key:'scene3d-walk',error:true});cancel();return;}
      changed();
    }
    frame=requestAnimationFrame(tick);
  };
  const capture=(type:string,handler:any,target:EventTarget=window)=>{target.addEventListener(type,handler,{capture:true,passive:false});offs.push(()=>target.removeEventListener(type,handler,{capture:true} as any));};
  const stop=(e:Event)=>{e.preventDefault();e.stopImmediatePropagation();};
  capture('mousemove',(e:MouseEvent)=>{look.x+=e.movementX;look.y+=e.movementY;});
  capture('pointermove',stop);
  capture('pointerdown',(e:PointerEvent)=>{stop(e);if(e.button===0)confirm();else if(e.button===2)cancel();});
  capture('contextmenu',stop);
  capture('wheel',(e:WheelEvent)=>{stop(e);speed=Math.min(1e4,Math.max(.01,speed*(e.deltaY<0?1.2:1/1.2)));changed();});
  capture('keydown',(e:KeyboardEvent)=>{
    stop(e);const key=e.key.toLowerCase();
    if(key==='escape')cancel();else if(key==='enter'||key===' ')confirm();
    else if(key==='+'||key==='='){speed*=1.2;changed();}else if(key==='-'){speed/=1.2;changed();}
    else if(KEYS.has(key))held.add(key);
    if(e.shiftKey)held.add('shift');if(e.altKey)held.add('alt');
  });
  capture('keyup',(e:KeyboardEvent)=>{stop(e);held.delete(e.key.toLowerCase());if(!e.shiftKey)held.delete('shift');if(!e.altKey)held.delete('alt');});
  capture('blur',()=>cancel());
  try{(stage.requestPointerLock as any)?.call(stage)?.catch?.(()=>{});}catch{/* pointer lock is optional */}
  frame=requestAnimationFrame(tick);
  function end(){
    done=true;cancelAnimationFrame(frame);for(const off of offs.splice(0))off();
    try{if(document.pointerLockElement===stage)document.exitPointerLock();}catch{/* optional */}
    finished();
  }
  function confirm(){if(done)return;if(camera){try{PM.Edit.commit('Walk camera');}catch(error){PM.toast?.(String(error),2200,{key:'scene3d-walk',error:true});}}end();}
  function cancel(){if(done)return;if(camera)PM.Edit.cancel();else applyViewportPose(PM,start);end();}
  return {
    header:()=>`Walk${camera?' (camera)':''} · speed ${Number(speed.toPrecision(3))}`,
    hints:()=>[['W A S D','Move'],['Q E','Down/Up'],['⇧','Fast'],['Wheel','Speed'],['LMB','Confirm'],['RMB','Cancel']],
    confirm,cancel
  };
}
