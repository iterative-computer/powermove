import { defineMaterial } from '@motion-core/motion-gpu';
import { GHOST_BREATH_LEAD, ghostBreath } from './ghost-field-material';
import { GLOW_PULSE_LEAD, glowEntrance, glowGain, glowPulse } from './prompt-glow-material';

/** Both existing activity rhythms, driven by one renderer's capped clock. */
export function atmosphereUniforms(elapsed: number, dark: boolean) {
  return {
    uElapsed: elapsed,
    uBreath: ghostBreath(elapsed + GHOST_BREATH_LEAD),
    uPulse: glowPulse(elapsed + GLOW_PULSE_LEAD),
    uIntensity: glowEntrance(elapsed),
    uGain: glowGain(dark),
  };
}

/** Ghost flow plus prompt sheen, folded into a small composition-plane mark. */
export const atmosphereMaterial = defineMaterial({
  fragment: `
// The ghost material's rotated noise and domain warp. Keep the same three
// octaves; the bounded emblem needs no higher-frequency detail.
fn atmosphereHash(cell: vec2f) -> f32 {
  return fract(sin(dot(cell, vec2f(127.1, 311.7))) * 43758.5453);
}
fn atmosphereNoise(point: vec2f) -> f32 {
  let cell = floor(point);
  let offset = fract(point);
  let weight = offset * offset * (3.0 - 2.0 * offset);
  return mix(
    mix(atmosphereHash(cell), atmosphereHash(cell + vec2f(1.0, 0.0)), weight.x),
    mix(atmosphereHash(cell + vec2f(0.0, 1.0)), atmosphereHash(cell + vec2f(1.0, 1.0)), weight.x),
    weight.y,
  );
}
fn atmosphereFbm(point: vec2f) -> f32 {
  let turn = mat2x2f(vec2f(0.80, 0.60), vec2f(-0.60, 0.80));
  var coord = point;
  var amplitude = 0.5;
  var total = 0.0;
  for (var octave = 0; octave < 3; octave = octave + 1) {
    total = total + amplitude * atmosphereNoise(coord);
    coord = turn * coord * 2.03;
    amplitude = amplitude * 0.5;
  }
  return total / 0.875;
}
fn atmosphereFlow(point: vec2f, elapsed: f32) -> f32 {
  let drift = vec2f(elapsed * 0.05, elapsed * -0.037);
  let warp = vec2f(
    atmosphereFbm(point + drift),
    atmosphereFbm(point + vec2f(4.3, 1.7) - drift),
  );
  return atmosphereFbm(point + warp * 1.7 + drift * 1.6);
}
fn frag(uv: vec2f) -> vec4f {
  let resolution = max(motiongpuFrame.resolution, vec2f(1.0));
  let elapsed = motiongpuUniforms.uElapsed;
  let centered = (uv - vec2f(0.5)) * resolution;
  let field = atmosphereFlow(vec2f(uv.x * resolution.x / resolution.y, uv.y) * 2.4, elapsed);
  let filament = pow(1.0 - abs(field * 2.0 - 1.0), 2.2);
  let tide = 0.5 + 0.5 * sin(uv.y * 3.4 - elapsed * 0.62 + field * 3.1);
  let flow = (filament * 0.82 + field * 0.18) * mix(0.30, 1.0, tide);

  // A rhombus matches the stacked composition planes above the field.
  let edge = (abs(uv.x - 0.5) + abs(uv.y - 0.5) * 0.8 - 0.35) * resolution.y;
  let interior = 1.0 - smoothstep(-1.0, 0.5, edge);
  let outside = max(edge, 0.0);
  let bloom = exp(-outside / 3.0) * (1.0 - smoothstep(4.0, 8.0, outside));
  let contact = exp(-abs(edge) / 0.9);

  // Prompt glow's diagonal travelling sheen, lit floor, and swell. Its sweep
  // stays spatially even on a wide, short mark instead of lingering at ends.
  let along = (centered.x + centered.y * 0.9) / resolution.x;
  let sheen = 0.5 + 0.5 * sin(along * 5.6 - elapsed * 1.15);
  let sweep = mix(0.62, 1.0, sheen * sheen);
  let swell = mix(0.55, 1.0, motiongpuUniforms.uPulse);
  let rim = (bloom * 0.12 + contact * 0.25) * sweep * swell;
  let body = flow * interior * (0.10 + 0.16 * motiongpuUniforms.uBreath);

  // Linear indigo and silver-cyan, encoded once by the shared SDR pipeline.
  var color = mix(vec3f(0.12, 0.14, 0.34), vec3f(0.22, 0.43, 0.50), clamp(flow + sheen * 0.3, 0.0, 1.0));
  color = mix(color, vec3f(0.46, 0.62, 0.67), contact * sheen * 0.35);
  let alpha = (body + rim) * motiongpuUniforms.uIntensity * motiongpuUniforms.uGain;
  return vec4f(color, clamp(alpha, 0.0, 0.48));
}
`,
  uniforms: atmosphereUniforms(0, true),
});
