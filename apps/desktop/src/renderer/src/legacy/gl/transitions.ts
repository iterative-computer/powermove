import type { TransitionDefinition } from '../../kernel/api';
import { ensureKernel, registryView } from '../kernel-view';
import type { PMRegistry } from '../registry';

const paramDeclarations = (params: any[]) => params
  .map((param: any) => `uniform ${param.type === 'color' ? 'vec3' : 'float'} u_${param.k};`)
  .join('\n');

export function install(PM: PMRegistry): void {
  PM.TRANSITION_PRE = `#version 300 es
precision highp float;
in vec2 v_uv; in vec2 v_st;
uniform sampler2D u_from, u_to;
uniform float u_prog;
uniform vec2 u_res, u_texel;
uniform float u_time;
out vec4 o;
vec4 src(vec2 uv){ return texture(u_from, uv); }
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

  const legacy = (definition: TransitionDefinition) => {
    if (definition.rawShader) {
      return { label: definition.label, group: definition.group || 'Transitions', params: definition.params, frag: definition.frag };
    }
    const declarations = paramDeclarations(definition.params);
    return {
      label: definition.label,
      group: definition.group || 'Transitions',
      params: definition.params,
      frag: `${PM.TRANSITION_PRE}\n${declarations}\nvoid main(){\n${definition.frag}\n}\n`,
    };
  };

  const kernel = ensureKernel(PM);
  /* Resolved per call, not captured: a later-installed kernel (or a test
     double) must win over the one this install created. */
  const kernelNow = (): any => (PM.Kernel as any) ?? kernel;
  type LegacyShape = ReturnType<typeof legacy>;
  const cache = new WeakMap<TransitionDefinition, LegacyShape>();
  const toLegacy = (definition: TransitionDefinition): LegacyShape => {
    const cached = cache.get(definition);
    if (cached) return cached;
    const convert = kernelNow()?.glsl?.toLegacyTransition;
    const converted: LegacyShape = convert ? convert(definition, PM.TRANSITION_PRE) : legacy(definition);
    cache.set(definition, converted);
    return converted;
  };

  kernel.transitions.onChange((change) => {
    if (change.kind === 'add') return;
    PM.GL?.dropProgram?.('tr:' + change.id);
  });

  PM.TRANSITIONS = registryView<TransitionDefinition, LegacyShape>(kernel.transitions, { read: (item) => toLegacy(item) });

  PM.transitionDef = (type: any) => {
    const definition = kernelNow()?.transitions?.get?.(type);
    return definition ? toLegacy(definition) : undefined;
  };

  PM.mkTransition = (type: any) => {
    const definition = PM.transitionDef(type);
    if (!definition) return null;
    const transition: any = { type, dur: 0.5, p: {} };
    definition.params.forEach((param: any) => { transition.p[param.k] = PM.P(param.def); });
    return transition;
  };
}
