import {afterEach,expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {clearStudioGeometryCaches,extractScene,isPending,studioExtractionIdle} from './extract';
afterEach(()=>{vi.restoreAllMocks();clearStudioGeometryCaches();});
it('incremental extraction retains every transform, triangle and visibility policy',async()=>{
 const scene=new THREE.Scene(),geometry=new THREE.BoxGeometry(),material=new THREE.MeshStandardMaterial(),mesh=new THREE.InstancedMesh(geometry,material,128);
 mesh.userData.sceneId='trees';scene.add(mesh);
 for(let i=0;i<mesh.count;i++)mesh.setMatrixAt(i,new THREE.Matrix4().makeTranslation(i%16,0,Math.floor(i/16)));mesh.instanceMatrix.needsUpdate=true;
 const environment={sky:new THREE.Color(),ground:new THREE.Color(),intensity:0},hiddenPrimary=new Set(['trees']);
 const expected=extractScene(scene,()=>true,environment,{synchronous:true,hiddenPrimary,onReady:vi.fn()});expect(isPending(expected)).toBe(false);
 let clock=0;vi.spyOn(performance,'now').mockImplementation(()=>clock+=8);
 const onReady=vi.fn(),options={hiddenPrimary,onReady};expect(isPending(extractScene(scene,()=>true,environment,options))).toBe(true);
 await studioExtractionIdle();const actual=extractScene(scene,()=>true,environment,options);
 expect(isPending(actual)).toBe(false);if(isPending(actual)||isPending(expected))throw new Error('Extraction did not finish');
 for(const field of ['positions','tris','blasNodes','tlasNodes','sdata','materials','instancePolicies'] as const)expect(actual[field]).toEqual(expected[field]);
 expect(actual.instanceCount).toBe(128);expect(actual.instancePolicies!.every(p=>!p.primaryVisible)).toBe(true);expect(onReady).toHaveBeenCalledOnce();
 geometry.dispose();material.dispose();
});
it('reuses unchanged graphs and invalidates material and transform edits',async()=>{
 const scene=new THREE.Scene(),geometry=new THREE.BoxGeometry(),material=new THREE.MeshStandardMaterial(),mesh=new THREE.Mesh(geometry,material);mesh.userData.sceneId='model';scene.add(mesh);
 const environment={sky:new THREE.Color(),ground:new THREE.Color(),intensity:0},options={onReady:vi.fn()};
 const read=async()=>{extractScene(scene,()=>true,environment,options);await studioExtractionIdle();return extractScene(scene,()=>true,environment,options);};
 const first=await read();expect(await read()).toBe(first);
 material.color.setRGB(.2,.3,.4);const recolored=await read();expect(recolored).not.toBe(first);
 mesh.position.x=2;const moved=await read();expect(moved).not.toBe(recolored);
 if(isPending(first)||isPending(recolored)||isPending(moved))throw new Error('Extraction did not finish');
 expect(recolored.positions).toBe(first.positions);expect(moved.positions).toBe(first.positions);expect(moved.sdata).not.toEqual(recolored.sdata);
 geometry.dispose();material.dispose();
});
