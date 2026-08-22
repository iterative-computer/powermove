/* Powermove — GLSL library: quad vertex shader, effect passes, shader-layer runtime. */
(() => {
const PM = window.PM;

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
  v_px = p.xy;
  gl_Position = vec4((p.x / u_res.x) * 2.0 - 1.0, 1.0 - (p.y / u_res.y) * 2.0, 0.0, 1.0);
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
  vec2 lp = (u_inv * vec3(v_px, 1.0)).xy;
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

/* ── effects registry ──────────────────────────────────── */
const F = (body, extra = '') => PRE + extra + '\nvoid main(){\n' + body + '\n}';

const FX = {
  blur: {
    label: 'Gaussian Blur', group: 'Blur & Sharpen', passes: 2,
    params: [{ k: 'amount', label: 'Blurriness', def: 24, min: 0, max: 400, step: .5, unit: 'px' }],
    frag: F(`
      float r = u_p0;
      if (r < .25) { o = texture(u_tex, v_st); return; }
      vec2 dir = u_pass == 0 ? vec2(1.,0.) : vec2(0.,1.);
      float sigma = r * .5;
      vec4 sum = vec4(0.); float wsum = 0.;
      int N = int(clamp(r*.6, 3., 28.));
      for (int i = -28; i <= 28; i++) {
        if (i < -N || i > N) continue;
        float x = float(i) / float(N) * r;
        float w = exp(-(x*x)/(2.*sigma*sigma+1e-4));
        sum += texture(u_tex, v_st + dir * u_texel * x) * w; wsum += w;
      }
      o = sum / max(wsum, 1e-4);`, 'uniform float u_p0; uniform int u_pass;'),
  },
  motionblurDir: {
    label: 'Directional Blur', group: 'Blur & Sharpen', passes: 1,
    params: [{ k: 'amount', label: 'Length', def: 30, min: 0, max: 300, step: 1, unit: 'px' },
             { k: 'angle', label: 'Direction', def: 0, min: -360, max: 360, step: 1, unit: '°' }],
    frag: F(`
      float a = radians(u_p1); vec2 dir = vec2(cos(a), sin(a)) * u_texel * u_p0;
      vec4 s = vec4(0.);
      for (int i = 0; i < 17; i++) { float t = (float(i)/16.0 - .5); s += texture(u_tex, v_st + dir * t); }
      o = s / 17.0;`, 'uniform float u_p0,u_p1;'),
  },
  sharpen: {
    label: 'Sharpen', group: 'Blur & Sharpen', passes: 1,
    params: [{ k: 'amount', label: 'Amount', def: 40, min: 0, max: 300, step: 1, unit: '%' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st);
      vec4 b = texture(u_tex,v_st+vec2(u_texel.x,0.))+texture(u_tex,v_st-vec2(u_texel.x,0.))
             + texture(u_tex,v_st+vec2(0.,u_texel.y))+texture(u_tex,v_st-vec2(0.,u_texel.y));
      o = vec4(clamp(c.rgb + (c.rgb - b.rgb*.25) * (u_p0/100.), 0., 4.), c.a);`, 'uniform float u_p0;'),
  },
  glow: {
    label: 'Glow', group: 'Stylize', passes: 3,
    params: [{ k: 'threshold', label: 'Threshold', def: 55, min: 0, max: 100, step: 1, unit: '%' },
             { k: 'radius', label: 'Radius', def: 60, min: 0, max: 400, step: 1, unit: 'px' },
             { k: 'intensity', label: 'Intensity', def: 90, min: 0, max: 400, step: 1, unit: '%' }],
    frag: F(`
      if (u_pass == 0) {
        vec4 c = texture(u_tex, v_st);
        float l = luma(c.rgb / max(c.a, .001));
        float k = smoothstep(u_p0/100. - .08, u_p0/100. + .08, l);
        o = c * k; return;
      }
      vec2 dir = u_pass == 1 ? vec2(1.,0.) : vec2(0.,1.);
      float r = max(u_p1, .5); float sigma = r * .5;
      vec4 sum = vec4(0.); float ws = 0.;
      for (int i = -20; i <= 20; i++) {
        float x = float(i)/20.0*r; float w = exp(-(x*x)/(2.*sigma*sigma+1e-4));
        sum += texture(u_tex, v_st + dir*u_texel*x) * w; ws += w;
      }
      o = sum / max(ws,1e-4);
      if (u_pass == 2) o = texture(u_orig, v_st) + o * (u_p2/100.);`,
      'uniform float u_p0,u_p1,u_p2; uniform int u_pass; uniform sampler2D u_orig;'),
    keepOrig: true,
  },
  color: {
    label: 'Color', group: 'Color', passes: 1,
    params: [{ k: 'exposure', label: 'Exposure', def: 0, min: -4, max: 4, step: .01, unit: 'ev' },
             { k: 'contrast', label: 'Contrast', def: 0, min: -100, max: 100, step: 1, unit: '' },
             { k: 'saturation', label: 'Saturation', def: 0, min: -100, max: 200, step: 1, unit: '' },
             { k: 'hue', label: 'Hue', def: 0, min: -180, max: 180, step: 1, unit: '°' },
             { k: 'temperature', label: 'Temperature', def: 0, min: -100, max: 100, step: 1, unit: '' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st); if (c.a < .0005) { o = c; return; }
      vec3 x = c.rgb / c.a;
      x *= pow(2., u_p0);
      x = (x - .5) * (1. + u_p1/100.) + .5;
      float l = luma(x); x = mix(vec3(l), x, 1. + u_p2/100.);
      if (abs(u_p3) > .01) { vec3 hsv = rgb2hsv(max(x,0.)); hsv.x = fract(hsv.x + u_p3/360.); x = hsv2rgb(hsv); }
      x += vec3(u_p4, u_p4*.06, -u_p4) * .0035;
      o = vec4(clamp(x,0.,8.) * c.a, c.a);`, 'uniform float u_p0,u_p1,u_p2,u_p3,u_p4;'),
  },
  levels: {
    label: 'Levels', group: 'Color', passes: 1,
    params: [{ k: 'inBlack', label: 'Input Black', def: 0, min: 0, max: 100, step: .5, unit: '' },
             { k: 'inWhite', label: 'Input White', def: 100, min: 0, max: 100, step: .5, unit: '' },
             { k: 'gamma', label: 'Gamma', def: 1, min: .1, max: 4, step: .01, unit: '' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st); if (c.a < .0005) { o = c; return; }
      vec3 x = c.rgb / c.a;
      x = clamp((x - u_p0/100.) / max(u_p1/100. - u_p0/100., 1e-3), 0., 1.);
      x = pow(x, vec3(1./max(u_p2,.01)));
      o = vec4(x * c.a, c.a);`, 'uniform float u_p0,u_p1,u_p2;'),
  },
  duotone: {
    label: 'Duotone', group: 'Color', passes: 1,
    params: [{ k: 'shadow', label: 'Shadow', def: '#1B2A4A', type: 'color' },
             { k: 'highlight', label: 'Highlight', def: '#FFD66B', type: 'color' },
             { k: 'amount', label: 'Amount', def: 100, min: 0, max: 100, step: 1, unit: '%' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st); if (c.a < .0005) { o = c; return; }
      vec3 x = c.rgb / c.a; float l = luma(x);
      vec3 t = mix(u_c0, u_c1, smoothstep(0., 1., l));
      o = vec4(mix(x, t, u_p2/100.) * c.a, c.a);`, 'uniform vec3 u_c0,u_c1; uniform float u_p2;'),
  },
  grain: {
    label: 'Film Grain', group: 'Stylize', passes: 1,
    params: [{ k: 'amount', label: 'Amount', def: 12, min: 0, max: 100, step: .5, unit: '%' },
             { k: 'size', label: 'Size', def: 1.4, min: .3, max: 8, step: .1, unit: '' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st);
      float n = hash(floor(v_px/max(u_p1,.1)) + fract(u_time)*137.0) - .5;
      o = vec4(clamp(c.rgb + n * (u_p0/100.) * c.a, 0., 8.), c.a);`, 'uniform float u_p0,u_p1;'),
  },
  vignette: {
    label: 'Vignette', group: 'Stylize', passes: 1,
    params: [{ k: 'amount', label: 'Amount', def: 45, min: 0, max: 100, step: 1, unit: '%' },
             { k: 'feather', label: 'Feather', def: 60, min: 1, max: 100, step: 1, unit: '%' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st);
      vec2 q = (v_px/u_res - .5) * 2.0;
      float d = length(q) * .72;
      float v = 1. - smoothstep(1. - u_p1/100., 1.0, d) * (u_p0/100.);
      o = vec4(c.rgb * v, c.a);`, 'uniform float u_p0,u_p1;'),
  },
  chroma: {
    label: 'Chromatic Aberration', group: 'Distort', passes: 1,
    params: [{ k: 'amount', label: 'Amount', def: 6, min: 0, max: 80, step: .5, unit: 'px' }],
    frag: F(`
      vec2 dir = (v_st - .5);
      vec2 d = dir * u_texel * u_p0 * 2.0;
      float r = texture(u_tex, v_st + d).r;
      vec4 g = texture(u_tex, v_st);
      float b = texture(u_tex, v_st - d).b;
      o = vec4(r, g.g, b, g.a);`, 'uniform float u_p0;'),
  },
  pixelate: {
    label: 'Mosaic', group: 'Stylize', passes: 1,
    params: [{ k: 'size', label: 'Block Size', def: 16, min: 1, max: 200, step: 1, unit: 'px' }],
    frag: F(`
      vec2 s = max(u_p0, 1.) * u_texel;
      o = texture(u_tex, (floor(v_st/s) + .5) * s);`, 'uniform float u_p0;'),
  },
  posterize: {
    label: 'Posterize', group: 'Stylize', passes: 1,
    params: [{ k: 'levels', label: 'Levels', def: 6, min: 2, max: 64, step: 1, unit: '' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st); if (c.a<.0005){o=c;return;}
      vec3 x = c.rgb/c.a; float n = max(u_p0,2.);
      o = vec4(floor(x*n+.5)/n*c.a, c.a);`, 'uniform float u_p0;'),
  },
  displace: {
    label: 'Turbulent Displace', group: 'Distort', passes: 1,
    params: [{ k: 'amount', label: 'Amount', def: 24, min: 0, max: 400, step: 1, unit: 'px' },
             { k: 'scale', label: 'Size', def: 2.2, min: .2, max: 20, step: .1, unit: '' },
             { k: 'speed', label: 'Evolution', def: .4, min: -4, max: 4, step: .05, unit: '' }],
    frag: F(`
      vec2 p = v_st * u_p1 * 3.0 + vec2(u_time * u_p2, u_time * u_p2 * .7);
      vec2 d = vec2(fbm(p) - .5, fbm(p + 31.7) - .5) * u_p0 * u_texel * 2.0;
      o = texture(u_tex, v_st + d);`, 'uniform float u_p0,u_p1,u_p2;'),
  },
  shadow: {
    label: 'Drop Shadow', group: 'Stylize', passes: 3,
    params: [{ k: 'distance', label: 'Distance', def: 20, min: 0, max: 400, step: 1, unit: 'px' },
             { k: 'angle', label: 'Angle', def: 135, min: -360, max: 360, step: 1, unit: '°' },
             { k: 'softness', label: 'Softness', def: 40, min: 0, max: 300, step: 1, unit: 'px' },
             { k: 'opacity', label: 'Opacity', def: 60, min: 0, max: 100, step: 1, unit: '%' },
             { k: 'color', label: 'Color', def: '#000000', type: 'color' }],
    frag: F(`
      if (u_pass == 0) {
        float a = radians(u_p1); vec2 off = vec2(cos(a), -sin(a)) * u_p0 * u_texel;
        o = vec4(0.,0.,0., texture(u_tex, v_st - off).a);
        return;
      }
      vec2 dir = u_pass == 1 ? vec2(1.,0.) : vec2(0.,1.);
      float r = max(u_p2, .5); float sigma = r*.5; vec4 sum = vec4(0.); float ws = 0.;
      for (int i=-18;i<=18;i++){ float x=float(i)/18.*r; float w=exp(-(x*x)/(2.*sigma*sigma+1e-4));
        sum += texture(u_tex, v_st + dir*u_texel*x)*w; ws+=w; }
      vec4 sh = sum/max(ws,1e-4);
      if (u_pass == 2) {
        vec4 o0 = texture(u_orig, v_st);
        float sa = sh.a * (u_p3/100.);
        vec3 sc = u_c4 * sa;
        o = vec4(o0.rgb + sc * (1.-o0.a), o0.a + sa * (1.-o0.a));
      } else o = sh;`,
      'uniform float u_p0,u_p1,u_p2,u_p3; uniform vec3 u_c4; uniform int u_pass; uniform sampler2D u_orig;'),
    keepOrig: true,
  },
  invert: {
    label: 'Invert', group: 'Color', passes: 1,
    params: [{ k: 'amount', label: 'Amount', def: 100, min: 0, max: 100, step: 1, unit: '%' }],
    frag: F(`
      vec4 c = texture(u_tex, v_st); if (c.a<.0005){o=c;return;}
      vec3 x = c.rgb/c.a; o = vec4(mix(x, 1.-x, u_p0/100.)*c.a, c.a);`, 'uniform float u_p0;'),
  },
};
PM.FX = FX;

PM.mkEffect = (type) => {
  const def = FX[type]; if (!def) return null;
  const fx = { id: PM.uid('f'), type, on: true, p: {} };
  def.params.forEach(pd => { fx.p[pd.k] = PM.P(pd.def); });
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
PM.parseUniforms = (code) => {
  const out = [];
  const re = /uniform\s+(float|vec2|vec3|vec4|int|bool)\s+(\w+)\s*;\s*(?:\/\/\s*@param\s*([^\n]*))?/g;
  let m;
  while ((m = re.exec(code))) {
    const [, type, name, ann] = m;
    if (/^i(Resolution|Time|GlobalTime|Progress|Frame|Mouse)$/.test(name)) continue;
    const a = (ann || '').trim().split(/\s+/).filter(Boolean);
    const d = { name, type, label: name.replace(/^u/, '').replace(/([a-z])([A-Z])/g, '$1 $2') };
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
})();
