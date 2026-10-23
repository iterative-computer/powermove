import { expect, it, vi } from 'vitest';
import * as THREE from 'three';
import { object3dFromEngine } from './compat-object3d';
import { disposeModel } from './import-model';

it('keeps engine hierarchy, mesh buffers, materials, animation and disposal usable', async () => {
  const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  const data = {
    objects: [
      {kind:'Group',name:'Scene',parent:null,matrix:identity},
      {kind:'Group',name:'Armature',parent:0,matrix:identity,node:0},
      {kind:'Mesh',name:'Part',parent:1,matrix:identity,primitive:[0,0],node:1},
    ],
    nodeObjects:[1,2],
    meshes:[{weights:[],primitives:[{topology:'Triangles',positions:[[0,0,0],[1,0,0],[0,1,0]],normals:[[0,0,1],[0,0,1],[0,0,1]],uv:[[0,0],[1,0],[0,1]],indices:[0,1,2],material:0,targets:[]}]}],
    materials:[{name:'Red',baseColor:[1,0,0,1],metallic:0,roughness:.5,alphaMode:'OPAQUE',doubleSided:true,emissive:[0,0,0],extensions:{}}],
    textures:[],images:[],skins:[],instances:[],lights:[],cameras:[],
    clips:[{name:'Move',duration:2,channels:[{node:1,path:'translation',interpolation:'LINEAR',times:[0,2],values:[0,0,0,2,0,0],size:3}]}],
    nodeMeshes:[null,0],
    metadata:{scene:{extras:{sceneTag:'portable'},extensions:{EXT_vendor_scene:{value:1}}},
      nodes:[{name:'Armature Source',extras:{nodeTag:'joint'}},{name:'Part Source',extras:{nodeTag:'mesh'}}],
      meshes:[{meta:{extras:{meshTag:'part'}},primitives:[{extras:{geometryTag:'triangle'},extensions:{EXT_vendor_primitive:{value:2}}}]}],
      materials:[{extras:{materialTag:'red'},extensions:{EXT_vendor_material:{value:3},KHR_materials_unlit:{}}}],
      animations:[{extras:{clipTag:'move'}}]},
    facts:{vertices:3,triangles:1,center:[.5,.5,0],radius:1},
  };
  const imported = await object3dFromEngine(data);
  const mesh = imported.object3d.getObjectByName('Part') as THREE.Mesh;
  expect(mesh).toBeInstanceOf(THREE.Mesh);
  expect(mesh.geometry.getAttribute('position').count).toBe(3);
  expect(mesh.material).toBeInstanceOf(THREE.MeshStandardMaterial);
  expect((mesh.material as THREE.MeshStandardMaterial).color.getHex()).toBe(0xff0000);
  expect(mesh.parent?.name).toBe('Armature');
  expect(imported.object3d.children[0]!.children[0]!.userData).toMatchObject({sceneTag:'portable',gltfExtensions:{EXT_vendor_scene:{value:1}}});
  expect(mesh.userData).toMatchObject({name:'Part Source',meshTag:'part',nodeTag:'mesh',gltfExtensions:{EXT_vendor_primitive:{value:2}}});
  expect(mesh.geometry.userData).toMatchObject({geometryTag:'triangle'});
  expect((mesh.material as THREE.Material).userData).toMatchObject({materialTag:'red',gltfExtensions:{EXT_vendor_material:{value:3}}});
  expect(imported.animations[0]!.userData).toMatchObject({clipTag:'move'});
  const clone = imported.object3d.clone(true);
  const mixer = new THREE.AnimationMixer(clone);
  mixer.clipAction(imported.animations[0]!).play();
  mixer.setTime(1);
  expect(clone.getObjectByName('Part')?.position.x).toBeCloseTo(1);
  const disposed = vi.spyOn(mesh.geometry, 'dispose');
  disposeModel(imported.object3d);
  expect(disposed).toHaveBeenCalledOnce();
});

it('plays engine cubic-spline clips with glTF tangents after cloning', async () => {
  const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  const data = {
    objects:[{kind:'Group',name:'Scene',parent:null,matrix:identity},{kind:'Mesh',name:'Animated',parent:0,matrix:identity,node:0,primitive:[0,0]}],
    nodeObjects:[1],meshes:[{weights:[],primitives:[{topology:'Triangles',positions:[[0,0,0],[1,0,0],[0,1,0]],indices:[0,1,2],material:null,targets:[]}]}],materials:[],images:[],textures:[],skins:[],instances:[],lights:[],cameras:[],
    clips:[{name:'Spline',duration:1,channels:[{
      node:0,path:'translation',interpolation:'CUBICSPLINE',times:[0,1],size:3,
      values:[0,0,0, 0,0,0, 2,0,0, 0,0,0, 1,0,0, 0,0,0]
    }]}],facts:{vertices:3,triangles:1,center:[.5,.5,0],radius:1}
  };
  const result = await object3dFromEngine(data);
  expect((result.animations[0]!.tracks[0]! as any).createInterpolant().evaluate(.5)[0]).toBeCloseTo(.75);
  const clone = result.object3d.clone(true);
  const mixer = new THREE.AnimationMixer(clone);
  mixer.clipAction(result.animations[0]!).play();
  mixer.setTime(.5);
  expect(clone.getObjectByName('Animated')?.position.x).toBeCloseTo(.75);
});

it('shares packed float buffers and preserves narrowed colors, joints, indices and morph zeros', async () => {
  const identity = [1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1];
  const storage = new ArrayBuffer(9 * 4 + 9 * 4 + 12 * 4);
  const positions = new Float32Array(storage, 0, 9);
  positions.set([0, 0, 0, 1, 0, 0, 0, 1, 0]);
  const morph = new Float32Array(storage, 9 * 4, 9);
  morph.set([0, 0, .25, 0, 0, 0, 0, 0, 0]);
  const colors = new Float32Array(storage, 18 * 4, 12);
  colors.set([1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 1]);
  const data = {
    objects: [{kind:'Group',name:'Scene',parent:null,matrix:identity},
      {kind:'Mesh',name:'Packed',parent:0,matrix:identity,primitive:[0,0]}],
    nodeObjects:[], meshes:[{weights:[],primitives:[{
      topology:'Triangles',positions,colors,colorComponents:3,
      joints:new Uint32Array([1,2,3,4, 4,3,2,1, 65535,0,0,0]),
      indices:new Uint32Array([0,1,2]), material:null,
      targets:[{position:morph,normal:null},{position:null,normal:new Float32Array([0,0,1, 0,0,1, 0,0,1])}],
    }]}],
    materials:[],images:[],textures:[],skins:[],instances:[],lights:[],cameras:[],clips:[],
    facts:{vertices:3,triangles:1,center:[.5,.5,0],radius:1},
  };
  const imported = await object3dFromEngine(data);
  const geometry = (imported.object3d.getObjectByName('Packed') as THREE.Mesh).geometry;
  expect(geometry.getAttribute('position').array).toBe(positions);
  expect(geometry.morphAttributes.position![0]!.array).toBe(morph);
  expect(Array.from(geometry.morphAttributes.position![1]!.array)).toEqual(Array(9).fill(0));
  expect(Array.from(geometry.getAttribute('color').array)).toEqual([1,0,0, 0,1,0, 0,0,1]);
  expect(geometry.getAttribute('skinIndex').array).toBeInstanceOf(Uint16Array);
  expect(Array.from(geometry.getAttribute('skinIndex').array).slice(8, 12)).toEqual([65535,0,0,0]);
  expect(geometry.index!.array).toBeInstanceOf(Uint16Array);
  expect(Array.from(geometry.index!.array)).toEqual([0,1,2]);
  const wideIndices = new Uint32Array([0, 1, 65535]);
  data.meshes[0]!.primitives[0]!.indices = wideIndices;
  const wide = await object3dFromEngine(data);
  expect((wide.object3d.getObjectByName('Packed') as THREE.Mesh).geometry.index!.array).toBe(wideIndices);
});
