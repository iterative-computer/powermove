import { afterEach, describe, expect, it } from 'vitest';
import * as THREE from 'three';
import { makePM } from '../../legacy/__tests__/make-pm';
import { LAYER3D_DEFINITIONS, compositionScene } from './layers';
import { editScene } from './operations';
import { compositionRuntime, disposeSceneRuntimes } from './service';
import { lightPositionFromViewer, lightViewerState } from './light-view';
import { makeScene3DAPI } from './api';
import { applyViewportPose } from './viewport';

const editors:any[]=[];
const rect={x:100,y:50,width:800,height:450};
function editor() {
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject({name:'Lights',w:1920,h:1080,dur:5});PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)
    ? {id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}} : null;
  expect(editScene(PM,{operation:'add_light',light:{id:'lamp',type:'point',p:{x:0,y:0,z:0}}}).ok).toBe(true);
  editors.push(PM);return PM;
}
afterEach(()=>{for(const PM of editors.splice(0))disposeSceneRuntimes(PM);});

describe('composition camera defaults',()=>{
  it('centers new compositions and gives an added default camera exactly the same pose',()=>{
    const PM=editor(),implicit=compositionRuntime(PM).camera.clone();
    expect(implicit.position.toArray()).toEqual([0,0,6]);
    expect(implicit.getWorldDirection(new THREE.Vector3()).distanceTo(new THREE.Vector3(0,0,-1))).toBeLessThan(1e-10);
    expect(editScene(PM,{operation:'add_camera',id:'lens'}).ok).toBe(true);
    const explicit=compositionRuntime(PM).camera;
    expect(explicit.matrixWorld.elements).toEqual(implicit.matrixWorld.elements);
    expect(explicit.projectionMatrix.elements).toEqual(implicit.projectionMatrix.elements);
    PM.hist.undo();expect(compositionRuntime(PM).camera.matrixWorld.elements).toEqual(implicit.matrixWorld.elements);
  });

  it('preserves saved unversioned camera defaults and explicit camera transforms on roundtrip',()=>{
    const PM=editor();delete PM.proj.scene3dDefaultsVersion;PM.touch();
    const old=compositionRuntime(PM).camera.matrixWorld.elements.slice();
    expect(compositionRuntime(PM).camera.position.toArray()).toEqual([3,2,6]);
    PM.proj=PM.deserialize(PM.serialize());PM.touch();
    expect(compositionRuntime(PM).camera.matrixWorld.elements).toEqual(old);
    expect(editScene(PM,{operation:'add_camera',id:'lens',camera:{p:{x:7,y:3,z:9,targetX:2}}}).ok).toBe(true);
    const explicit=compositionRuntime(PM).camera.matrixWorld.elements.slice();
    PM.proj=PM.deserialize(PM.serialize());PM.touch();
    expect(compositionRuntime(PM).camera.matrixWorld.elements).toEqual(explicit);
  });

  it('keeps camera versions per composition across switching, precompose, undo and save',()=>{
    const PM=editor(),root=PM.proj.compId;
    delete PM.proj.scene3dDefaultsVersion;
    const fresh=PM.Comps.create({name:'New comp'});
    expect(PM.proj.scene3dDefaultsVersion).toBe(2);
    PM.Comps.open(root);expect(PM.proj.scene3dDefaultsVersion).toBeUndefined();
    expect(compositionScene(PM).camera.p.x!.v).toBe(3);
    const nested=PM.Comps.precompose(['lamp'],{name:'Moved light'});
    expect(PM.proj.comps[nested].scene3dDefaultsVersion).toBe(1);
    PM.hist.undo();expect(PM.L('lamp')).toBeTruthy();
    PM.Comps.open(fresh);expect(PM.proj.scene3dDefaultsVersion).toBe(2);
    PM.proj=PM.deserialize(PM.serialize());
    expect(compositionScene(PM).camera.p.x!.v).toBe(0);
    PM.Comps.open(root);expect(PM.proj.scene3dDefaultsVersion).toBeUndefined();
  });
});

describe('light viewer data and drag conversion',()=>{
  it('projects zero Position to comp center with the same world position as a model',()=>{
    const PM=editor(),api=makeScene3DAPI(PM,()=>{});
    editScene(PM,{operation:'add_object',object:{id:'model',source:{primitive:'box'}}});
    const state=api.lightViewerState('lamp',0,rect)!;
    expect(state).toMatchObject({worldPosition:{x:0,y:0,z:0},viewerPosition:{x:500,y:275},visible:true,inFront:true,inFrame:true,type:'point',color:'#FFFFFF',intensity:3,direction:null,coneAngleDegrees:null});
    expect(compositionRuntime(PM).objects.get('model')!.getWorldPosition(new THREE.Vector3()).toArray()).toEqual(Object.values(state.worldPosition));
    expect(api.lightPositionFromViewer('lamp',0,state.viewerPosition!,rect)).toEqual({x:0,y:0,z:0});
  });

  it.each(['perspective','orthographic'] as const)('round-trips drags through an explicit %s camera, with animated channels and transformed parents',projection=>{
    const PM=editor();
    editScene(PM,{operation:'add_camera',id:'lens',camera:{projection,p:{x:4,y:2,z:8,targetX:1}}});
    editScene(PM,{operation:'add_object',object:{id:'parent',source:{primitive:'box'},p:{x:.3}}});
    const changed=PM.Edit.apply([{type:'set_layer',target:'lamp',patch:{parent:'parent'}},
      {type:'set_property',target:'lamp',path:'anchor.x',value:.2},
      {type:'replace_keyframes',target:'lamp',path:'position.x',keyframes:[{time:0,value:0},{time:2,value:1}]}]);expect(changed.ok,changed.message).toBe(true);
    expect(editScene(PM,{operation:'update_object',target:'parent',patch:{p:{ry:35,sx:2,sy:1.5}}}).ok).toBe(true);
    const time=1,state=lightViewerState(PM,'lamp',time,rect)!;
    const original=lightPositionFromViewer(PM,'lamp',time,state.viewerPosition!,rect)!;
    expect(original.x).toBeCloseTo(.5);expect(original.y).toBeCloseTo(0);expect(original.z).toBeCloseTo(0);
    const pointer={x:state.viewerPosition!.x+95,y:state.viewerPosition!.y-35};
    const next=lightPositionFromViewer(PM,'lamp',time,pointer,rect)!;
    expect(editScene(PM,{operation:'update_light',target:'lamp',patch:{p:next}}).ok).toBe(true);
    // Replace the animated x value for this assertion at its evaluated time.
    expect(PM.Edit.apply({type:'set_property',target:'lamp',path:'position.x',value:next.x,time,mode:'keyframe'}).ok).toBe(true);
    const moved=lightViewerState(PM,'lamp',time,rect)!;
    expect(moved.viewerPosition!.x).toBeCloseTo(pointer.x);expect(moved.viewerPosition!.y).toBeCloseTo(pointer.y);
    const camera=compositionRuntime(PM,time).camera,normal=camera.getWorldDirection(new THREE.Vector3());
    const delta=new THREE.Vector3(moved.worldPosition.x-state.worldPosition.x,moved.worldPosition.y-state.worldPosition.y,moved.worldPosition.z-state.worldPosition.z);
    expect(delta.dot(normal)).toBeCloseTo(0);
  });

  it('evaluates spot appearance, direction and cone through grouped animated transforms',()=>{
    const PM=editor();
    editScene(PM,{operation:'update_light',target:'lamp',patch:{type:'spot',p:{z:2,color:'#3366FF',angle:30,intensity:12}}});
    const group=PM.groupLayers(['lamp']);
    expect(PM.Edit.apply([{type:'set_property',target:group.id,path:'rotation.y',value:90},
      {type:'replace_keyframes',target:'lamp',path:'light.intensity',keyframes:[{time:0,value:10},{time:2,value:20}]},
      {type:'replace_keyframes',target:'lamp',path:'light.angle',keyframes:[{time:0,value:20},{time:2,value:40}]}]).ok).toBe(true);
    const state=lightViewerState(PM,'lamp',1,rect)!;
    expect(state).toMatchObject({color:'#3366FF',intensity:15,type:'spot',coneAngleDegrees:30});
    expect(state.direction!.x).toBeCloseTo(-1);expect(state.direction!.y).toBeCloseTo(0);expect(state.direction!.z).toBeCloseTo(0);
    editScene(PM,{operation:'update_light',target:'lamp',patch:{type:'sun'}});
    expect(lightViewerState(PM,'lamp',1,rect)).toMatchObject({type:'sun',coneAngleDegrees:null});
  });

  it('follows the Editor camera without changing render-camera data or the project',()=>{
    const PM=editor(),saved=PM.serialize();
    applyViewportPose(PM,{position:new THREE.Vector3(4,2,6),target:new THREE.Vector3(1,0,0),zoom:1});
    const viewer=lightViewerState(PM,'lamp',0,rect)!,shot=lightViewerState(PM,'lamp',0,rect,'camera')!;
    expect(viewer.viewerPosition).not.toEqual(shot.viewerPosition);expect(shot.viewerPosition).toEqual({x:500,y:275});
    const original=lightPositionFromViewer(PM,'lamp',0,viewer.viewerPosition!,rect)!;
    expect(original.x).toBeCloseTo(0);expect(original.y).toBeCloseTo(0);expect(original.z).toBeCloseTo(0);
    expect(PM.serialize()).toBe(saved);
  });

  it('reports inactive, behind-camera, clipped and coincident lights and refuses invalid drags',()=>{
    const PM=editor();
    editScene(PM,{operation:'update_light',target:'lamp',patch:{p:{z:8}}});
    expect(lightViewerState(PM,'lamp',0,rect)).toMatchObject({inFront:false,visible:false,inFrame:false});
    expect(lightPositionFromViewer(PM,'lamp',0,{x:500,y:275},rect)).toBeNull();
    editScene(PM,{operation:'update_light',target:'lamp',patch:{p:{z:6}}});
    expect(lightViewerState(PM,'lamp',0,rect)!.viewerPosition).toBeNull();
    editScene(PM,{operation:'update_light',target:'lamp',patch:{type:'spot',p:{x:100,z:0}}});
    expect(lightViewerState(PM,'lamp',0,rect)).toMatchObject({inFront:true,visible:true,inFrame:false});
    editScene(PM,{operation:'update_light',target:'lamp',patch:{p:{x:0,visible:false}}});
    expect(lightViewerState(PM,'lamp',0,rect)).toMatchObject({visible:false,direction:null});
    expect(lightViewerState(PM,'missing',0,rect)).toBeNull();
    expect(lightViewerState(PM,'lamp',0,{...rect,width:0})).toBeNull();
    expect(lightPositionFromViewer(PM,'lamp',0,{x:NaN,y:275},rect)).toBeNull();
    editScene(PM,{operation:'add_object',object:{id:'parent',source:{primitive:'box'},p:{}}});
    const reparented=PM.Edit.apply({type:'set_layer',target:'lamp',patch:{parent:'parent'}});expect(reparented.ok,reparented.message).toBe(true);
    editScene(PM,{operation:'update_object',target:'parent',patch:{p:{sx:0}}});
    expect(lightPositionFromViewer(PM,'lamp',0,{x:500,y:275},rect)).toBeNull();
  });
});
