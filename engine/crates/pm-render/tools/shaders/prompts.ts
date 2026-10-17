/*
 * Requests for the generated agent corpus of the shader gate (plan section
 * 11: "a generated corpus of agent-authored shaders (prompts covering every
 * contract feature, written by the same agent harness users use)").
 *
 * Each prompt names the contract features it is meant to reach (`covers`);
 * `generate.ts` sends it with the in-app agent's own instructions and
 * records what came back. `CONTRACT` lists every feature of the shader-layer
 * and extension-fragment rows (plus the effect and transition rows the
 * effects lane owns, for the agent-written ones); `generate.ts` fails when a
 * feature has no prompt.
 */
export type Kind = 'shader' | 'extension' | 'effect' | 'transition';
export interface Prompt { name: string; kind: Kind; covers: string[]; prompt: string }

export const CONTRACT = [
  // Shader layers and extension fragment layers (section 11).
  'iResolution', 'iTime', 'iGlobalTime', 'iProgress', 'iFrame', 'iMouse', 'uv', 'fragColor',
  'helper-hash', 'helper-noise', 'helper-fbm', 'helper-rot', 'helper-palette', 'helper-sdf', 'helper-hsv', 'helper-luma',
  'param-float', 'param-color', 'param-bool', 'param-vec2', 'param-vec4', 'param-int', 'param-no-annotation',
  'extension-number', 'extension-color', 'extension-toggle-bool',
  // Language features agents reach for in such programs.
  'alpha-output', 'hdr-output', 'discard', 'derivatives', 'gl_FragCoord', 'raymarch-loop', 'structs', 'arrays',
  'integer-bitwise', 'matrices', 'switch', 'out-params', 'defines', 'precision', 'early-return',
  'non-sampler2D-sampler', 'sampler2D-uniform',
  // Effect and transition rows (the effects lane's contract), agent-written.
  'effect-uniforms', 'effect-passes', 'effect-keepOrig', 'effect-backdrop', 'effect-rawShader', 'effect-toggle-float',
  'transition-uniforms',
] as const;

const s = (name: string, covers: string[], prompt: string): Prompt => ({ name, kind: 'shader', covers, prompt });
const x = (name: string, covers: string[], prompt: string): Prompt => ({ name, kind: 'extension', covers, prompt });
const e = (name: string, covers: string[], prompt: string): Prompt => ({ name, kind: 'effect', covers, prompt });
const t = (name: string, covers: string[], prompt: string): Prompt => ({ name, kind: 'transition', covers, prompt });

export const PROMPTS: Prompt[] = [
  s('aurora-ribbons', ['iTime', 'iResolution', 'fragColor', 'helper-fbm', 'param-float', 'param-color'],
    'Make a shader layer with slow aurora ribbons drifting across a dark sky. Give me sliders for speed and ribbon count and a color picker for the glow.'),
  s('progress-wipe-bar', ['iProgress', 'alpha-output', 'param-color'],
    'Shader layer: a thin loading bar along the bottom that fills from left to right over the layer\'s duration. Everything else must be transparent so it can sit over footage.'),
  s('frame-flicker', ['iFrame', 'helper-hash', 'integer-bitwise'],
    'Old film flicker as a shader layer: brightness should jump randomly once per frame (not smoothly), with some dust specks that change every frame.'),
  s('global-clock-rings', ['iGlobalTime', 'iTime', 'helper-sdf'],
    'Concentric rings pulsing outward. The pulse should stay in sync with the composition clock even if the layer starts later, but the fade-in should start when the layer starts.'),
  s('cursor-glow', ['iMouse', 'iResolution'],
    'A glow that follows the mouse cursor, Shadertoy style, over a subtle gradient background.'),
  s('voronoi-cells', ['helper-hash', 'uv', 'param-float', 'arrays'],
    'Animated Voronoi cells with dark borders, like cracked glass. Cell scale and border width as sliders.'),
  s('hsv-hue-sweep', ['helper-hsv', 'helper-luma', 'param-float'],
    'A full-frame rainbow that sweeps its hue over time; add a saturation slider and desaturate the bottom third using luminance.'),
  s('palette-tunnel', ['helper-palette', 'helper-rot', 'iTime'],
    'Infinite tunnel effect with a cosine color palette, rotating slowly. Speed slider.'),
  s('noise-terrain-contours', ['helper-noise', 'helper-fbm', 'derivatives'],
    'Topographic map look: contour lines over fbm noise terrain, with lines that stay one pixel thick at any resolution.'),
  s('sdf-badge', ['helper-sdf', 'alpha-output', 'param-color', 'param-float'],
    'A rounded badge shape with an outline and soft drop shadow, transparent background. Corner radius and outline width sliders, fill and outline colors.'),
  s('raymarched-blob', ['raymarch-loop', 'structs', 'iTime', 'out-params'],
    'Raymarched metaball blob that slowly morphs, lit with a key light and a rim light. Use a hit struct for the march result.'),
  s('pixel-grid-fragcoord', ['gl_FragCoord', 'integer-bitwise', 'param-int'],
    'Pixel-perfect checkerboard where each square is N actual pixels wide (an int parameter, default 8), with every 4th square highlighted.'),
  s('starfield-hdr', ['hdr-output', 'helper-hash', 'iTime'],
    'Starfield flying toward the camera. Make the brightest stars overbright (values above 1) so a glow effect can pick them up later.'),
  s('vec-param-offset', ['param-vec2', 'param-vec4', 'param-float', 'param-no-annotation'],
    'Shader layer of a soft spotlight whose center is a vec2 parameter and whose color is a vec4 parameter (rgba), plus a falloff slider. Also keep an unannotated uniform for a future control.'),
  s('toggle-invert-stripes', ['param-bool', 'param-float', 'defines'],
    'Diagonal stripes with a checkbox to invert the colors and a slider for stripe density. Use a #define for the stripe angle.'),
  s('dissolve-discard', ['discard', 'helper-noise', 'iProgress'],
    'A noise dissolve: the layer shows a warm gradient that burns away with a glowing edge as the layer progresses; dissolved pixels should be discarded.'),
  s('kaleidoscope', ['matrices', 'helper-rot', 'switch'],
    'Kaleidoscope pattern with a segments setting (3, 4, 6, or 8 — use a switch on it) and slow rotation.'),
  s('scanline-crt', ['precision', 'iResolution', 'gl_FragCoord'],
    'CRT test pattern: color bars with scanlines and a subtle vignette; declare mediump precision for the helper functions.'),
  s('early-out-vignette', ['early-return', 'param-float', 'alpha-output'],
    'Transparent vignette overlay: fully transparent in the middle (return early there for speed), darkening toward the edges. Strength and radius sliders.'),
  s('env-cubemap-sky', ['non-sampler2D-sampler', 'iTime'],
    'Sky gradient shader layer that also samples a samplerCube environment uniform for reflections (it can be empty for now), mixing it in at 20%.'),
  s('texture-uniform-tile', ['sampler2D-uniform', 'uv'],
    'Tile pattern that samples a sampler2D uniform called uPattern (unbound for now) and mixes it over a procedural base so it still looks good without a texture.'),
  s('lava-lamp', ['helper-fbm', 'helper-palette', 'param-color', 'iTime'],
    'Lava lamp: big warm blobs rising slowly; two color pickers for the blob and background colors.'),
  s('glitch-blocks', ['iFrame', 'helper-hash', 'helper-luma', 'param-float'],
    'Digital glitch blocks that shift horizontally and change every few frames, over a simple gradient. Intensity slider.'),
  s('countdown-ring', ['iProgress', 'helper-sdf', 'alpha-output', 'param-color'],
    'Circular countdown ring that empties as the layer plays, on a transparent background; ring color and thickness controls.'),

  x('orb-extension', ['extension-number', 'extension-color', 'extension-toggle-bool', 'uv'],
    'Write an extension that registers a programmable layer "Glow Orb" (fragment renderer): radius and softness sliders, a color, and a toggle to pulse with time.'),
  x('bars-equalizer', ['extension-number', 'extension-toggle-bool', 'iTime', 'helper-hash'],
    'Extension layer type: fake audio equalizer bars. Bar count and speed parameters, a toggle for mirrored bars.'),
  x('studio-floor', ['extension-color', 'raymarch-loop', 'extension-toggle-bool'],
    'Extension layer that renders a seamless studio backdrop (cyclorama) with a soft floor shadow; background color, light color, and a toggle for a grid on the floor.'),
  x('clock-face', ['extension-number', 'extension-color', 'iGlobalTime', 'helper-sdf'],
    'Extension layer: an analog clock face whose hands follow the composition time; color controls for the face and hands and a size slider.'),
  x('confetti-field', ['extension-number', 'iTime', 'helper-hash', 'arrays'],
    'Extension layer that draws falling confetti pieces; density and wind sliders.'),
  x('gradient-mesh', ['extension-color', 'extension-number', 'iProgress'],
    'Extension layer: a four-corner gradient mesh with four color parameters and a slider for how much it animates over the layer\'s progress.'),

  e('agent-chromatic-split', ['effect-uniforms', 'effect-toggle-float'],
    'Effect: RGB split that offsets red and blue channels by an amount in pixels with an angle; include a toggle to animate the split with time.'),
  e('agent-two-pass-blur', ['effect-passes', 'effect-uniforms'],
    'Effect: a two-pass separable box blur (horizontal then vertical) with a radius slider.'),
  e('agent-edge-glow', ['effect-keepOrig', 'effect-uniforms'],
    'Effect: neon edge glow — detect edges in a first pass, blur them in a second pass, and add them back over the original layer in a third.'),
  e('agent-frosted-glass', ['effect-backdrop', 'effect-uniforms'],
    'Effect for a shape layer that looks like frosted glass: it blurs and brightens whatever is beneath the layer inside the layer\'s alpha.'),
  e('agent-raw-duotone', ['effect-rawShader'],
    'Raw shader effect (write the complete GLSL program yourself) that maps luminance to two colors given as color params.'),
  t('agent-iris-wipe', ['transition-uniforms'],
    'Transition: an iris wipe — a circle growing from the center reveals the incoming layer, with a soft edge slider.'),
  t('agent-pixel-dissolve', ['transition-uniforms'],
    'Transition: a pixelated dissolve where blocks of the outgoing layer flip to the incoming one in random order, with a block size slider.'),
];
