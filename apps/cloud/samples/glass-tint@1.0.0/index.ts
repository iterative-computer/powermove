import type { PowermoveAPI } from 'powermove';

export default function activate(api: PowermoveAPI) {
  api.effects.register({
    id: `${api.id}.tint`, label: 'Glass Tint', group: 'Stylize',
    params: [
      { k: 'amount', label: 'Amount', def: 0.45, min: 0, max: 1, step: 0.01 },
      { k: 'tint', label: 'Tint colour', def: '#80bde8', type: 'color' },
      { k: 'grain', label: 'Grain', def: true, type: 'toggle' },
    ],
    frag: `
      vec4 source = texture(u_tex, v_st);
      float fleck = (noise(v_px * 0.72) - 0.5) * 0.07 * u_grain;
      vec3 colour = clamp(u_tint + vec3(fleck), 0.0, 1.0);
      o = vec4(mix(source.rgb, colour * source.a, u_amount), source.a);
    `,
  });
}
