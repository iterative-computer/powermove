import type { EffectDefinition, PowermoveAPI } from 'powermove';

export const definition: EffectDefinition = {
  id: 'softglow.effect', label: 'Soft Glow', group: 'Light & Shadow', passes: 1,
  params: [
    { k: 'threshold', label: 'Threshold', def: 0.6, min: 0, max: 1, step: 0.01 },
    { k: 'radius', label: 'Radius', def: 12, min: 0, max: 64, step: 0.5, unit: 'px' },
    { k: 'intensity', label: 'Intensity', def: 0.8, min: 0, max: 2, step: 0.01 }
  ],
  frag: `
    vec4 c = texture(u_tex, v_st);
    vec2 px = u_radius / u_res;
    vec3 sum = vec3(0.0);
    for (int i = 0; i < 8; i++) {
      float a = float(i) * 0.7853982;
      vec4 s = texture(u_tex, v_st + vec2(cos(a), sin(a)) * px);
      sum += s.rgb * smoothstep(u_threshold, 1.0, luma(s.rgb));
    }
    o = vec4(c.rgb + sum / 8.0 * u_intensity * c.a, c.a);
  `
};

export default function activate(api: PowermoveAPI) {
  api.effects.register(definition);
}
