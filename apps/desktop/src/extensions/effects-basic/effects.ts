import type { EffectDefinition } from 'powermove';

const PRE = `#version 300 es
precision highp float;
in vec2 v_uv; in vec2 v_st; in vec2 v_px;
uniform sampler2D u_tex;
uniform vec2 u_res;
uniform vec2 u_coordRes;
uniform vec2 u_pxOrigin;
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
const F = (body: any, extra: any = '') => PRE + extra + '\nvoid main(){\n' + body + '\n}';

const FX: Record<string, Omit<EffectDefinition, 'id' | 'rawShader'>> = {
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
      vec2 q = (v_uv - .5) * 2.0;
      float d = length(q) * .72;
      float v = 1. - smoothstep(1. - u_p1/100., 1.0, d) * (u_p0/100.);
      o = vec4(c.rgb * v, c.a);`, 'uniform float u_p0,u_p1;'),
  },
  chroma: {
    label: 'Chromatic Aberration', group: 'Distort', passes: 1,
    params: [{ k: 'amount', label: 'Amount', def: 6, min: 0, max: 80, step: .5, unit: 'px' }],
    frag: F(`
      vec2 dir = (vec2(v_uv.x, 1. - v_uv.y) - .5);
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
      float block = max(u_p0, 1.);
      vec2 local = v_px - u_pxOrigin;
      vec2 center = (floor(local / block) + .5) * block;
      vec2 uv = vec2(center.x / u_res.x, 1. - center.y / u_res.y);
      o = texture(u_tex, uv);`, 'uniform float u_p0;'),
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
      vec2 p = vec2(v_uv.x, 1. - v_uv.y) * u_p1 * 3.0 + vec2(u_time * u_p2, u_time * u_p2 * .7);
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
  gradient: {
    label: 'Gradient Ramp', group: 'Generate', passes: 1,
    params: [{ k: 'startColor', label: 'Start Color', def: '#FFFFFF', type: 'color' },
             { k: 'endColor', label: 'End Color', def: '#000000', type: 'color' },
             { k: 'angle', label: 'Angle', def: 90, min: -360, max: 360, step: 1, unit: '°' },
             { k: 'centerX', label: 'Center X', def: 0, min: -100, max: 100, step: .5, unit: '%' },
             { k: 'centerY', label: 'Center Y', def: 0, min: -100, max: 100, step: .5, unit: '%' },
             { k: 'spread', label: 'Spread', def: 100, min: 1, max: 400, step: 1, unit: '%' },
             { k: 'midpoint', label: 'Midpoint', def: 50, min: 1, max: 99, step: .5, unit: '%' },
             { k: 'radial', label: 'Radial', def: false, type: 'toggle' },
             { k: 'shade', label: 'Keep Luminance', def: false, type: 'toggle' },
             { k: 'dither', label: 'Dither', def: 1, min: 0, max: 20, step: .1, unit: '' },
             { k: 'amount', label: 'Amount', def: 100, min: 0, max: 100, step: 1, unit: '%' }],
    /* Named uniforms (the current param contract) rather than the positional
       u_p<i> of the older effects above — eleven params read better by name.
       The ramp spans the frame like Vignette, and paints inside the layer's own
       alpha, so text and shapes keep their edges instead of being replaced by a
       full-frame fill. */
    frag: F(`
      vec4 c = texture(u_tex, v_st); if (c.a < .0005) { o = c; return; }
      vec3 x = c.rgb / c.a;
      vec2 p = (vec2(v_uv.x, 1. - v_uv.y) - .5) - vec2(u_centerX, -u_centerY) / 200.;
      float span = max(u_spread / 100., .001);
      float a = radians(u_angle);
      /* Aspect-correct the radial distance so the rings stay circular. */
      float t = u_radial > .5
        ? length(p * vec2(u_coordRes.x / max(u_coordRes.y, 1.), 1.)) * 2. / span
        : dot(p, vec2(cos(a), -sin(a))) / span + .5;
      /* Static per-pixel jitter breaks 8-bit banding without shimmering. */
      t = clamp(t + (hash(v_px) - .5) * u_dither * .004, 0., 1.);
      t = pow(t, log(.5) / log(clamp(u_midpoint / 100., .01, .99)));
      vec3 g = mix(u_startColor, u_endColor, t);
      if (u_shade > .5) { vec3 h = rgb2hsv(g); g = hsv2rgb(vec3(h.x, h.y, rgb2hsv(max(x, 0.)).z)); }
      o = vec4(mix(x, g, u_amount / 100.) * c.a, c.a);`,
      `uniform vec3 u_startColor,u_endColor;
       uniform float u_angle,u_centerX,u_centerY,u_spread,u_midpoint,u_radial,u_shade,u_dither,u_amount;`),
  },
};

const VIEWPORT_PADDING: Record<string, string[]> = {
  blur: ['amount'], motionblurDir: ['amount'], glow: ['radius'],
  displace: ['amount'], shadow: ['distance', 'softness'], chroma: ['amount'],
};

export const EFFECTS: EffectDefinition[] = Object.entries(FX).map(([id, definition]) => ({
  ...definition,
  id,
  rawShader: true,
  viewportSafe: true,
  viewportPadding: VIEWPORT_PADDING[id] || [],
}));
