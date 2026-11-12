import { describe,expect,it,vi } from 'vitest';
import * as THREE from 'three';
import { installEnvironmentFiltering } from './environment-filtering';
import { applyMaterialProperties } from './material-properties';

function shaderFor(material:THREE.Material){
  const shader:any={uniforms:{},fragmentShader:'#include <cube_uv_reflection_fragment>\n#include <lights_fragment_begin>'};
  material.onBeforeCompile(shader,{} as any);return shader;
}
describe('smooth coarse environment reflections',()=>{
  it('keeps material roughness and composes with existing shader/ambient hooks',()=>{
    const material=new THREE.MeshStandardMaterial({roughness:.12}),previous=vi.fn();material.onBeforeCompile=previous;
    applyMaterialProperties(material,{roughness:.12,ambientResponse:50},{roughness:.12,opacity:1,depthWrite:true});
    const shader=shaderFor(material);
    expect(material.roughness).toBe(.12);expect(previous).toHaveBeenCalledOnce();
    expect(shader.fragmentShader).toContain('pmEnvironmentCubic(envMap,uv,tileMin,tileMax)');
    expect(shader.fragmentShader).toContain('if(mipInt >= CUBEUV_MAX_MIP - 2.0) return pmEnvironmentTap(envMap,uv)');
    expect(shader.fragmentShader).toContain('pmAmbientResponse');expect(shader.uniforms.pmAmbientResponse.value).toBe(.5);
    expect(THREE.ShaderChunk.cube_uv_reflection_fragment).not.toContain('pmEnvironmentCubic');
  });
  it('installs once per instance, including independently cloned materials',()=>{
    const material=new THREE.MeshStandardMaterial({roughness:.12});installEnvironmentFiltering(material);
    const first=material.onBeforeCompile;installEnvironmentFiltering(material);expect(material.onBeforeCompile).toBe(first);
    const clone=material.clone();installEnvironmentFiltering(clone);expect(shaderFor(clone).fragmentShader).toContain('pmEnvironmentCubic');
    const basic=new THREE.MeshBasicMaterial(),before=basic.onBeforeCompile;installEnvironmentFiltering(basic);expect(basic.onBeforeCompile).toBe(before);
  });
});
