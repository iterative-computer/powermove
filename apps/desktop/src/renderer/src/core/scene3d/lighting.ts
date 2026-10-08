import * as THREE from 'three';
import {layer3DRole} from './layers';
import {parent3D,world3D} from '../../legacy/core/space-3d';
import {aimPointPx,editableLayer,propertyCommand} from './targets';
import {compCenter,PX_PER_UNIT} from './space';
import {sceneCommands} from './operations';

/*
 * Lighting as directions around the subject. A light's direction is the unit
 * vector from its aim point to the light, in composition pixels (+y down,
 * -z toward the viewer), which a light ball shows as a dot on a sphere seen
 * from the camera. Presets replace the composition's lights in one Undo.
 */

export type LightInfo={id:string;name:string;type:'sun'|'point'|'spot'|'area';color:string;intensity:number;direction:[number,number,number];locked:boolean};
export type LightingPreset={id:string;label:string;lights:{name:string;type:'sun'|'point'|'spot'|'area';azimuth:number;elevation:number;color:string;intensity:number}[]};

/** Azimuth turns from the camera toward +x (right); elevation lifts toward screen-up. */
export function directionFrom(azimuth:number,elevation:number):THREE.Vector3 {
  const a=THREE.MathUtils.degToRad(azimuth),e=THREE.MathUtils.degToRad(elevation);
  return new THREE.Vector3(Math.sin(a)*Math.cos(e),-Math.sin(e),-Math.cos(a)*Math.cos(e)).normalize();
}
export const LIGHTING_PRESETS:LightingPreset[]=[
  {id:'studio',label:'Studio',lights:[
    {name:'Key light',type:'sun',azimuth:-40,elevation:35,color:'#FFF4E8',intensity:2.6},
    {name:'Fill light',type:'sun',azimuth:55,elevation:10,color:'#DCE7FF',intensity:.8},
    {name:'Rim light',type:'sun',azimuth:165,elevation:30,color:'#FFFFFF',intensity:1.6}]},
  {id:'soft',label:'Soft',lights:[
    {name:'Key light',type:'sun',azimuth:-20,elevation:50,color:'#FFFFFF',intensity:1.7},
    {name:'Fill light',type:'sun',azimuth:60,elevation:20,color:'#F1F4FF',intensity:1.1},
    {name:'Bounce light',type:'sun',azimuth:0,elevation:-35,color:'#FFF6EE',intensity:.5}]},
  {id:'dramatic',label:'Dramatic',lights:[
    {name:'Key light',type:'sun',azimuth:-75,elevation:20,color:'#FFE2C4',intensity:3.2},
    {name:'Rim light',type:'sun',azimuth:150,elevation:15,color:'#9FC2FF',intensity:2.2}]},
  {id:'top',label:'Top',lights:[
    {name:'Top light',type:'sun',azimuth:0,elevation:80,color:'#FFFFFF',intensity:2.8},
    {name:'Fill light',type:'sun',azimuth:30,elevation:5,color:'#E6ECFF',intensity:.4}]},
  {id:'sunset',label:'Sunset',lights:[
    {name:'Sun',type:'sun',azimuth:-80,elevation:8,color:'#FF9A55',intensity:3},
    {name:'Sky light',type:'sun',azimuth:90,elevation:25,color:'#6B8CFF',intensity:.6}]}
];

const comp=(PM:any)=>PM.curComp?.()||PM.proj;
const position=(PM:any,layer:any)=>new THREE.Vector3().setFromMatrixPosition(new THREE.Matrix4().fromArray(world3D(PM,layer,PM.time)));

export function listLights(PM:any):LightInfo[] {
  return comp(PM).layers.filter((l:any)=>layer3DRole(l)==='light'&&l.on!==false).map((l:any)=>{
    const light=l.d.data.light,ev=(key:string)=>PM.evP(l,light.p[key],PM.time,`light.${key}`);
    const target=aimPointPx(PM,l)??new THREE.Vector3(),direction=position(PM,l).sub(target);
    if(direction.lengthSq()<1e-9)direction.set(0,-1,-1);direction.normalize();
    return {id:l.id,name:l.name,type:light.type,color:String(ev('color')),intensity:Number(ev('intensity')),direction:direction.toArray() as [number,number,number],
      locked:!editableLayer(PM,l)};
  });
}
/** Edits that swing a light around its aim point to `direction`, keeping its distance. */
export function aimLightCommands(PM:any,id:string,direction:[number,number,number]):any[] {
  const layer=PM.L(id);
  if(layer3DRole(layer)!=='light')throw new Error('Choose a light');
  if(!editableLayer(PM,layer))throw new Error('Unlock the light first');
  const target=aimPointPx(PM,layer)??new THREE.Vector3(),current=position(PM,layer);
  const distance=Math.max(current.distanceTo(target),PX_PER_UNIT);
  const dir=new THREE.Vector3(...direction);if(dir.lengthSq()<1e-9)throw new Error('Choose a direction');
  const next=target.clone().addScaledVector(dir.normalize(),distance).applyMatrix4(new THREE.Matrix4().fromArray(parent3D(PM,layer,PM.time)).invert());
  return (['x','y','z'] as const).map(axis=>propertyCommand(PM,id,`position.${axis}`,next[axis]));
}
/** Replace the composition's lights with a preset rig aimed at the composition centre. */
export function applyLightingPreset(PM:any,presetId:string):string[] {
  const preset=LIGHTING_PRESETS.find(p=>p.id===presetId);if(!preset)throw new Error(`Unknown lighting preset "${presetId}"`);
  const lights=comp(PM).layers.filter((l:any)=>layer3DRole(l)==='light');
  if(lights.some((l:any)=>!editableLayer(PM,l)))throw new Error('Unlock the lights first');
  const center=compCenter(comp(PM)),distance=6*PX_PER_UNIT,commands:any[]=[],ids:string[]=[];
  if(lights.length)commands.push({type:'delete_layers',targets:lights.map((l:any)=>l.id)});
  for(const light of preset.lights){
    const at=new THREE.Vector3(center.x,center.y,center.z).addScaledVector(directionFrom(light.azimuth,light.elevation),distance);
    const plan=sceneCommands(PM,{operation:'add_light',light:{name:light.name,type:light.type,p:{x:at.x,y:at.y,z:at.z,targetX:center.x,targetY:center.y,targetZ:center.z,color:light.color,intensity:light.intensity}}});
    for(const command of plan.commands)if(command.type==='add_layer')(command as any).select=false;
    commands.push(...plan.commands);if(plan.id)ids.push(plan.id);
  }
  const result=PM.Edit.apply(commands,{origin:'inspector',label:`${preset.label} lighting`});
  if(!result.ok)throw new Error(result.message);
  return ids;
}
