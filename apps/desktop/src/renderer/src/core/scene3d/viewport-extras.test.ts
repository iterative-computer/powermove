import {describe,expect,it} from 'vitest';
import * as THREE from 'three';
import {walkStep} from './walk';
import {rollView,viewportPose,viewLabel,setViewAxis,localViewIds,applyViewportPose,viewPosition} from './viewport';
import {runViewportOperator} from './commands';
import {selectAll} from './operators';
import {setViewportPreferences,viewportPreferences} from './editor-state';
import {compositionRuntime} from './service';
import {editScene} from './operations';
import {LAYER3D_DEFINITIONS} from './layers';
import {parseScene} from './schema';
import {makePM} from '../../legacy/__tests__/make-pm';

function editor(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject();PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  return PM;
}

describe('Blender viewport extras',()=>{
  it('walks along the ground, strafes, rises and never pitches past vertical',()=>{
    const pose={target:new THREE.Vector3(0,0,0),rotation:new THREE.Quaternion(),distance:5},position=new THREE.Vector3(0,0,5);
    let next=walkStep(pose,position,new THREE.Vector3(0,0,1),{x:0,y:0});
    expect(next.position.toArray().map(v=>Number(v.toFixed(6)))).toEqual([0,0,4]);
    next=walkStep(next.pose,next.position,new THREE.Vector3(1,1,0),{x:0,y:0});
    expect(next.position.x).toBeCloseTo(1);expect(next.position.y).toBeCloseTo(1);
    const looked=walkStep(next.pose,next.position,new THREE.Vector3(),{x:0,y:-100000});
    expect(Math.abs(new THREE.Vector3(0,0,-1).applyQuaternion(looked.pose.rotation).y)).toBeLessThan(.996);
    expect(looked.pose.target.distanceTo(looked.position)).toBeCloseTo(5);
  });
  it('rolls the view about its own axis, keeping a named view',()=>{
    const PM=editor();expect(editScene(PM,{operation:'add_object',object:{id:'cube',source:{primitive:'box'}}}).ok).toBe(true);
    setViewAxis(PM,'front');const before=viewportPose(PM);rollView(PM,15);const after=viewportPose(PM);
    expect(viewPosition(after).distanceTo(viewPosition(before))).toBeCloseTo(0);
    expect(new THREE.Vector3(0,1,0).applyQuaternion(after.rotation).x).toBeCloseTo(-Math.sin(Math.PI/12));
    expect(viewLabel(PM)).toBe('Front Orthographic');
  });
  it('Local View shows only the chosen models in the editor and restores the previous view',()=>{
    const PM=editor();
    for(const [id,x] of [['a',-3],['b',3]] as const)expect(editScene(PM,{operation:'add_object',object:{id,source:{primitive:'box'},p:{x}}}).ok).toBe(true);
    applyViewportPose(PM,{target:new THREE.Vector3(0,0,0),rotation:new THREE.Quaternion(),distance:20});
    const snapshot=JSON.stringify(PM.proj.layers);PM.selectLayers(['b']);
    expect(runViewportOperator(PM,'view.local')).toBe(true);
    expect([...localViewIds(PM)!]).toEqual(['b']);expect(viewLabel(PM)).toBe('User Perspective (Local)');
    expect(viewportPose(PM).target.x).toBeCloseTo(3);
    PM.selectLayers([]);selectAll(PM,'select');expect(PM.sel.layers).toEqual(['b']);
    expect(runViewportOperator(PM,'view.local')).toBe(true);expect(localViewIds(PM)).toBeNull();
    expect(viewportPose(PM).distance).toBeCloseTo(20);expect(JSON.stringify(PM.proj.layers)).toBe(snapshot);
    PM.selectLayers([]);expect(runViewportOperator(PM,'view.local')).toBe(false);
  });
  it('adds rectangular area lights that face their target in the native preview',()=>{
    const PM=editor();
    expect(editScene(PM,{operation:'add_light',light:{id:'panel',type:'area',p:{x:0,y:4,z:0,width:2,height:.5,targetX:0,targetY:0,targetZ:0}}}).ok).toBe(true);
    const light=compositionRuntime(PM).objects.get(PM.proj.layers[0].id) as THREE.RectAreaLight;
    expect(light.isRectAreaLight).toBe(true);expect([light.width,light.height]).toEqual([2,.5]);
    expect(new THREE.Vector3(0,0,-1).applyQuaternion(light.getWorldQuaternion(new THREE.Quaternion())).y).toBeCloseTo(-1);
    expect(()=>parseScene({lights:[{id:'bad',type:'area',p:{width:-1}}]})).toThrow();
  });
  it('toggles X-ray as an editor preference',()=>{
    const PM=editor();const before=viewportPreferences(PM).xray;
    expect(runViewportOperator(PM,'xray.toggle')).toBe(true);expect(viewportPreferences(PM).xray).toBe(!before);
    setViewportPreferences(PM,{xray:false});
  });
});

describe('editor looks never reach output',()=>{
  it('keeps the shared depth prepass separate for X-ray',()=>{
    const PM=editor();
    for(const [id,z] of [['front',1],['back',-1]] as const)expect(editScene(PM,{operation:'add_object',object:{id,source:{primitive:'box'},p:{z}}}).ok).toBe(true);
    const runtime=compositionRuntime(PM) as any,keys:string[]=[];
    const original=runtime.renderPass.bind(runtime);
    runtime.renderPass=(...args:any[])=>{keys.push(args[6]);return original(...args);};
    try{runtime.render(64,36,'front',undefined,true,runtime.camera,{xray:true});}catch{/* no WebGL in unit tests */}
    try{runtime.render(64,36,'front',undefined,true,runtime.camera);}catch{/* no WebGL in unit tests */}
    expect(keys).toEqual(['false:true','false:false']);
  });
});
