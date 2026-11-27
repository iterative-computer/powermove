import {describe,expect,it,vi} from 'vitest';
import * as THREE from 'three';
import {EnvironmentShadows,environmentSamples} from './environment-shadows';

function panorama(){
  const data=new Float32Array(64*32*4);
  for(let y=20;y<24;y++)for(let x=36;x<40;x++)data.set([20,10,5,1],(y*64+x)*4);
  const texture=new THREE.DataTexture(data,64,32,THREE.RGBAFormat,THREE.FloatType);texture.needsUpdate=true;
  return texture;
}
describe('environment shadow lighting',()=>{
  it('uses radiance and solid angle, caches sampling, and follows texture revisions',()=>{
    const texture=panorama(),regions=environmentSamples(texture);
    expect(regions.length).toBeGreaterThan(0);expect(regions.length).toBeLessThanOrEqual(4);
    for(const s of regions){expect(s.direction.y).toBeGreaterThan(0);expect(s.energy.x/s.energy.y).toBeCloseTo(2);}
    expect(environmentSamples(texture)).toBe(regions);
    texture.needsUpdate=true;expect(environmentSamples(texture)).not.toBe(regions);
    texture.flipY=true;expect(environmentSamples(texture)[0]!.direction.y).toBeLessThan(0);
  });
  it('does not invent light for black, missing or non-pixel maps',()=>{
    expect(environmentSamples(new THREE.Texture())).toEqual([]);
    expect(environmentSamples(new THREE.DataTexture(new Float32Array(16),2,2,THREE.RGBAFormat,THREE.FloatType))).toEqual([]);
  });
  it('rotates shadow-only probes, fits translated geometry, caches passes, and disposes disabled shadows',()=>{
    const scene=new THREE.Scene(),material=new THREE.MeshStandardMaterial(),mesh=new THREE.Mesh(new THREE.BoxGeometry(4,2,3),material);
    mesh.position.set(1000,500,-300);scene.add(mesh);
    const sun=new THREE.DirectionalLight();sun.castShadow=true;scene.add(sun);
    const shadows=new EnvironmentShadows(scene),texture=panorama();shadows.sync(texture,new THREE.Euler(),true);
    expect(shadows.weights.value[0]!.length()).toBe(0);
    const before=shadows.lights.map(l=>l.position.clone().sub(l.target.position).normalize());
    for(const light of shadows.lights){expect(light.intensity).toBe(0);expect(light.castShadow).toBe(true);expect(light.target.position).toEqual(mesh.position);}
    shadows.lights.forEach(l=>l.shadow.needsUpdate=false);shadows.preparePass('a');
    expect(shadows.lights.every(l=>l.shadow.needsUpdate)).toBe(true);
    shadows.lights.forEach(l=>l.shadow.needsUpdate=false);shadows.preparePass('a');
    expect(shadows.lights.some(l=>l.shadow.needsUpdate)).toBe(false);
    shadows.preparePass('b');expect(shadows.lights.every(l=>l.shadow.needsUpdate)).toBe(true);
    shadows.sync(texture,new THREE.Euler(0,Math.PI/2,0),true);
    shadows.lights.forEach((l,i)=>expect(l.position.clone().sub(l.target.position).normalize().distanceTo(before[i]!.clone().applyAxisAngle(new THREE.Vector3(0,1,0),Math.PI/2))).toBeLessThan(1e-8));
    const dispose=vi.spyOn(shadows.lights[0]!.shadow,'dispose');shadows.sync(texture,new THREE.Euler(),false);
    expect(shadows.lights).toHaveLength(0);expect(dispose).toHaveBeenCalledOnce();
    expect(scene.children.filter(o=>o.userData.environmentShadow)).toHaveLength(0);
  });
});
