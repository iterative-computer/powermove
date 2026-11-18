import type { EffectDefinition, PowermoveAPI } from 'powermove';
const number = (k: string, label: string, def: number, min: number, max: number) =>
  ({ k, label, def, min, max });
const params = [
  number('amount', 'Amount', 60, 0, 100),
  number('every', 'Every N frames', 12, 1, 120),
  number('sharpness', 'Snap sharpness', 100, 0, 100),
  { k: 'hold', label: 'Hold', type: 'toggle' as const, def: false },
  number('seed', 'Seed', 1, 0, 65535),
  number('seedOffset', 'Layer seed offset', 0, 0, 65535),
  number('perspective', 'Perspective', 70, 0, 100),
  ...['tl', 'tr', 'br', 'bl'].flatMap(c => [
    number(c+'X', c.toUpperCase()+' X', 0, -100, 100),
    number(c+'Y', c.toUpperCase()+' Y', 0, -100, 100),
  ]),
  { ...number('padding', 'Buffer padding (pixels)', 512, 0, 4096), advanced: true },
];
// Full shaders declare uniforms explicitly, as required by rawShader.
export const definition: EffectDefinition = {
  id: 'corner-pin-snap.fx', label: 'Corner Pin Snap', group: 'Distort', params,
  rawShader: true, viewportSafe: false, viewportPadding: ['padding'],
  ui: ['tl','tr','br','bl'].map(c => ({ kind:'point' as const, x:c+'X', y:c+'Y', label:c.toUpperCase() })),
  frag: `#version 300 es
precision highp float;
precision highp int;
in vec2 v_st;
out vec4 o;
uniform sampler2D u_tex;
uniform mat3 u_layerUV, u_ownerFromUV;
uniform vec2 u_res;
uniform float u_frame, u_ownerValid;
${params.map(p => `uniform float u_${p.k};`).join('\n')}
// Integer hash avoids device-dependent sin/fract pose changes.
float randomPin(uint index) {
  uint x = index ^ (uint(max(0.0,floor(u_seed+0.5))) + uint(max(0.0,floor(u_seedOffset+0.5)))*747796405u);
  x = (x ^ (x >> 16u))*2246822519u;
  x = (x ^ (x >> 13u))*3266489917u;
  return float(x ^ (x >> 16u))/4294967295.0;
}
vec2 pin(float pose, uint corner, vec2 base) {
  uint key = uint(max(0.0,pose))*8u+corner*2u;
  vec2 jitter = vec2(randomPin(key),randomPin(key+1u))*2.0-1.0;
  float perspective = clamp(u_perspective/100.0,0.0,1.0);
  float amount = clamp(u_amount/100.0,0.0,1.0);
  float direction = floor(randomPin(uint(max(0.0,pose))*8u+100003u)*4.0);
  // Keep random poses convex: taper one of four edges, then add bounded tilt.
  if(direction < 2.0) {
    float edge = direction < 1.0 ? base.y : 1.0-base.y;
    float vanishing = mix(1.0,0.4,perspective*edge);
    vec2 taper = vec2((0.5-base.x)*perspective*edge*0.55,0.0);
    return base + amount*(taper+jitter*vec2(0.22*vanishing,0.12));
  }
  float edge = direction < 3.0 ? base.x : 1.0-base.x;
  float vanishing = mix(1.0,0.4,perspective*edge);
  vec2 taper = vec2(0.0,(0.5-base.y)*perspective*edge*0.55);
  return base + amount*(taper+jitter*vec2(0.12,0.22*vanishing));
}
void main() {
  if(u_ownerValid < 0.5) { o=texture(u_tex,v_st); return; }
  float frame = u_hold > 0.5 ? 0.0 : max(0.0,u_frame);
  float interval = max(1.0,floor(u_every+0.5));
  float pose = floor(frame/interval);
  float easeFrames = 2.0*(1.0-u_sharpness/100.0);
  float blend = easeFrames < 0.001 || pose < 1.0 ? 1.0 : smoothstep(0.0,easeFrames,mod(frame,interval));
  vec2 a = mix(pin(max(0.0,pose-1.0),0u,vec2(0,0)),pin(pose,0u,vec2(0,0)),blend)+vec2(u_tlX,u_tlY)/100.0;
  vec2 b = mix(pin(max(0.0,pose-1.0),1u,vec2(1,0)),pin(pose,1u,vec2(1,0)),blend)+vec2(u_trX,u_trY)/100.0;
  vec2 c = mix(pin(max(0.0,pose-1.0),2u,vec2(1,1)),pin(pose,2u,vec2(1,1)),blend)+vec2(u_brX,u_brY)/100.0;
  vec2 d = mix(pin(max(0.0,pose-1.0),3u,vec2(0,1)),pin(pose,3u,vec2(0,1)),blend)+vec2(u_blX,u_blY)/100.0;
  vec2 delta = a-b+c-d, e = b-c, f = d-c;
  float det = e.x*f.y-f.x*e.y;
  vec2 projective = abs(det)<1e-7 ? vec2(0) : vec2(delta.x*f.y-f.x*delta.y,e.x*delta.y-delta.x*e.y)/det;
  mat3 forward = mat3(vec3(b-a+projective.x*b,projective.x),vec3(d-a+projective.y*d,projective.y),vec3(a,1));
  if(abs(determinant(forward))<1e-7) { o=texture(u_tex,v_st); return; }
  // Texture UV is bottom-origin; owner transforms use top-origin pixels.
  vec2 outputPixel = vec2(v_st.x,1.0-v_st.y)*u_res;
  vec3 local = u_layerUV*vec3(outputPixel,1);
  if(abs(local.z)<1e-7) { o=vec4(0); return; }
  vec3 source = inverse(forward)*vec3(local.xy/local.z,1);
  if(abs(source.z)<1e-7) { o=vec4(0); return; }
  vec2 uv = source.xy/source.z;
  vec2 edgeWidth = max(0.5*fwidth(uv),vec2(1e-6));
  vec2 coverage = smoothstep(-edgeWidth,edgeWidth,uv)*(1.0-smoothstep(1.0-edgeWidth,1.0+edgeWidth,uv));
  vec3 pixel = u_ownerFromUV*vec3(uv,1);
  if(abs(pixel.z)<1e-7) { o=vec4(0); return; }
  vec2 texUV = pixel.xy/pixel.z/u_res;
  texUV.y = 1.0-texUV.y;
  o=texture(u_tex,texUV)*(coverage.x*coverage.y);
}`,
};
export default function activate(api: PowermoveAPI) { api.effects.register(definition); }
