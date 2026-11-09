import { expect, it, vi } from 'vitest';
import * as THREE from 'three';
import { studioTargetBytes, studioSceneBytes, STUDIO_SESSION_BYTES } from './resource-budget';
import { extractScene, clearStudioGeometryCaches } from './extract';

it('bounds target dimensions, aggregate allocations and adapter storage bindings',()=>{
  expect(studioTargetBytes(1920,1080)).toBeLessThan(STUDIO_SESSION_BYTES);
  for(const size of [[8192,8192],[4096,2160],[0,100],[NaN,10]])expect(()=>studioTargetBytes(size[0]!,size[1]!)).toThrow(/budget|dimensions/);
  expect(()=>studioTargetBytes(1920,1080,32*1024**2)).toThrow('buffer budget');
});
it('rejects an oversized imported mesh before allocating its BVH and keeps a healthy layer',()=>{
  const warning=vi.spyOn(console,'warn').mockImplementation(()=>{});
  const graph=new THREE.Scene(),healthy=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial());
  healthy.userData.sceneId='healthy';graph.add(healthy);
  const geometry=new THREE.BufferGeometry();geometry.setAttribute('position',new THREE.Float32BufferAttribute([0,0,0,1,0,0,0,1,0],3));
  geometry.setIndex(new THREE.BufferAttribute(new Uint32Array(1_800_000*3),1));
  const oversized=new THREE.Mesh(geometry,new THREE.MeshStandardMaterial());oversized.userData.sceneId='oversized';graph.add(oversized);
  try {
    const scene=extractScene(graph,()=>true,{sky:new THREE.Color(),ground:new THREE.Color(),intensity:0});
    expect(scene.triangles).toBe(12);expect(scene.instanceCount).toBe(1);
    expect(warning).toHaveBeenCalledWith(expect.stringContaining('memory budget'));
    expect(studioSceneBytes(scene,0)).toBeLessThan(1024**2);
  } finally {warning.mockRestore();geometry.dispose();healthy.geometry.dispose();clearStudioGeometryCaches();}
});
