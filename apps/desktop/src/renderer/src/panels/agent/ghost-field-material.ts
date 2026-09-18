import { defineMaterial } from '@motion-core/motion-gpu';

/** The field fades up rather than snapping on when a run claims its panel. */
export const GHOST_ENTRANCE_SECONDS = 0.55;
/** One slow breath per cycle: generation feels alive, not urgent. */
export const GHOST_BREATH_SECONDS = 3.6;
/** The field arrives at the top of an inhale instead of at an empty trough. */
export const GHOST_BREATH_LEAD = GHOST_BREATH_SECONDS * 0.38;
/**
 * A wash this faint reads instantly on a near-black panel and not at all on a
 * white one, so the light theme is given the alpha it needs to be seen.
 */
export const GHOST_LIGHT_GAIN = 2.0;

/** Matches the ghost's CSS corner radius so the shader edge tracks the border. */
export const GHOST_CORNER_RADIUS = 12;

/**
 * The field is synthesized light rather than captured pixels, so it is authored
 * in linear space and encoded once for the sRGB canvas on presentation.
 */
export const GHOST_COLOR_PIPELINE = {
  dynamicRange: 'sdr',
  canvasColorSpace: 'srgb',
  outputEncoding: 'srgb',
} as const;

export function ghostEntrance(elapsed: number): number {
  const progress = Math.max(0, Math.min(1, elapsed / GHOST_ENTRANCE_SECONDS));
  return progress * progress * (3 - 2 * progress);
}

/**
 * A breath, not a blink: a quicker inhale and a longer exhale, so every layer
 * of the field swells and settles on one shared rhythm.
 */
export function ghostBreath(elapsed: number): number {
  const phase = (Math.max(0, elapsed) % GHOST_BREATH_SECONDS) / GHOST_BREATH_SECONDS;
  const inhale = 0.38;
  const progress = phase < inhale ? phase / inhale : 1 - (phase - inhale) / (1 - inhale);
  return progress * progress * (3 - 2 * progress);
}

export function ghostGain(dark: boolean): number {
  return dark ? 1 : GHOST_LIGHT_GAIN;
}

/** Device pixels, clamped so a short ghost never inverts its own corners. */
export function ghostCornerRadius(width: number, height: number, scale: number): number {
  const radius = GHOST_CORNER_RADIUS * Math.max(1, scale);
  return Math.max(0, Math.min(radius, Math.min(width, height) * 0.5));
}

/** Powermove owns the look; Motion GPU owns its WebGPU lifecycle and resources. */
export const ghostMaterial = defineMaterial({
  fragment: `
// The accent and its neighbours, linear: a deep ember for the body, the accent
// itself through the flow, gold on the crests, and a warm near-white haze that
// is the one tone allowed to leave the orange family, so the inner bloom does
// not flatten into the rest of it.
const GHOST_EMBER = vec3f(0.44, 0.04, 0.004);
const GHOST_ACCENT = vec3f(1.0, 0.147, 0.010);
const GHOST_GOLD = vec3f(1.0, 0.42, 0.07);
const GHOST_HAZE = vec3f(1.0, 0.82, 0.62);

fn ghostRoundedBox(point: vec2f, halfSize: vec2f, radius: f32) -> f32 {
  let corner = abs(point) - halfSize + radius;
  return length(max(corner, vec2f(0.0))) + min(max(corner.x, corner.y), 0.0) - radius;
}

fn ghostHash(cell: vec2f) -> f32 {
  return fract(sin(dot(cell, vec2f(127.1, 311.7))) * 43758.5453);
}

fn ghostNoise(point: vec2f) -> f32 {
  let cell = floor(point);
  let offset = fract(point);
  let weight = offset * offset * (3.0 - 2.0 * offset);
  return mix(
    mix(ghostHash(cell), ghostHash(cell + vec2f(1.0, 0.0)), weight.x),
    mix(ghostHash(cell + vec2f(0.0, 1.0)), ghostHash(cell + vec2f(1.0, 1.0)), weight.x),
    weight.y,
  );
}

// Three rotated octaves: enough structure to read as flow, cheap enough to run
// for the whole length of a generation next to the live editor.
fn ghostFbm(point: vec2f) -> f32 {
  let turn = mat2x2f(vec2f(0.80, 0.60), vec2f(-0.60, 0.80));
  var coord = point;
  var amplitude = 0.5;
  var total = 0.0;
  for (var octave = 0; octave < 3; octave = octave + 1) {
    total = total + amplitude * ghostNoise(coord);
    coord = turn * coord * 2.03;
    amplitude = amplitude * 0.5;
  }
  return total / 0.875;
}

// Domain warping folds the field through itself, so the bands curl and swim
// instead of scrolling in one visible direction.
fn ghostFlow(point: vec2f, elapsed: f32) -> f32 {
  let drift = vec2f(elapsed * 0.05, elapsed * -0.037);
  let warp = vec2f(
    ghostFbm(point + drift),
    ghostFbm(point + vec2f(4.3, 1.7) - drift),
  );
  return ghostFbm(point + warp * 1.7 + drift * 1.6);
}

fn frag(uv: vec2f) -> vec4f {
  let resolution = max(motiongpuFrame.resolution, vec2f(1.0));
  let aspect = resolution.x / resolution.y;
  let centered = (uv - vec2f(0.5)) * resolution;
  // Negative inside the ghost, zero on its border, positive outside.
  let edge = ghostRoundedBox(centered, resolution * 0.5 - vec2f(1.0), motiongpuUniforms.uRadius);
  let interior = 1.0 - smoothstep(-1.5, 0.0, edge);

  let elapsed = motiongpuUniforms.uElapsed;
  let breath = motiongpuUniforms.uBreath;
  let entrance = motiongpuUniforms.uIntensity;

  // Sampling in aspect-corrected panel space keeps the curl the same size in a
  // wide dock and a narrow inspector.
  let field = ghostFlow(vec2f(uv.x * aspect, uv.y) * 2.4, elapsed);
  // Ridges turn the smooth field into filaments that read as motion at a glance.
  let filament = pow(1.0 - abs(field * 2.0 - 1.0), 2.2);
  // A soft tide drifts down the panel, bent by the field it passes through.
  let tide = 0.5 + 0.5 * sin(uv.y * 3.4 - elapsed * 0.62 + field * 3.1);
  let flow = (filament * 0.82 + field * 0.18) * mix(0.30, 1.0, tide);

  // A hairline on the border, an inner bloom pooling behind it, and a short
  // outer haze so the ghost sits inside the layout instead of on top of it.
  let hairline = exp(-abs(edge) * 0.45);
  let innerBloom = exp(-max(-edge, 0.0) / 26.0) * interior;
  let outerHaze = exp(-max(edge, 0.0) / 7.0) * (1.0 - smoothstep(9.0, 18.0, edge));

  // The same travelling crest as the spatial ripple, wrapped around the border.
  let direction = centered / resolution;
  let travel = atan2(direction.y, direction.x) - elapsed * 0.42;
  let crest = pow(0.5 + 0.5 * cos(travel), 6.0);
  let echo = pow(0.5 + 0.5 * cos(travel + 1.7), 12.0) * 0.42;
  let sweep = crest + echo;

  let pulse = mix(0.55, 1.0, breath);
  let bodyAlpha = flow * interior * (0.045 + 0.07 * breath);
  let rimAlpha = hairline * (0.08 + sweep * 0.25 * pulse)
    + innerBloom * (0.025 + flow * 0.055) * pulse
    + outerHaze * sweep * 0.035 * pulse;

  // Ember carries the body, the accent runs through the flow, and gold lifts
  // the filaments and the travelling crest.
  var color = mix(GHOST_EMBER, GHOST_ACCENT, clamp(field * 0.9, 0.0, 1.0));
  color = mix(color, GHOST_GOLD, clamp(filament * 0.7 + sweep * 0.45, 0.0, 1.0));
  // The pale haze only ever appears as a broad defocused warmth behind the rim,
  // never as a crisp edge of its own.
  color = color + GHOST_HAZE * innerBloom * breath * flow * 0.08;

  let alpha = clamp((bodyAlpha + rimAlpha) * entrance * motiongpuUniforms.uGain, 0.0, 0.62);
  // Motion GPU premultiplies at presentation, so the field stays straight-alpha.
  return vec4f(color, alpha);
}
`,
  uniforms: {
    uBreath: 0,
    uElapsed: 0,
    uGain: 1,
    uIntensity: 0,
    uRadius: 12,
  },
});
