import { defineMaterial } from '@motion-core/motion-gpu';

/**
 * The glow wraps the prompt the agent is answering: a halo held just off the
 * bubble's rounded edge, with light travelling around it. It says which request
 * is live without putting a word of placeholder prose on the panel.
 */
export const PROMPT_GLOW_PAD = 26;
/** Matches the bubble's CSS corner radius so the halo tracks its shape. */
export const PROMPT_GLOW_RADIUS = 14;
/** The bubble's tail: its bottom-right corner is nearly square, and a halo drawn
    at the full radius visibly lifts off the bubble there. */
export const PROMPT_GLOW_TAIL_RADIUS = 5;

/** The halo fades up when a run starts instead of snapping on. */
export const GLOW_ENTRANCE_SECONDS = 0.45;
/** One pulse per cycle — faster than the ghost's breath, still unhurried. */
export const GLOW_PULSE_SECONDS = 2.4;
/** The halo arrives mid-swell rather than at an empty trough. */
export const GLOW_PULSE_LEAD = GLOW_PULSE_SECONDS * 0.34;
/** How long the halo takes to fade once the run is done. */
export const GLOW_EXIT_MS = 520;
/**
 * The light theme multiplies rather than screens, so the same alpha lands as a
 * far heavier mark there. It is given less than the dark theme, not more: the
 * accent still reads on white, and never as a smudge over the panel.
 */
export const GLOW_LIGHT_GAIN = 0.75;

/** Synthesized light: authored linear, encoded once for the sRGB canvas. */
export const GLOW_COLOR_PIPELINE = {
  dynamicRange: 'sdr',
  canvasColorSpace: 'srgb',
  outputEncoding: 'srgb',
} as const;

export function glowEntrance(elapsed: number): number {
  const progress = Math.max(0, Math.min(1, elapsed / GLOW_ENTRANCE_SECONDS));
  return progress * progress * (3 - 2 * progress);
}

/** A quicker swell and a longer settle, the same shape the ghost field breathes. */
export function glowPulse(elapsed: number): number {
  const phase = (Math.max(0, elapsed) % GLOW_PULSE_SECONDS) / GLOW_PULSE_SECONDS;
  const rise = 0.36;
  const progress = phase < rise ? phase / rise : 1 - (phase - rise) / (1 - rise);
  return progress * progress * (3 - 2 * progress);
}

export function glowGain(dark: boolean): number {
  return dark ? 1 : GLOW_LIGHT_GAIN;
}

/** Device pixels, so the halo keeps its CSS thickness at any canvas density. */
export function glowPad(scale: number): number {
  return PROMPT_GLOW_PAD * Math.max(1, scale);
}

/** Device pixels, clamped so a one-line bubble never inverts its own corners. */
export function glowCornerRadius(width: number, height: number, scale: number, radius = PROMPT_GLOW_RADIUS): number {
  const inner = Math.min(width, height) - 2 * glowPad(scale);
  return Math.max(0, Math.min(radius * Math.max(1, scale), inner * 0.5));
}

/** Powermove owns the look; Motion GPU owns its WebGPU lifecycle and resources. */
export const promptGlowMaterial = defineMaterial({
  fragment: `
// The accent, and two neighbours of it: a deep ember the body sits in and a
// gold the moving crest lifts to. Authored linear, encoded once on presentation.
const GLOW_EMBER = vec3f(0.52, 0.05, 0.005);
const GLOW_ACCENT = vec3f(1.0, 0.147, 0.010);
const GLOW_GOLD = vec3f(1.0, 0.40, 0.06);

fn glowRoundedBox(point: vec2f, halfSize: vec2f, radius: f32) -> f32 {
  let corner = abs(point) - halfSize + radius;
  return length(max(corner, vec2f(0.0))) + min(max(corner.x, corner.y), 0.0) - radius;
}

fn frag(uv: vec2f) -> vec4f {
  let resolution = max(motiongpuFrame.resolution, vec2f(1.0));
  let elapsed = motiongpuUniforms.uElapsed;
  let pulse = motiongpuUniforms.uPulse;
  let entrance = motiongpuUniforms.uIntensity;
  let pad = motiongpuUniforms.uPad;

  // Motion GPU hands over y-up coordinates, so negative y is the lower half.
  let centered = (uv - vec2f(0.5)) * resolution;
  // The bubble's tail corner is nearly square. Carrying the full radius around
  // it lifts the halo off the very corner it is supposed to be hugging.
  let onTail = centered.x > 0.0 && centered.y < 0.0;
  let radius = select(motiongpuUniforms.uRadius, motiongpuUniforms.uTailRadius, onTail);
  // Negative inside the bubble, zero on its edge, positive out in the padding.
  let edge = glowRoundedBox(centered, resolution * 0.5 - vec2f(pad), radius);
  let outside = max(edge, 0.0);

  // Light, not an outline: one wide bloom carries almost all of it, and the
  // near term only firms up the first pixels so the bloom has somewhere to
  // start. Both taper to nothing before the layer's own edge, so a panel that
  // clips the halo never cuts it on a visible line.
  let taper = 1.0 - smoothstep(pad * 0.62, pad, outside);
  let bloom = exp(-outside / (pad * 0.40)) * taper;
  let contact = exp(-outside / (pad * 0.13)) * taper;

  // A sheen travelling diagonally across the halo. An angular sweep looks wrong
  // here: on a bubble far wider than it is tall, equal angles are wildly unequal
  // lengths of edge, so the light lingers at the ends and leaves the long sides
  // dark. Distance along the diagonal moves at one speed the whole way round.
  let along = (centered.x + centered.y * 0.9) / resolution.x;
  let sheen = 0.5 + 0.5 * sin(along * 5.6 - elapsed * 1.15);
  // The halo never breaks: the sheen rides on top of a lit floor.
  let sweep = mix(0.62, 1.0, sheen * sheen);

  // The bubble is opaque and sits above the halo, so nothing is spent inside it.
  let exterior = smoothstep(-1.5, 1.5, edge);
  let swell = mix(0.55, 1.0, pulse);
  let alpha = (bloom * 0.30 + contact * 0.16) * sweep * swell * exterior * entrance
    * motiongpuUniforms.uGain;

  // Ember in the body, the accent through the middle of the falloff, gold only
  // where the sheen crests — so the halo is the accent colour moving, not three
  // colours competing.
  var color = mix(GLOW_EMBER, GLOW_ACCENT, clamp(bloom * 1.3, 0.0, 1.0));
  color = mix(color, GLOW_GOLD, clamp((sheen - 0.55) * 1.8, 0.0, 1.0) * 0.75);

  // Motion GPU premultiplies at presentation, so the halo stays straight-alpha.
  return vec4f(color, clamp(alpha, 0.0, 0.62));
}
`,
  uniforms: {
    uElapsed: 0,
    uGain: 1,
    uIntensity: 0,
    uPad: PROMPT_GLOW_PAD,
    uPulse: 0,
    uRadius: PROMPT_GLOW_RADIUS,
    uTailRadius: PROMPT_GLOW_TAIL_RADIUS,
  },
});
