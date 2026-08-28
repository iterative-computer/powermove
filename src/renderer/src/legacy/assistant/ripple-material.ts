import { defineMaterial } from '@motion-core/motion-gpu';

export const RIPPLE_FADE_START_SECONDS = 0.48;
export const RIPPLE_SETTLE_SECONDS = 1.45;

/**
 * WindowCapture pixels already match the live Electron surface. Keep the
 * captured channels unencoded and present them on an sRGB canvas so entering
 * change mode cannot reinterpret the interface as Display-P3.
 */
export const RIPPLE_COLOR_PIPELINE = {
  dynamicRange: 'auto',
  canvasColorSpace: 'srgb',
  outputEncoding: 'linear',
} as const;

export function rippleIntensity(elapsed: number): number {
  const progress = Math.max(0, Math.min(1,
    (elapsed - RIPPLE_FADE_START_SECONDS) / (RIPPLE_SETTLE_SECONDS - RIPPLE_FADE_START_SECONDS),
  ));
  return 1 - progress * progress * (3 - 2 * progress);
}

export function rippleFollow(current: number, target: number, deltaSeconds: number): number {
  const follow = 1 - Math.exp(-Math.min(0.05, Math.max(0, deltaSeconds)) * 18);
  return current + (target - current) * follow;
}

/** Powermove owns the look; Motion GPU owns its WebGPU lifecycle and resources. */
export const rippleMaterial = defineMaterial({
  fragment: `
fn frag(uv: vec2f) -> vec4f {
  let source = motiongpuUniforms.uOrigin / motiongpuFrame.resolution;
  let aspect = motiongpuFrame.resolution.x / motiongpuFrame.resolution.y;
  let delta = (uv - source) * vec2f(aspect, 1.0);
  let distanceFromSource = length(delta);
  // The cursor remains the emitter, while the wavefront is free to carry
  // beyond it and across the full window before the animation settles.
  let entrance = smoothstep(0.0, 0.11, motiongpuUniforms.uElapsed);
  let propagationFade = exp(-motiongpuUniforms.uElapsed * 0.38);
  let front = motiongpuUniforms.uElapsed * 1.32;

  // Wide, low-energy feedback bands make the deliberate shake legible
  // across the editor instead of reading as tiny lines near the cursor.
  let crest = exp(-pow((distanceFromSource - front) * 4.4, 2.0)) * propagationFade;
  let echo = exp(-pow((distanceFromSource - front + 0.28) * 6.2, 2.0)) * propagationFade;
  let wakeMask = smoothstep(front + 0.48, front - 0.2, distanceFromSource);
  let wake = (0.5 + 0.5 * sin(distanceFromSource * 34.0 - motiongpuUniforms.uElapsed * 12.0))
    * wakeMask * exp(-distanceFromSource * 1.2);
  let core = exp(-distanceFromSource * 5.4) * exp(-motiongpuUniforms.uElapsed * 0.74);
  let cursorLens = exp(-pow(distanceFromSource * 3.4, 2.0));
  let cursorRipple = sin(distanceFromSource * 38.0 - motiongpuUniforms.uElapsed * 17.0)
    * cursorLens * 0.006;
  let shimmer = 0.5 + 0.5 * cos(atan2(delta.y, delta.x) * 3.0 - motiongpuUniforms.uElapsed * 2.0);

  let ringAlpha = clamp((crest * 0.24 + echo * 0.08 + wake * 0.04 + core * 0.13)
    * motiongpuUniforms.uIntensity * entrance, 0.0, 0.42);
  let violet = vec3f(0.34, 0.18, 0.55);
  let hot = clamp(crest + core + shimmer * echo * 0.35, 0.0, 1.0);
  // Keep the defined displacement edge neutral; orange belongs only to
  // the very broad HDR haze below, never to a crisp ring.
  let ringColor = mix(violet, vec3f(0.92, 0.86, 0.82), hot * 0.24);

  // True radial displacement samples the captured interface around the crest,
  // with a restrained RGB split.
  let radialDirection = delta / max(distanceFromSource, 0.0001);
  let displacementStrength = (crest * 0.018 - echo * 0.006 + wake * 0.0015 + cursorRipple)
    * motiongpuUniforms.uIntensity * entrance;
  let displacement = radialDirection * displacementStrength;
  let displacedUV = clamp(uv - displacement, vec2f(0.001), vec2f(0.999));
  let red = textureSample(uScene, uSceneSampler,
    clamp(displacedUV - displacement * 0.08, vec2f(0.001), vec2f(0.999))).r;
  let green = textureSample(uScene, uSceneSampler, displacedUV).g;
  let blue = textureSample(uScene, uSceneSampler,
    clamp(displacedUV + displacement * 0.08, vec2f(0.001), vec2f(0.999))).b;
  let displacedScene = vec3f(red, green, blue);
  let sceneColor = displacedScene * (1.0 + crest * 0.012 * motiongpuUniforms.uIntensity);

  // Orange stays defocused in two broad HDR lobes with no sharp orange crest.
  let hdrBloomNear = exp(-pow((distanceFromSource - front) * 1.15, 2.0));
  let hdrBloomFar = exp(-pow((distanceFromSource - front) * 0.52, 2.0));
  let hdrCrest = (hdrBloomNear * 0.18 + hdrBloomFar * 0.045)
    * propagationFade * motiongpuUniforms.uIntensity * entrance;
  let hdrColor = vec3f(1.0, 0.72, 0.52);
  let composited = sceneColor + ringColor * ringAlpha + hdrColor * hdrCrest;
  let outputAlpha = mix(ringAlpha, 1.0, motiongpuUniforms.uHasScene);
  // Motion GPU premultiplies at final presentation, so the transparent path
  // remains straight-alpha while the captured-scene path is already opaque.
  let outputColor = mix(ringColor, composited, motiongpuUniforms.uHasScene);
  return vec4f(outputColor, outputAlpha);
}
`,
  uniforms: {
    uElapsed: 0,
    uHasScene: 0,
    uIntensity: 1,
    uOrigin: [0, 0],
  },
  textures: {
    uScene: {
      source: null,
      colorSpace: 'linear',
      flipY: true,
      update: 'once',
      filter: 'linear',
      addressModeU: 'clamp-to-edge',
      addressModeV: 'clamp-to-edge',
    },
  },
});
