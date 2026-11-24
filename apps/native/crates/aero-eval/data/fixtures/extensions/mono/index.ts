import type { EffectDefinition, PowermoveAPI } from 'powermove';

export const definition: EffectDefinition = {
  id: 'mono.effect', label: 'Mono', group: 'Color',
  params: [{ k: 'amount', label: 'Amount', def: 1, min: 0, max: 1, step: 0.01 }],
  frag: `
    vec4 c = texture(u_tex, v_st);
    float y = dot(c.rgb, vec3(0.2126, 0.7152, 0.0722));
    vec3 mono = vec3(c.a - y);
    o = vec4(mix(c.rgb, mono, u_amount), c.a);
  `
};

export default function activate(api: PowermoveAPI) {
  api.effects.register(definition);
}
