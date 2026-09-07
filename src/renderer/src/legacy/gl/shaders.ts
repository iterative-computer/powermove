/* Ported from js/gl/shaders.js — behavior-preserving. */
import type { EffectDefinition } from '../../kernel/api';
import { toLegacyFx, type LegacyFx } from '../../kernel/glsl';
import { ensureKernel, registryView } from '../kernel-view';
import type { PMRegistry } from '../registry';

export function install(PM: PMRegistry): void {

PM.VERT = `#version 300 es
precision highp float;
in vec2 a_pos;
uniform mat3 u_m;
uniform vec2 u_res;
uniform vec4 u_uv;           // xy = offset, zw = scale
out vec2 v_uv;               // content space (top-left origin)
out vec2 v_st;               // fbo space
out vec2 v_px;               // comp pixels
void main(){
  v_uv = u_uv.xy + a_pos * u_uv.zw;
  v_st = vec2(a_pos.x, 1.0 - a_pos.y);
  vec3 p = u_m * vec3(a_pos, 1.0);
  v_px = p.xy / p.z;
  gl_Position = vec4((p.x / u_res.x) * 2.0 - p.z, p.z - (p.y / u_res.y) * 2.0, 0.0, p.z);
}`;

const PRE = `#version 300 es
precision highp float;
in vec2 v_uv; in vec2 v_st; in vec2 v_px;
uniform sampler2D u_tex;
uniform vec2 u_res;
uniform vec2 u_texel;
uniform float u_time;
uniform float u_prog;
out vec4 o;
vec4 src(vec2 uv){ return texture(u_tex, uv); }
float luma(vec3 c){ return dot(c, vec3(.2126,.7152,.0722)); }
vec3 hue2rgb(float h){ return clamp(abs(mod(h*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.); }
vec3 rgb2hsv(vec3 c){
  vec4 K=vec4(0.,-1./3.,2./3.,-1.); vec4 p=mix(vec4(c.bg,K.wz),vec4(c.gb,K.xy),step(c.b,c.g));
  vec4 q=mix(vec4(p.xyw,c.r),vec4(c.r,p.yzx),step(p.x,c.r));
  float d=q.x-min(q.w,q.y); float e=1e-10;
  return vec3(abs(q.z+(q.w-q.y)/(6.*d+e)), d/(q.x+e), q.x);
}
vec3 hsv2rgb(vec3 c){ return c.z*mix(vec3(1.),clamp(abs(mod(c.x*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.),c.y); }
float hash(vec2 p){ return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
float noise(vec2 p){
  vec2 i=floor(p), f=fract(p); f=f*f*(3.-2.*f);
  return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y);
}
float fbm(vec2 p){ float v=0.,a=.5; for(int i=0;i<5;i++){ v+=a*noise(p); p*=2.03; a*=.5; } return v; }
`;
PM.GLSL_PRE = PRE;

/* ── the composite pass (layer → accumulation, with blend modes) ── */
PM.FRAG_COMPOSITE = PRE + `
uniform sampler2D u_dst;
uniform float u_alpha;
uniform int u_blend;
vec3 blend(vec3 b, vec3 s, int m){
  if(m==1) return b+s;
  if(m==2) return 1.-(1.-b)*(1.-s);
  if(m==3) return b*s;
  if(m==4) return mix(2.*b*s, 1.-2.*(1.-b)*(1.-s), step(.5,b));
  if(m==5) return mix(2.*b*s+b*b*(1.-2.*s), sqrt(b)*(2.*s-1.)+2.*b*(1.-s), step(.5,s));
  if(m==6) return abs(b-s);
  if(m==7) return max(b,s);
  if(m==8) return min(b,s);
  return s;
}
void main(){
  vec4 s = texture(u_tex, v_st);
  vec4 d = texture(u_dst, v_st);
  s.a *= u_alpha; s.rgb *= u_alpha;
  vec3 sc = s.a > .0001 ? s.rgb / s.a : vec3(0.);
  vec3 dc = d.a > .0001 ? d.rgb / d.a : vec3(0.);
  vec3 bc = blend(dc, sc, u_blend);
  vec3 res = mix(dc, bc, s.a);
  float a = s.a + d.a * (1. - s.a);
  o = vec4(res * a, a);
}`;

/* ── plain textured draw (content → layer buffer) ── */
PM.FRAG_DRAW = PRE + `
uniform float u_alpha;
uniform int u_fromFbo;
void main(){
  vec2 uv = u_fromFbo == 1 ? vec2(v_uv.x, 1.0 - v_uv.y) : v_uv;
  vec4 c = texture(u_tex, uv);
  if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) c = vec4(0.);
  o = c * u_alpha;
}`;

PM.FRAG_COPY = PRE + `void main(){ o = texture(u_tex, v_st); }`;

PM.FRAG_SOLID = PRE + `uniform vec4 u_color; void main(){ o = u_color; }`;
PM.FRAG_BACKGROUND_FILL = PRE + `
uniform vec4 u_stops[8];
uniform int u_count;
uniform int u_type;
uniform float u_angle;
void main(){
  vec2 p=v_st-vec2(.5);
  float a=radians(u_angle);
  float t=u_type==2 ? clamp(length(p)*1.4142,0.,1.) : clamp(dot(p,vec2(cos(a),sin(a)))+.5,0.,1.);
  vec3 color=u_stops[0].rgb;
  for(int i=0;i<7;i++){
    if(i>=u_count-1) break;
    vec4 left=u_stops[i], right=u_stops[i+1];
    float amount=smoothstep(left.a,right.a,t);
    color=mix(color,right.rgb,amount);
  }
  o=vec4(color,1.);
}`;

/* ── layer masks: analytic SDF coverage in one fullscreen pass ── */
PM.FRAG_MASK = PRE + `
uniform mat3 u_inv;          // comp px → layer px
uniform int u_cnt;
uniform int u_hasAdd;
uniform vec4 u_g[8];         // cx, cy, w, h   (layer px)
uniform vec4 u_q[8];         // rotation rad, feather px, shape 0rect/1ellipse, mode 0add/1sub
float sdBox(vec2 p, vec2 b){ vec2 d = abs(p) - b; return length(max(d, 0.)) + min(max(d.x, d.y), 0.); }
void main(){
  vec3 local = u_inv * vec3(v_px, 1.0);
  vec2 lp = local.xy / local.z;
  float cov = u_hasAdd == 1 ? 0.0 : 1.0;
  for (int i = 0; i < 8; i++) {
    if (i >= u_cnt) break;
    vec4 g = u_g[i];
    vec4 q = u_q[i];
    if (g.z < .01 || g.w < .01) continue;
    vec2 p = lp - g.xy;
    float r = q.x;
    if (abs(r) > 1e-4) { float c = cos(r), s = sin(r); p = mat2(c, -s, s, c) * p; }
    vec2 h = max(g.zw * .5, vec2(.001));
    float d = q.z < .5 ? sdBox(p, h) : (length(p / h) - 1.0) * min(h.x, h.y);
    float f = max(q.y, 0.0);
    float a = f > .01 ? 1.0 - smoothstep(-f * .5, f * .5, d) : step(d, 0.0);
    if (q.w < .5) cov = max(cov, a);
    else cov = min(cov, 1.0 - a);
  }
  o = vec4(cov, cov, cov, 1.0);
}`;

PM.FRAG_MASK_APPLY = PRE + `
uniform sampler2D u_cov;
void main(){
  vec4 c = texture(u_tex, v_st);
  o = c * texture(u_cov, v_st).r;
}`;

/* The kernel owns the effect table. `PM.FX` is a live view over definitions
   registered by extensions, translated back into the legacy shape so the
   compositor, inspector, palette and FX browser remain unchanged. */
const kernel = ensureKernel(PM);
const legacyFx = new WeakMap<EffectDefinition, LegacyFx>();
const toLegacy = (definition: EffectDefinition): LegacyFx => {
  let cached = legacyFx.get(definition);
  if (!cached) {
    cached = toLegacyFx(definition, PRE);
    legacyFx.set(definition, cached);
  }
  return cached;
};

/* A replaced or removed effect invalidates the compiled program cached under
   `fx:<id>`; without this the compositor would keep drawing the old shader. */
kernel.effects.onChange((change) => {
  if (change.kind === 'add') return;
  PM.GL?.dropProgram?.('fx:' + change.id);
});

PM.FX = registryView<EffectDefinition, LegacyFx>(kernel.effects, { read: (item) => toLegacy(item) });

PM.mkEffect = (type: any) => {
  const def = PM.FX[type]; if (!def) return null;
  const fx: any = { id: PM.uid('f'), type, on: true, p: {} };
  def.params.forEach((pd: any) => { fx.p[pd.k] = PM.P(pd.def); });
  return fx;
};

/* ── shader-layer runtime ──────────────────────────────── */
PM.SHADER_HEADER = `#version 300 es
precision highp float;
in vec2 v_uv;
out vec4 fragColor;
uniform vec2 iResolution;   // layer size in px
uniform float iTime;        // layer-local seconds
uniform float iGlobalTime;  // comp seconds
uniform float iProgress;    // 0..1 across the layer
uniform int iFrame;
uniform vec2 iMouse;
#define uv v_uv
float hash(vec2 p){ return fract(sin(dot(p,vec2(127.1,311.7)))*43758.5453); }
float hash1(float p){ return fract(sin(p*127.1)*43758.5453); }
float noise(vec2 p){ vec2 i=floor(p),f=fract(p); f=f*f*(3.-2.*f);
  return mix(mix(hash(i),hash(i+vec2(1,0)),f.x),mix(hash(i+vec2(0,1)),hash(i+vec2(1,1)),f.x),f.y); }
float fbm(vec2 p){ float v=0.,a=.5; for(int i=0;i<6;i++){v+=a*noise(p);p*=2.02;a*=.5;} return v; }
mat2 rot(float a){ float c=cos(a),s=sin(a); return mat2(c,-s,s,c); }
float luma(vec3 c){ return dot(c, vec3(.2126,.7152,.0722)); }
vec3 rgb2hsv(vec3 c){ vec4 K=vec4(0.,-1./3.,2./3.,-1.); vec4 p=mix(vec4(c.bg,K.wz),vec4(c.gb,K.xy),step(c.b,c.g)); vec4 q=mix(vec4(p.xyw,c.r),vec4(c.r,p.yzx),step(p.x,c.r)); float d=q.x-min(q.w,q.y); return vec3(abs(q.z+(q.w-q.y)/(6.*d+1e-10)), d/(q.x+1e-10), q.x); }
vec3 hsv2rgb(vec3 c){ return c.z*mix(vec3(1.),clamp(abs(mod(c.x*6.+vec3(0.,4.,2.),6.)-3.)-1.,0.,1.),c.y); }
vec3 palette(float t, vec3 a, vec3 b, vec3 c, vec3 d){ return a + b*cos(6.28318*(c*t+d)); }
float sdCircle(vec2 p, float r){ return length(p)-r; }
float sdBox(vec2 p, vec2 b){ vec2 d=abs(p)-b; return length(max(d,0.))+min(max(d.x,d.y),0.); }
`;

PM.SHADER_TEMPLATE = `// Powermove shader layer — GLSL ES 3.0
// uv is 0..1 across the layer. Declare uniforms with @param to get inspector controls.
uniform float uSpeed;   // @param 0.35 0 3
uniform float uScale;   // @param 3.0 0.5 12
uniform vec3  uTint;    // @param #FF6B1A

void main() {
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);
  float t = iTime * uSpeed;
  float n = fbm(p * uScale + vec2(t, t * 0.6));
  n += 0.35 * fbm(p * uScale * 2.4 - t * 0.4);
  float band = smoothstep(0.28, 0.72, n);
  vec3 col = mix(vec3(0.03, 0.03, 0.04), uTint, band);
  col += pow(band, 6.0) * 0.7;
  float vig = 1.0 - 0.7 * dot(p, p);
  fragColor = vec4(col * vig, 1.0);
}`;

/** Parse `// @param default min max` annotations into inspector-ready uniform defs. */
PM.parseUniforms = (code: any) => {
  const out: any[] = [];
  const re = /uniform\s+(float|vec2|vec3|vec4|int|bool)\s+(\w+)\s*;\s*(?:\/\/\s*@param\s*([^\n]*))?/g;
  let m;
  while ((m = re.exec(code))) {
    const [, type, name, ann] = m as any;
    if (/^i(Resolution|Time|GlobalTime|Progress|Frame|Mouse)$/.test(name)) continue;
    const a = (ann || '').trim().split(/\s+/).filter(Boolean);
    const d: any = { name, type, label: name.replace(/^u/, '').replace(/([a-z])([A-Z])/g, '$1 $2') };
    if (type === 'vec3' && a[0] && a[0].startsWith('#')) { d.control = 'color'; d.def = a[0]; }
    else if (type === 'bool') { d.control = 'toggle'; d.def = a[0] === 'true'; }
    else {
      d.control = 'num';
      d.def = a[0] !== undefined ? parseFloat(a[0]) : 0;
      d.min = a[1] !== undefined ? parseFloat(a[1]) : (d.def < 0 ? d.def * 2 : 0);
      d.max = a[2] !== undefined ? parseFloat(a[2]) : Math.max(1, Math.abs(d.def) * 2);
      if (isNaN(d.def)) d.def = 0;
    }
    out.push(d);
  }
  return out;
};

PM.SHADER_PRESETS = {
  'Aurora Field': PM.SHADER_TEMPLATE,
  'Signal Grid': `uniform float uSpeed;  // @param 1.0 0 4
uniform float uCells;  // @param 18.0 2 80
uniform vec3  uInk;    // @param #4C8DFF
void main(){
  vec2 g = uv * uCells;
  vec2 id = floor(g), f = fract(g) - .5;
  float d = sdBox(f, vec2(.34));
  float pulse = smoothstep(.9, 1.0, sin(iTime*uSpeed*2.0 + hash(id)*6.283 + id.x*.2));
  float m = smoothstep(.02, .0, d) * (.12 + pulse);
  vec3 col = vec3(.02) + uInk * m;
  col += uInk * smoothstep(.06,.0,abs(d)) * .25;
  fragColor = vec4(col, 1.0);
}`,
  'Liquid Chrome': `uniform float uSpeed; // @param .5 0 3
uniform float uWarp;  // @param 2.5 0 8
void main(){
  vec2 p = (uv-.5)*2.0; p.x *= iResolution.x/iResolution.y;
  float t = iTime*uSpeed;
  for(int i=0;i<4;i++){ p += vec2(sin(p.y*3.0+t), cos(p.x*3.0-t))*.18*uWarp*.25; }
  float v = fbm(p*1.6+t*.2);
  vec3 col = palette(v, vec3(.5), vec3(.5), vec3(1.,1.,1.), vec3(0.,.33,.67));
  col = mix(col, vec3(luma(vec3(col))), .35);
  fragColor = vec4(col,1.0);
}`,
  'Ember Gradient': `uniform vec3 uA; // @param #FF6B1A
uniform vec3 uB; // @param #120A06
uniform float uAngle; // @param 0.35 0 6.28
void main(){
  vec2 p = uv - .5; p = rot(uAngle) * p;
  float g = smoothstep(-.5,.5,p.y);
  float n = (noise(uv*vec2(600.,600.))-.5)*.012;
  fragColor = vec4(mix(uB,uA,g)+n, 1.0);
}`,
  'Scanline Bars': `uniform float uCount; // @param 40 4 200
uniform float uSpeed; // @param .6 0 4
uniform vec3 uCol;    // @param #E8E2CF
void main(){
  float y = fract(uv.y*uCount - iTime*uSpeed);
  float m = smoothstep(.5,.48,abs(y-.5));
  float fade = smoothstep(0.,.4,iProgress)*smoothstep(1.,.7,iProgress);
  fragColor = vec4(uCol*m*fade, m*fade);
}`,
};
}
