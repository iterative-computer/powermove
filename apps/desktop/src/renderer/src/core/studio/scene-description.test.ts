import { STUDIO_MATERIAL_FLOATS } from './layout';
import * as THREE from 'three';
import { describe, expect, it } from 'vitest';
import { extractScene } from './extract';
import { compileSceneDescription, describePreparedScene } from './scene-description';
import type { StudioView } from './gpu';
const env={sky:new THREE.Color(),ground:new THREE.Color(),intensity:0};
const view:StudioView={camera:{position:[0,0,5],forward:[0,0,-1],right:[1,0,0],up:[0,1,0],tanY:.5,aspect:1,lensRadius:0,focus:5},exposure:1,backdrop:[0,0,0],backdropOpaque:false,shadowCatcher:false,shadows:true,floorY:0,environment:{zenith:[.5,.5,.5],horizon:[.3,.3,.3],ground:[.1,.1,.1]}};
describe('Studio Rust scene contract',()=>{
 it('round trips world-space geometry, material policies and stable layer identities',()=>{
  const graph=new THREE.Scene(),box=new THREE.Mesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial({color:'#ff0000',side:THREE.DoubleSide}));
  box.userData.sceneId='model';box.userData.studioPart='0_2';box.position.set(5,2,-3);box.rotation.y=.4;box.castShadow=false;box.receiveShadow=true;
  box.material.userData.materialAmbientUniform={value:.25};graph.add(box);
  const prepared=extractScene(graph,()=>true,env),json=JSON.parse(JSON.stringify(describePreparedScene(prepared,view,[])));
  expect(json.scene.objects[0].mesh.indices).toHaveLength(12);expect(json.object_ids[0]).toMatchObject({layer:'model',part:'0_2',cast_shadow:false,receive_shadow:true});
  expect(json.material_options[0].ambient_response).toBe(.25);
  const rebuilt=compileSceneDescription(json).scene;
  expect(rebuilt.objects).toEqual(['model']);expect(rebuilt.triangles).toBe(12);
  expect(rebuilt.bounds.min.distanceTo(prepared.bounds.min)).toBeLessThan(1e-6);expect(rebuilt.bounds.max.distanceTo(prepared.bounds.max)).toBeLessThan(1e-6);
  expect(rebuilt.materials[24]).toBe(.25);expect(rebuilt.materials[25]!&1).toBe(0);
 });
 it('keeps vertex colors and alpha through JSON without storing NaN floats',()=>{
  const graph=new THREE.Scene(),g=new THREE.PlaneGeometry();g.setAttribute('color',new THREE.Float32BufferAttribute([1,0,0,.5,0,1,0,1,0,0,1,1,1,1,0,1],4));
  const mesh=new THREE.Mesh(g,new THREE.MeshStandardMaterial({vertexColors:true,side:THREE.DoubleSide}));mesh.userData.sceneId='art';graph.add(mesh);
  const json=JSON.parse(JSON.stringify(describePreparedScene(extractScene(graph,()=>true,env),view,[])));
  expect(json.object_ids[0].vertex_colors).toHaveLength(6);expect(json.object_ids[0].vertex_colors.flat()).not.toContain(null);
  const rebuilt=compileSceneDescription(json).scene;expect(rebuilt.materials[25]!&16).toBe(16);
 });
 it('updates morph geometry while retaining static mesh buffers during ordinary transforms',()=>{
  const graph=new THREE.Scene(),g=new THREE.BoxGeometry(),base=g.getAttribute('position');
  const morphed=base.clone();for(let i=0;i<morphed.count;i++)morphed.setX(i,base.getX(i)+2);
  g.morphAttributes.position=[morphed];const mesh=new THREE.Mesh(g,new THREE.MeshStandardMaterial());mesh.userData.sceneId='morph';graph.add(mesh);
  const rest=extractScene(graph,()=>true,env);mesh.morphTargetInfluences![0]=1;
  const moved=extractScene(graph,()=>true,env);expect(moved.geometryKey).not.toBe(rest.geometryKey);expect(moved.bounds.min.x-rest.bounds.min.x).toBeCloseTo(2);
 });
 it('evaluates skinned normals and positions in the same bone pose',()=>{
  const g=new THREE.BufferGeometry();g.setAttribute('position',new THREE.Float32BufferAttribute([-1,-1,0,1,-1,0,0,1,0],3));g.setAttribute('normal',new THREE.Float32BufferAttribute([0,0,1,0,0,1,0,0,1],3));
  g.setAttribute('skinIndex',new THREE.Uint16BufferAttribute(new Uint16Array(12),4));g.setAttribute('skinWeight',new THREE.Float32BufferAttribute([1,0,0,0,1,0,0,0,1,0,0,0],4));
  const mesh=new THREE.SkinnedMesh(g,new THREE.MeshStandardMaterial()),bone=new THREE.Bone();mesh.add(bone);mesh.bind(new THREE.Skeleton([bone]));mesh.userData.sceneId='skin';bone.rotation.y=Math.PI/2;
  const graph=new THREE.Scene();graph.add(mesh);const prepared=extractScene(graph,()=>true,env);
  expect(prepared.tris[0]).toBeCloseTo(1);expect(prepared.tris[2]).toBeCloseTo(0);expect(prepared.bounds.max.x-prepared.bounds.min.x).toBeLessThan(1e-6);
 });
 it('keeps standard imported anisotropy and dispersion alongside Studio presets',()=>{
  const graph=new THREE.Scene(),material=new THREE.MeshPhysicalMaterial({anisotropy:.7,dispersion:.4});
  const mesh=new THREE.Mesh(new THREE.BoxGeometry(),material);mesh.userData.sceneId='imported';graph.add(mesh);
  const prepared=extractScene(graph,()=>true,env);expect(prepared.materials[15]).toBeCloseTo(.7);expect(prepared.materials[22]).toBeCloseTo(.4);
 });

});

it('keeps one mesh BVH for a whole imported instance set',()=>{
 const graph=new THREE.Scene(),mesh=new THREE.InstancedMesh(new THREE.BoxGeometry(),new THREE.MeshStandardMaterial(),100);
 mesh.userData.sceneId='forest';for(let i=0;i<100;i++)mesh.setMatrixAt(i,new THREE.Matrix4().makeTranslation(i*2,0,0));graph.add(mesh);
 const prepared=extractScene(graph,()=>true,env);expect(prepared.triangles).toBe(12);expect(prepared.instanceCount).toBe(100);expect(prepared.bounds.max.x).toBeCloseTo(198.5);
 const description=describePreparedScene(prepared,view,[]);expect(description.scene.objects).toHaveLength(100);expect(description.layer_ids).toEqual(['forest']);
});
it('shares cloned file geometry and identical materials within one model layer',()=>{
 const graph=new THREE.Scene(),g=new THREE.BoxGeometry();
 for(let i=0;i<2;i++){const clone=g.clone();clone.userData.studioGeometryKey=g.uuid;const mesh=new THREE.Mesh(clone,new THREE.MeshStandardMaterial());mesh.userData.sceneId='assembly';mesh.position.x=i*2;graph.add(mesh);}
 const prepared=extractScene(graph,()=>true,env);expect(prepared.instanceCount).toBe(2);expect(prepared.triangles).toBe(12);expect(prepared.materials.length).toBe(STUDIO_MATERIAL_FLOATS);
});
it('retains imported instance colors without copying mesh triangles',()=>{
 const mesh=new THREE.InstancedMesh(new THREE.BoxGeometry(),new THREE.MeshBasicMaterial({color:'#ffffff'}),2);mesh.userData.sceneId='colored-instances';mesh.setColorAt(0,new THREE.Color(1,0,0));mesh.setColorAt(1,new THREE.Color(0,0,1));mesh.setMatrixAt(1,new THREE.Matrix4().makeTranslation(2,0,0));
 const graph=new THREE.Scene();graph.add(mesh);const prepared=extractScene(graph,()=>true,env),doc=describePreparedScene(prepared,view,[]);
 expect(prepared.triangles).toBe(12);expect(prepared.instanceCount).toBe(2);
 const bases=doc.scene.objects.map(o=>doc.scene.materials[o.material]!.base);expect(bases).toContainEqual({x:1,y:0,z:0});expect(bases).toContainEqual({x:0,y:0,z:1});
});
