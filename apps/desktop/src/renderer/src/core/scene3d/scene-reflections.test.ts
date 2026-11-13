import {describe,expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {reflectionPlane,reflectedCamera,SceneReflections} from './scene-reflections';

describe('scene reflections',()=>{
  it('recognizes imported flat geometry after rotation and scale, while retaining curved geometry',()=>{
    const plane=new THREE.Mesh(new THREE.PlaneGeometry(8,8));plane.rotation.x=-Math.PI/2;plane.position.y=2;plane.scale.set(2,3,1);plane.updateMatrixWorld();
    const surface=reflectionPlane(plane,new THREE.Vector3(0,4,0))!;
    expect(surface.normal.distanceTo(new THREE.Vector3(0,1,0))).toBeLessThan(1e-8);
    expect(surface.distanceToPoint(new THREE.Vector3(3,2,5))).toBeCloseTo(0);
    expect(reflectionPlane(new THREE.Mesh(new THREE.SphereGeometry()),new THREE.Vector3(0,4,0))).toBeNull();
  });
  it.each(['perspective','orthographic'])('mirrors and clips a %s camera without changing the shot',type=>{
    const camera=type==='perspective'?new THREE.PerspectiveCamera(50,1,.01,100):new THREE.OrthographicCamera(-5,5,5,-5,.01,100);
    camera.position.set(3,4,5);camera.lookAt(0,0,0);camera.updateMatrixWorld();
    camera.matrixAutoUpdate=false;
    const before=camera.matrixWorld.toArray(),projection=camera.projectionMatrix.toArray();
    const mirror=reflectedCamera(camera,new THREE.Plane(new THREE.Vector3(0,1,0),0));
    expect(mirror.camera.position.toArray()).toEqual([3,-4,5]);
    expect(new THREE.Vector3(0,1,0).project(mirror.camera).z).toBeGreaterThan(-1);
    expect(new THREE.Vector3(0,-1,0).project(mirror.camera).z).toBeLessThan(-1);
    expect(camera.matrixWorld.toArray()).toEqual(before);expect(camera.projectionMatrix.toArray()).toEqual(projection);
  });
  it.each([true,false])('caches reflections across repeated draws and restores state after failures (flat=%s)',flat=>{
    const scene=new THREE.Scene(),floor=new THREE.Mesh(flat?new THREE.PlaneGeometry(8,8):new THREE.SphereGeometry(),new THREE.MeshStandardMaterial({roughness:.1}));
    if(flat)floor.rotation.x=-Math.PI/2;floor.userData.sceneId='floor';
    const box=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial({roughness:1}));box.position.y=1;box.userData.sceneId='box';box.userData.layerOpacity=.4;
    scene.add(floor,box);const objects=new Map([['floor',floor],['box',box]]);
    const camera=new THREE.PerspectiveCamera(50,1,.01,100);camera.position.set(3,4,5);camera.lookAt(0,0,0);camera.updateMatrixWorld();
    const viewport=new THREE.Vector4(4,5,300,200),clear=new THREE.Color('#123456');
    let target:THREE.WebGLRenderTarget|null=null;
    const renderer:any={coordinateSystem:THREE.WebGLCoordinateSystem,capabilities:{maxCubemapSize:2048,maxSamples:4},autoClear:false,xr:{enabled:true},shadowMap:{enabled:false,autoUpdate:true},
      getRenderTarget:()=>target,getActiveCubeFace:()=>0,getActiveMipmapLevel:()=>0,setRenderTarget:(t:any)=>{target=t;},
      getViewport:(out:any)=>out.copy(viewport),getScissor:(out:any)=>out.copy(viewport),getScissorTest:()=>true,
      setViewport:vi.fn(),setScissor:vi.fn(),setScissorTest:vi.fn(),getClearColor:(out:any)=>out.copy(clear),getClearAlpha:()=>.3,setClearColor:vi.fn(),
      state:{buffers:{depth:{setMask:vi.fn()}}},clear:vi.fn(),render:vi.fn()};
    const reflections=new SceneReflections();
    reflections.update(renderer,scene,objects,camera,1,300,300);expect(renderer.render).toHaveBeenCalledTimes(flat?1:6);
    reflections.update(renderer,scene,objects,camera,1,300,300);expect(renderer.render).toHaveBeenCalledTimes(flat?1:6);
    const materialHook=(floor.material as THREE.Material).onBeforeCompile;
    reflections.update(renderer,scene,objects,camera,1,300,300,new Set(['floor']));
    reflections.update(renderer,scene,objects,camera,1,300,300,new Set(['floor','box']));
    expect((floor.material as THREE.Material).onBeforeCompile).toBe(materialHook);
    expect(renderer.render).toHaveBeenCalledTimes(flat?2:12);
    renderer.render.mockImplementationOnce(()=>{throw new Error('capture failed');});
    expect(()=>reflections.update(renderer,scene,objects,camera,2,300,300)).toThrow('capture failed');
    expect(floor.visible).toBe(true);expect(box.visible).toBe(true);expect(target).toBeNull();
    expect(box.material.opacity).toBe(1);expect(box.material.transparent).toBe(false);expect(box.material.depthWrite).toBe(true);
    expect(renderer.autoClear).toBe(false);expect(renderer.xr.enabled).toBe(true);expect(renderer.shadowMap.autoUpdate).toBe(true);
    expect(renderer.setViewport).toHaveBeenLastCalledWith(viewport);expect(renderer.setClearColor).toHaveBeenLastCalledWith(clear,.3);
    reflections.update(renderer,scene,objects,camera,3,300,300);
    expect(renderer.render).toHaveBeenCalledTimes(flat?4:19);
    reflections.dispose();
  });
});
