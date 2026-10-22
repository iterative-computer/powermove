import {describe,expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {applyViewportPose,onViewportChange,setViewportMode,viewportCamera,viewportEngineCamera,viewportPose} from './viewport';
import {compositionRuntime} from './service';
import {editScene} from './operations';
import {makePM} from '../../legacy/__tests__/make-pm';
import {LAYER3D_DEFINITIONS} from './layers';

function scenePM(){
  const PM=makePM('core/easing','core/model','core/selection','core/anim','core/history','core/editing');
  PM.proj=PM.mkProject();PM.time=0;
  PM.layerDefinition=(id:string)=>Object.values(LAYER3D_DEFINITIONS).includes(id as any)?{id,label:'3D',version:1,params:[],defaults:{},renderer:{kind:'layer3d'}}:null;
  expect(editScene(PM,{operation:'add_object',object:{id:'a',source:{primitive:'box'}}}).ok).toBe(true);
  return PM;
}

/* The native engine's view override takes the Editor view's camera
   (extensions/viewer/engine-present/view-params.ts). */
describe('3D editor camera for the native engine',()=>{
  it('is null in the Camera view and the editor camera, world to camera, in the Editor view',()=>{
    const PM=scenePM();
    expect(viewportEngineCamera(PM)).toBeNull();
    applyViewportPose(PM,{position:new THREE.Vector3(2,3,9),target:new THREE.Vector3(1,0,0),zoom:1});
    const engine=viewportEngineCamera(PM)!,camera=viewportCamera(PM,compositionRuntime(PM).camera);
    expect(engine.view).toHaveLength(16);
    // The target lies straight ahead of the camera (-z in camera space).
    const target=new THREE.Vector3(1,0,0).applyMatrix4(new THREE.Matrix4().fromArray(engine.view));
    expect(target.x).toBeCloseTo(0);expect(target.y).toBeCloseTo(0);
    expect(target.z).toBeCloseTo(-new THREE.Vector3(2,3,9).distanceTo(new THREE.Vector3(1,0,0)));
    expect(engine.projection.kind).toBe(camera instanceof THREE.OrthographicCamera?'orthographic':'perspective');
    setViewportMode(PM,'camera');
    expect(viewportEngineCamera(PM)).toBeNull();
  });

  it('folds three.js zoom into the vertical field of view', ()=>{
    const PM=scenePM();
    const pose=viewportPose(PM);
    applyViewportPose(PM,{...pose,zoom:1});
    const camera=viewportCamera(PM,compositionRuntime(PM).camera) as THREE.PerspectiveCamera;
    expect((camera as any).isPerspectiveCamera).toBe(true);
    const one=viewportEngineCamera(PM)!.projection as {kind:'perspective';fovYDegrees:number;near:number;far:number};
    expect(one.fovYDegrees).toBeCloseTo(camera.fov);
    expect(one).toMatchObject({near:camera.near,far:camera.far});
    applyViewportPose(PM,{...pose,zoom:2});
    const two=viewportEngineCamera(PM)!.projection as {fovYDegrees:number};
    expect(two.fovYDegrees).toBeCloseTo(THREE.MathUtils.radToDeg(2*Math.atan(Math.tan(THREE.MathUtils.degToRad(camera.fov)/2)/2)));
  });

  it('tells view listeners about each pose change, not only mode changes',()=>{
    const PM=scenePM(),listener=vi.fn();
    const off=onViewportChange(PM,listener);
    const pose=viewportPose(PM);
    applyViewportPose(PM,pose);
    applyViewportPose(PM,{...pose,position:pose.position.clone().add(new THREE.Vector3(1,0,0))});
    expect(listener).toHaveBeenCalledTimes(2);
    expect(listener).toHaveBeenLastCalledWith('editor');
    off();
  });
});
