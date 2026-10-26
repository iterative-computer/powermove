import type { EffectDefinition, EffectParamDefinition } from 'powermove';

// Independent implementations of Adobe's documented color operations. Keep
// these additive: changing the original duotone/color shaders changes old art.
const fragment = (uniforms: string, body: string) => `#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
${uniforms}
out vec4 o;
void main() {
  vec4 c = texture(u_tex, v_st);
  // Work in straight RGB, then restore the compositor's premultiplied alpha.
  if (c.a <= 0.) { o = vec4(0.); return; }
  vec3 rgb = c.rgb / c.a;
  ${body}
}`;

const coefficient = (k: string, label: string, def: number): EffectParamDefinition =>
  ({ k, label, def, group: label.split('-')[0], advanced: false, min: -200, max: 200, step: 1, unit: '%' });

export const COLOR_CORRECTION_EFFECTS: EffectDefinition[] = [
  {
    id: 'tint', label: 'Tint', group: 'Color', passes: 1,
    actions: [{ kind: 'swap', label: 'Swap Colors', keys: ['mapBlackTo', 'mapWhiteTo'] }],
    params: [
      { k: 'mapBlackTo', label: 'Map Black To', type: 'color', def: '#000000' },
      { k: 'mapWhiteTo', label: 'Map White To', type: 'color', def: '#FFFFFF' },
      { k: 'amountToTint', label: 'Amount to Tint', def: 100, min: 0, max: 100, step: 1, unit: '%' },
    ],
    frag: fragment('uniform vec3 u_mapBlackTo, u_mapWhiteTo; uniform float u_amountToTint;', `
      float luminance = dot(rgb, vec3(.2126, .7152, .0722));
      vec3 mapped = mix(u_mapBlackTo, u_mapWhiteTo, luminance);
      o = vec4(mix(rgb, mapped, u_amountToTint / 100.) * c.a, c.a);`),
    rawShader: true, viewportSafe: true, viewportPadding: [],
  },
  {
    id: 'channelMixer', label: 'Channel Mixer', group: 'Color', passes: 1,
    params: [
      coefficient('redRed', 'Red-Red', 100),
      coefficient('redGreen', 'Red-Green', 0),
      coefficient('redBlue', 'Red-Blue', 0),
      coefficient('redConst', 'Red-Const', 0),
      coefficient('greenRed', 'Green-Red', 0),
      coefficient('greenGreen', 'Green-Green', 100),
      coefficient('greenBlue', 'Green-Blue', 0),
      coefficient('greenConst', 'Green-Const', 0),
      coefficient('blueRed', 'Blue-Red', 0),
      coefficient('blueGreen', 'Blue-Green', 0),
      coefficient('blueBlue', 'Blue-Blue', 100),
      coefficient('blueConst', 'Blue-Const', 0),
      { k: 'monochrome', label: 'Monochrome', type: 'toggle', def: false, advanced: false },
    ],
    frag: fragment(`uniform float u_redRed, u_redGreen, u_redBlue, u_redConst;
      uniform float u_greenRed, u_greenGreen, u_greenBlue, u_greenConst;
      uniform float u_blueRed, u_blueGreen, u_blueBlue, u_blueConst;
      uniform float u_monochrome;`, `
      vec3 mixed = vec3(
        dot(rgb, vec3(u_redRed, u_redGreen, u_redBlue)) + u_redConst,
        dot(rgb, vec3(u_greenRed, u_greenGreen, u_greenBlue)) + u_greenConst,
        dot(rgb, vec3(u_blueRed, u_blueGreen, u_blueBlue)) + u_blueConst
      ) / 100.;
      if (u_monochrome > .5) mixed = vec3(mixed.r);
      o = vec4(clamp(mixed, 0., 1.) * c.a, c.a);`),
    rawShader: true, viewportSafe: true, viewportPadding: [],
  },
];
