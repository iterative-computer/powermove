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
  convexQuad: ['tlX','tlY','trX','trY','brX','brY','blX','blY'],
  rawShader: true, viewportSafe: false, viewportPadding: ['padding'],
  ui: ['tl','tr','br','bl'].map(c => ({ kind:'point' as const, x:c+'X', y:c+'Y', label:c.toUpperCase() })),
  frag: `#version 300 es
precision highp float;
precision highp int;
in vec2 v_st;
out vec4 o;
uniform sampler2D u_tex;
uniform mat3 u_layerUV, u_ownerFromUV;
uniform vec2 u_res, u_layerRes;
uniform float u_frame, u_ownerValid;
${params.map(p => `uniform float u_${p.k};`).join('\n')}
// All points are top-origin owner UV. Limits are measured in physical pixels.
struct Quad { vec2 a; vec2 b; vec2 c; vec2 d; };
float randomPin(uint seed, uint index) {
  uint x = index ^ seed;
  x = (x ^ (x >> 16u))*2246822519u;
  x = (x ^ (x >> 13u))*3266489917u;
  return float(x ^ (x >> 16u))/4294967295.0;
}
Quad identityQuad() { return Quad(vec2(0),vec2(1,0),vec2(1),vec2(0,1)); }
Quad blendQuad(Quad p, Quad q, float t) {
  return Quad(mix(p.a,q.a,t),mix(p.b,q.b,t),mix(p.c,q.c,t),mix(p.d,q.d,t));
}
float crossPin(vec2 a, vec2 b) { return a.x*b.y-a.y*b.x; }
bool readableQuad(Quad q, vec2 size) {
  vec2 edges[4] = vec2[4]((q.b-q.a)*size,(q.c-q.b)*size,(q.d-q.c)*size,(q.a-q.d)*size);
  for(int i=0;i<4;i++) {
    vec2 e=edges[i], n=edges[(i+1)%4];
    float lengthE=length(e), lengthN=length(n);
    float original=(i%2==0)?size.x:size.y;
    if(lengthE < 0.45*original || lengthE > 1.8*original) return false;
    if(crossPin(e,n) <= 0.0001*lengthE*lengthN) return false;
    // Interior angles 35..145 degrees, without acos precision loss.
    if(abs(dot(-e,n)) > 0.8191520443*lengthE*lengthN) return false;
  }
  return true;
}
Quad randomQuad(uint seed, float pose, float perspective, vec2 size) {
  Quad base=identityQuad();
  // Rejection consumes a deterministic, disjoint slice of the seed stream.
  for(uint attempt=0u;attempt<32u;attempt++) {
    uint key=uint(max(0.0,pose))*4096u+attempt*32u;
    float direction=floor(randomPin(seed,key)*4.0);
    float taper=mix(0.12,0.48,perspective)*randomPin(seed,key+1u);
    vec2 points[4]=vec2[4](base.a,base.b,base.c,base.d);
    for(int i=0;i<4;i++) {
      vec2 p=points[i];
      vec2 inset=vec2(randomPin(seed,key+2u+uint(i)*2u),randomPin(seed,key+3u+uint(i)*2u))*0.06;
      p=mix(p,vec2(0.5),inset);
      if(direction < 2.0) {
        float edge=direction<1.0 ? 1.0-p.x : p.x;
        p.y=mix(p.y,0.5,edge*taper);
      } else {
        float edge=direction<3.0 ? 1.0-p.y : p.y;
        p.x=mix(p.x,0.5,edge*taper);
      }
      points[i]=p;
    }
    Quad candidate=Quad(points[0],points[1],points[2],points[3]);
    if(readableQuad(candidate,size)) return candidate;
  }
  return base;
}
Quad generatedQuad(uint seed, float frame, float interval, float sharpness, float amount, float perspective, vec2 size) {
  float pose=floor(frame/interval), easeFrames=2.0*(1.0-sharpness);
  float blend=easeFrames<0.001 || pose<1.0 ? 1.0 : smoothstep(0.0,easeFrames,mod(frame,interval));
  Quad target=blendQuad(randomQuad(seed,max(0.0,pose-1.0),perspective,size),randomQuad(seed,pose,perspective,size),blend);
  // Interpolating two valid quads need not preserve every metric. Bound the
  // actual eased/Amount result too; deterministic contraction always terminates.
  for(int i=0;i<12;i++) {
    Quad result=blendQuad(identityQuad(),target,amount);
    if(readableQuad(result,size)) return result;
    amount*=0.5;
  }
  return identityQuad();
}
mat3 quadMatrix(Quad q) {
  vec2 delta=q.a-q.b+q.c-q.d,e=q.b-q.c,f=q.d-q.c;
  float det=crossPin(e,f);
  vec2 projective=vec2(crossPin(delta,f),crossPin(e,delta))/det;
  return mat3(vec3(q.b-q.a+projective.x*q.b,projective.x),vec3(q.d-q.a+projective.y*q.d,projective.y),vec3(q.a,1));
}
void main() {
  if(u_ownerValid < 0.5) { o=texture(u_tex,v_st); return; }
  float frame = u_hold > 0.5 ? 0.0 : max(0.0,u_frame);
  float interval = max(1.0,floor(u_every+0.5));
  uint seed=uint(max(0.0,floor(u_seed+0.5)))+uint(max(0.0,floor(u_seedOffset+0.5)))*747796405u;
  Quad generated=generatedQuad(seed,frame,interval,clamp(u_sharpness/100.0,0.0,1.0),clamp(u_amount/100.0,0.0,1.0),clamp(u_perspective/100.0,0.0,1.0),u_layerRes);
  // Manual pins define the outer projective frame. All generated points lie
  // inside the unit rectangle, so a valid manual quad cannot fold them.
  Quad manual=Quad(vec2(u_tlX,u_tlY)/100.0,vec2(1,0)+vec2(u_trX,u_trY)/100.0,vec2(1)+vec2(u_brX,u_brY)/100.0,vec2(0,1)+vec2(u_blX,u_blY)/100.0);
  mat3 forward=quadMatrix(manual)*quadMatrix(generated);
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
