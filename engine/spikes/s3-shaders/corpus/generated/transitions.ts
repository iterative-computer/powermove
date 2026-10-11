/*
 * Generated agent-style transitions (kernel `api.transitions.register`).
 */
import type { GeneratedShader } from './index';

const tr = (name: string, covers: string[], prompt: string, def: Record<string, any>): GeneratedShader => ({
  kind: 'transition', name, covers, prompt, def: { id: `gen.${name}`, label: name, params: [], ...def },
});

export const TRANSITIONS: GeneratedShader[] = [
  tr('noise-dissolve', ['transition', 'color', 'helpers'],
    'Dissolve transition with a glowing burn edge.', {
      params: [
        { k: 'scale', label: 'Scale', def: 6, min: 1, max: 30 },
        { k: 'edge', label: 'Edge', def: 0.06, min: 0, max: 0.3 },
        { k: 'burn', label: 'Burn', type: 'color', def: '#FF6A00' },
      ],
      frag: `
    float n = fbm(v_st * u_scale);
    float t = u_prog * (1.0 + u_edge);
    float m = smoothstep(t - u_edge, t, n);
    vec4 a = texture(u_from, v_st);
    vec4 b = texture(u_to, v_st);
    float glow = (1.0 - abs(n - t + u_edge * 0.5) / max(u_edge, 1e-3)) * step(0.001, u_prog) * step(u_prog, 0.999);
    o = mix(b, a, m) + vec4(u_burn, 1.0) * clamp(glow, 0.0, 1.0) * 0.8;`,
    }),

  tr('angled-wipe', ['transition', 'toggle'],
    'Soft wipe at an angle with a reverse checkbox.', {
      params: [
        { k: 'angle', label: 'Angle', def: 30, min: -180, max: 180 },
        { k: 'softness', label: 'Softness', def: 0.08, min: 0, max: 0.5 },
        { k: 'reverse', label: 'Reverse', type: 'toggle', def: false },
      ],
      frag: `
    float a = radians(u_angle);
    vec2 d = vec2(cos(a), sin(a));
    float x = dot(v_st - 0.5, d) / (abs(d.x) + abs(d.y)) + 0.5;
    if (u_reverse > 0.5) x = 1.0 - x;
    float p = u_prog * (1.0 + 2.0 * u_softness) - u_softness;
    float m = smoothstep(p - u_softness, p + u_softness, x);
    o = mix(texture(u_to, v_st), texture(u_from, v_st), m);`,
    }),

  tr('raw-circle-reveal', ['transition', 'rawShader', 'positionalAlias'],
    'Circle reveal transition; I already have the full shader, use it raw.', {
      rawShader: true,
      params: [{ k: 'feather', label: 'Feather', def: 0.05, min: 0, max: 0.3 }],
      frag: `#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_from, u_to;
uniform float u_prog;
uniform vec2 u_res;
uniform float u_p0;
out vec4 o;
void main() {
  vec2 p = (v_st - 0.5) * vec2(u_res.x / u_res.y, 1.0);
  float r = u_prog * length(vec2(u_res.x / u_res.y, 1.0)) * 0.5;
  float m = smoothstep(r, r + u_p0, length(p));
  o = mix(texture(u_to, v_st), texture(u_from, v_st), m);
}`,
    }),

  tr('pixelize-through', ['transition', 'dataDependentLoop'],
    'Pixelate out of the first clip and into the second.', {
      params: [{ k: 'maxCell', label: 'Max Cell', def: 32, min: 2, max: 96 }],
      frag: `
    float k = 1.0 - abs(u_prog * 2.0 - 1.0);
    float cell = max(1.0, floor(u_maxCell * k));
    vec2 px = v_st / u_texel;
    vec2 base = floor(px / cell) * cell;
    vec4 acc = vec4(0.0);
    float n = 0.0;
    int steps = int(min(cell, 8.0));
    for (int j = 0; j < steps; j++) {
      for (int i = 0; i < steps; i++) {
        vec2 q = (base + (vec2(float(i), float(j)) + 0.5) * cell / float(steps)) * u_texel;
        acc += u_prog < 0.5 ? texture(u_from, q) : texture(u_to, q);
        n += 1.0;
      }
    }
    o = acc / n;`,
    }),

  tr('glitch-slices', ['transition', 'time', 'helpers'],
    'Glitchy slice transition: horizontal bands jump between the clips.', {
      params: [{ k: 'bands', label: 'Bands', def: 18, min: 2, max: 60 }],
      frag: `
    float band = floor(v_st.y * u_bands);
    float r = hash(vec2(band, floor(u_time * 24.0)));
    float shift = (r - 0.5) * 0.2 * sin(u_prog * 3.14159265);
    vec2 p = vec2(v_st.x + shift, v_st.y);
    o = r < u_prog ? texture(u_to, p) : texture(u_from, p);`,
    }),
];
