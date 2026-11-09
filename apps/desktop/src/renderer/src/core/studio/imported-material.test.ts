import * as THREE from 'three';
import { describe,expect,it } from 'vitest';
import { extractScene } from './extract';
import { IMPORT_MAP_SLOTS,STUDIO_TRIANGLE_FLOATS } from './layout';
import { describePreparedScene,compileSceneDescription } from './scene-description';
import type { StudioView } from './gpu';
const env={sky:new THREE.Color(),ground:new THREE.Color(),intensity:0};
const view:StudioView={camera:{position:[0,0,3],forward:[0,0,-1],right:[1,0,0],up:[0,1,0],tanY:.5,aspect:1,lensRadius:0,focus:3},exposure:1,backdrop:[0,0,0],backdropOpaque:false,shadowCatcher:false,shadows:true,floorY:0,environment:{zenith:[0,0,0],horizon:[0,0,0],ground:[0,0,0]}};
function graph(){
  const scene=new THREE.Scene(),g=new THREE.PlaneGeometry(2,2);g.setAttribute('uv1',new THREE.Float32BufferAttribute([.1,.2,.3,.4,.5,.6,.7,.8],2));
  const material=new THREE.MeshPhysicalMaterial({specularColor:new THREE.Color(.2,.4,.6),specularIntensity:.75,ior:1.7,clearcoat:.8,clearcoatRoughness:.2,transmission:.6,thickness:.4,attenuationColor:new THREE.Color(.3,.5,.7),attenuationDistance:2,sheen:1,sheenColor:new THREE.Color(.8,.1,.3),sheenRoughness:.35,iridescence:.7,iridescenceIOR:1.4,iridescenceThicknessRange:[120,420],anisotropy:.5,anisotropyRotation:.6});
  material.userData.sourceMaterialIndex=0;
  material.normalScale.set(.6,.7);
  const textures=IMPORT_MAP_SLOTS.map((slot,i)=>{
    const t=new THREE.DataTexture(new Uint8Array(16).fill(i+1),2,2);t.flipY=false;t.channel=1;t.wrapS=THREE.MirroredRepeatWrapping;t.wrapT=THREE.ClampToEdgeWrapping;t.repeat.set(2,3);t.offset.set(.2,.3);t.rotation=.5;(material as any)[slot]=t;return t;
  });
  const mesh=new THREE.Mesh(g,material);mesh.userData.sceneId='panel';mesh.castShadow=mesh.receiveShadow=true;scene.add(mesh);
  return {scene,mesh,material,textures};
}
describe('imported physical material transport',()=>{
  it('retains all 18 map slots, per-slot transforms, secondary UVs and physical scalars',()=>{
    const {scene,mesh,material,textures}=graph(),s=extractScene(scene,()=>true,env),m=s.materials;
    expect(s.maps).toHaveLength(18);expect(m[32]).toBeCloseTo(.2);expect(m[35]).toBeCloseTo(.35);
    expect(m[39]).toBeCloseTo(.7);expect(m[43]).toBe(2);expect(m[44]).toBeCloseTo(.4);expect(m[45]).toBeCloseTo(.6);expect(m[46]).toBeCloseTo(1.4);expect(m[47]).toBe(120);expect(m[48]).toBe(420);expect(m[55]).toBe(1);
    for(const [slot,t] of textures.entries()){
      const o=56+slot*12,index=s.maps.findIndex(map=>'source' in map && map.source===t.image),e=t.matrix.elements;
      expect([...m.slice(o,o+4)]).toEqual([index,1,2,1]);
      expect(m[o+4]).toBeCloseTo(e[0]!);expect(m[o+5]).toBeCloseTo(e[3]!);expect(m[o+6]).toBeCloseTo(e[6]!);expect(m[o+8]).toBeCloseTo(e[1]!);expect(m[o+9]).toBeCloseTo(e[4]!);expect(m[o+10]).toBeCloseTo(e[7]!);
    }
    expect(s.tris.length).toBe(2*STUDIO_TRIANGLE_FLOATS);expect(s.tris[30]).toBe(1);
    expect([...s.tris.slice(24,30)].every(Number.isFinite)).toBe(true);
    mesh.geometry.dispose();material.dispose();textures.forEach(t=>t.dispose());
  });
  it('round-trips maps, UV sets and physical values through the portable scene contract',()=>{
    const {scene}=graph(),s=extractScene(scene,()=>true,env),size=s.mapSize!;
    const document=describePreparedScene(s,view,s.maps.map(()=>new Uint8Array(size*size*4)));
    expect(document.object_ids[0]!.secondary_uvs).toHaveLength(6);
    expect(document.material_options[0]!.imported).toHaveLength(240);
    const compiled=compileSceneDescription(document);
    expect(compiled.scene.materials[31]).toBeCloseTo(.6);
    for(let i=32;i<56;i++)expect(compiled.scene.materials[i]).toBeCloseTo(s.materials[i]!,5);
    expect(compiled.scene.maps).toHaveLength(18);expect(compiled.scene.tris[30]).toBe(1);
    compiled.graph.traverse((o:any)=>{o.geometry?.dispose();o.material?.dispose();});
  });
});
