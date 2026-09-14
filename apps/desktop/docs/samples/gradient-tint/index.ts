import type { EffectDefinition, PowermoveAPI } from 'powermove';

// Keep the complete definition available for validate_effect before staging.
export const definition: EffectDefinition = {
  id: 'gradient-tint', label: 'Gradient Tint', group: 'Stylize',
  params: [
    { k: 'startColor', label: 'Start Color', type: 'color', def: '#ff3366' },
    { k: 'endColor', label: 'End Color', type: 'color', def: '#3366ff' },
    { k: 'angle', label: 'Angle', def: 0, min: -180, max: 180, unit: '°' },
    { k: 'amount', label: 'Amount', def: 1, min: 0, max: 1, step: 0.01 }
  ],
  // Powermove supplies main(), uniforms and the output variable o.
  // Preserve premultiplied alpha, including transparent pixels around text.
  frag: `
    vec4 source = texture(u_tex, v_st);
    float angle = radians(u_angle);
    vec2 direction = vec2(cos(angle), sin(angle));
    float position = clamp(dot(v_st - 0.5, direction) + 0.5, 0.0, 1.0);
    vec3 tint = mix(u_startColor, u_endColor, position);
    o = vec4(mix(source.rgb, tint * source.a, u_amount), source.a);
  `
};

export default function activate(api: PowermoveAPI) {
  api.effects.register(definition);
  // Registration supplies Effects & Presets and keyframeable Inspector rows.
  // Applying the effect is a separate user/agent project edit, not activation.
}
