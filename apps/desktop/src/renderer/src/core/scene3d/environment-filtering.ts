import * as THREE from 'three';

const installed=new WeakSet<THREE.Material>();

// Reconstruct coarse PMREM levels with a smooth cubic B-spline. Bilinear
// magnification leaves a visible grid in broad, otherwise smooth reflections.
// Four bilinear taps implement the cubic filter; sharp levels keep one tap.
const cubic = /* glsl */`
vec4 pmEnvironmentWeights(float t) {
  float t2=t*t, t3=t2*t;
  return vec4(1.0-3.0*t+3.0*t2-t3, 4.0-6.0*t2+3.0*t3,
    1.0+3.0*t+3.0*t2-3.0*t3, t3)/6.0;
}
vec3 pmEnvironmentTap(sampler2D envMap, vec2 uv) {
  #ifdef texture2DGradEXT
    return texture2DGradEXT(envMap,uv,vec2(0.0),vec2(0.0)).rgb;
  #else
    return texture2D(envMap,uv).rgb;
  #endif
}
vec3 pmEnvironmentCubic(sampler2D envMap, vec2 uv, vec2 tileMin, vec2 tileMax) {
  vec2 texel=vec2(CUBEUV_TEXEL_WIDTH,CUBEUV_TEXEL_HEIGHT);
  vec2 position=uv/texel-0.5, base=floor(position), f=fract(position);
  vec4 wx=pmEnvironmentWeights(f.x), wy=pmEnvironmentWeights(f.y);
  vec2 lo=vec2(wx.x+wx.y,wy.x+wy.y), hi=vec2(wx.z+wx.w,wy.z+wy.w);
  vec2 p0=clamp((base-1.0+vec2(wx.y,wy.y)/lo+0.5)*texel,tileMin,tileMax);
  vec2 p1=clamp((base+1.0+vec2(wx.w,wy.w)/hi+0.5)*texel,tileMin,tileMax);
  vec3 row0=mix(pmEnvironmentTap(envMap,p0),pmEnvironmentTap(envMap,vec2(p1.x,p0.y)),hi.x);
  vec3 row1=mix(pmEnvironmentTap(envMap,vec2(p0.x,p1.y)),pmEnvironmentTap(envMap,p1),hi.x);
  return mix(row0,row1,hi.y);
}
`;

const chunk=THREE.ShaderChunk.cube_uv_reflection_fragment
  .replace(/#ifdef texture2DGradEXT[\s\S]*?#endif/, `
    if(mipInt >= CUBEUV_MAX_MIP - 2.0) return pmEnvironmentTap(envMap,uv);
    vec2 texel=vec2(CUBEUV_TEXEL_WIDTH,CUBEUV_TEXEL_HEIGHT);
    vec2 tileMin=uv-localUV*texel+0.5*texel;
    vec2 tileMax=tileMin+(faceSize-1.0)*texel;
    return pmEnvironmentCubic(envMap,uv,tileMin,tileMax);
  `)
  .replace('vec3 bilinearCubeUV(',`${cubic}\nvec3 bilinearCubeUV(`)
  .replace('highp vec2 uv = getUV( direction, face ) * ( faceSize - 2.0 ) + 1.0;',
    'highp vec2 localUV = getUV( direction, face ) * ( faceSize - 2.0 ) + 1.0; highp vec2 uv = localUV;');

/** Preserve other material hooks and leave Three's global chunks untouched. */
export function installEnvironmentFiltering(material: THREE.Material): void {
  if(installed.has(material) || !(material as THREE.MeshStandardMaterial).isMeshStandardMaterial)return;
  installed.add(material);
  const previous=material.onBeforeCompile,key=material.customProgramCacheKey();
  material.onBeforeCompile=function(shader,renderer){
    previous.call(this,shader,renderer);
    shader.fragmentShader=shader.fragmentShader.replace('#include <cube_uv_reflection_fragment>',chunk);
  };
  material.customProgramCacheKey=()=>`${key}:pm-environment-cubic-v1`;
  material.needsUpdate=true;
}
