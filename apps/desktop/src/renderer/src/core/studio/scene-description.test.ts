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
});
