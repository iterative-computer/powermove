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
