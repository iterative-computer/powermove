/*
 * Generated agent-style shader layers (Shadertoy-style `d.code` with `@param`
 * annotations) and extension fragment layers (`api.layers.register` with a
 * `fragment` renderer).
 */
import type { GeneratedShader } from './index';

const shader = (name: string, covers: string[], prompt: string, code: string): GeneratedShader => ({ kind: 'shader', name, covers, prompt, code });
const layer = (name: string, covers: string[], prompt: string, def: Record<string, any>): GeneratedShader => ({
  kind: 'extension', name, covers, prompt, def: { id: `gen.${name}`, label: name, version: 1, params: [], ...def },
});

export const SHADER_LAYERS: GeneratedShader[] = [
  shader('plasma-time-palette', ['shaderLayer', 'iTime', 'color', 'palette'],
    'Animated plasma background with a speed slider and a tint color.', `
uniform float uSpeed; // @param 0.8 0 3
uniform vec3 uTint; // @param #66CCFF
void main(){
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0) * 3.0;
  float t = iTime * uSpeed;
  float v = sin(p.x + t) + sin(p.y * 1.3 - t) + sin(length(p) * 2.0 + t * 0.7);
  vec3 col = palette(v * 0.25 + 0.5, vec3(0.5), vec3(0.5), vec3(1.0), vec3(0.0, 0.33, 0.67));
  fragColor = vec4(col * mix(vec3(1.0), uTint, 0.5), 1.0);
}`),

  shader('mouse-spotlight', ['shaderLayer', 'iMouse'],
    'Spotlight that follows the mouse like on Shadertoy.', `
uniform float uRadius; // @param 0.35 0.05 1
void main(){
  vec2 frag = uv * iResolution;
  vec2 m = iMouse.xy;
  float d = length(frag - m) / iResolution.y;
  float light = smoothstep(uRadius, 0.0, d);
  vec3 base = mix(vec3(0.05, 0.06, 0.1), vec3(0.2, 0.25, 0.4), uv.y);
  fragColor = vec4(base + light * vec3(1.0, 0.9, 0.7), 1.0);
}`),

  shader('frame-counter-stripes', ['shaderLayer', 'iFrame', 'iProgress'],
    'Stripes that step once per frame plus a progress bar along the bottom.', `
void main(){
  float stripe = mod(floor(uv.x * 24.0) + float(iFrame % 24), 2.0);
  vec3 col = mix(vec3(0.1), vec3(0.9, 0.3, 0.4), stripe);
  if (uv.y > 0.9 && uv.x < iProgress) col = vec3(1.0);
  fragColor = vec4(col, 1.0);
}`),

  shader('sdf-badge-toggle', ['shaderLayer', 'sdf', 'toggle', 'helpers'],
    'Rounded badge shape using the SDF helpers, with an outline-only toggle.', `
uniform bool uOutline; // @param false
uniform float uAngle; // @param 0.3 -3.14 3.14
uniform vec3 uFill; // @param #FFB000
void main(){
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);
  p = rot(uAngle) * p;
  float d = sdBox(p, vec2(0.3, 0.14)) - 0.04;
  d = min(d, sdCircle(p - vec2(0.3, 0.0), 0.09));
  float aa = 1.5 / iResolution.y;
  float shape = uOutline ? 1.0 - smoothstep(0.0, aa, abs(d) - 0.01) : 1.0 - smoothstep(-aa, aa, d);
  fragColor = vec4(uFill * shape, shape);
}`),

  shader('int-and-vec2-params', ['shaderLayer', 'intParamStaysZero', 'vec2ParamStaysZero'],
    'Repeating rings with a ring count and a center offset as parameters.', `
uniform int uCount; // @param 5 1 12
uniform vec2 uOffset; // @param 0.2 -1 1
uniform vec4 uExtra; // @param 0.5
uniform float uWidth; // @param 0.02 0.001 0.1
void main(){
  vec2 p = uv - 0.5 - uOffset;
  float n = float(max(uCount, 1));
  float r = fract(length(p) * n * 2.0);
  float ring = smoothstep(uWidth * n, 0.0, abs(r - 0.5) - 0.25);
  fragColor = vec4(vec3(ring) + uExtra.rgb * 0.1, 1.0);
}`),

  shader('raymarch-sphere', ['shaderLayer', 'dataDependentLoop', 'iTime'],
    'Ray-marched shiny sphere rotating over time.', `
uniform float uSpin; // @param 1 0 4
float map(vec3 p){
  vec3 q = p;
  q.xz = rot(iTime * uSpin) * q.xz;
  return min(length(q) - 1.0, p.y + 1.2) + 0.02 * sin(q.x * 10.0) * sin(q.y * 10.0);
}
vec3 normalAt(vec3 p){
  vec2 e = vec2(1e-3, 0.0);
  return normalize(vec3(map(p + e.xyy) - map(p - e.xyy), map(p + e.yxy) - map(p - e.yxy), map(p + e.yyx) - map(p - e.yyx)));
}
void main(){
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0) * 2.0;
  vec3 ro = vec3(0.0, 0.2, 3.5);
  vec3 rd = normalize(vec3(p, -1.8));
  float t = 0.0;
  bool hit = false;
  for (int i = 0; i < 96; i++) {
    float d = map(ro + rd * t);
    if (d < 1e-3) { hit = true; break; }
    t += d;
    if (t > 20.0) break;
  }
  vec3 col = vec3(0.02, 0.03, 0.06) + 0.1 * uv.y;
  if (hit) {
    vec3 pos = ro + rd * t;
    vec3 n = normalAt(pos);
    vec3 l = normalize(vec3(0.6, 0.8, 0.4));
    float diff = max(dot(n, l), 0.0);
    float spec = pow(max(dot(reflect(-l, n), -rd), 0.0), 32.0);
    col = vec3(0.2, 0.4, 0.9) * diff + vec3(spec) + 0.05;
  }
  fragColor = vec4(col, 1.0);
}`),

  shader('fbm-clouds-global-time', ['shaderLayer', 'iGlobalTime', 'helpers'],
    'Drifting clouds tied to the composition time so they line up across cuts.', `
uniform float uCover; // @param 0.5 0 1
void main(){
  vec2 p = uv * vec2(iResolution.x / iResolution.y, 1.0) * 3.0 + vec2(iGlobalTime * 0.05, 0.0);
  float c = fbm(p + fbm(p + iGlobalTime * 0.02));
  float cloud = smoothstep(1.0 - uCover, 1.0 - uCover + 0.3, c);
  vec3 sky = mix(vec3(0.35, 0.55, 0.9), vec3(0.75, 0.85, 1.0), uv.y);
  fragColor = vec4(mix(sky, vec3(1.0), cloud), 1.0);
}`),

  shader('derivative-grid', ['shaderLayer', 'dFdy', 'dFdx', 'fwidth'],
    'Antialiased graph-paper grid whose lines stay one pixel wide at any size.', `
uniform float uCells; // @param 12 2 48
uniform vec3 uLine; // @param #2A9DFF
void main(){
  vec2 g = uv * vec2(uCells * iResolution.x / iResolution.y, uCells);
  vec2 w = fwidth(g);
  vec2 f = abs(fract(g - 0.5) - 0.5) / w;
  float line = 1.0 - min(min(f.x, f.y), 1.0);
  float slope = dFdy(g.y) > 0.0 ? 1.0 : 0.0;
  float tilt = abs(dFdx(g.x));
  fragColor = vec4(mix(vec3(0.97), uLine, line) * (0.9 + 0.1 * slope) + vec3(tilt * 0.0), 1.0);
}`),
];

export const EXTENSION_LAYERS: GeneratedShader[] = [
  layer('pulse-orb', ['extensionLayer', 'toggle', 'iTime', 'color'],
    'An extension layer that draws a glowing orb with a pulse checkbox.', {
      params: [
        { k: 'radius', label: 'Radius', def: 0.25, min: 0.05, max: 0.5 },
        { k: 'glow', label: 'Glow', type: 'color', def: '#7C5CFF' },
        { k: 'pulse', label: 'Pulse', type: 'toggle', def: true },
      ],
      renderer: { kind: 'fragment', fragment: `
void main(){
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);
  float r = u_radius * (u_pulse ? 1.0 + 0.1 * sin(iTime * 4.0) : 1.0);
  float d = length(p);
  float core = smoothstep(r, r - 0.01, d);
  float halo = exp(-max(d - r, 0.0) * 12.0) * 0.8;
  vec3 col = u_glow * (core + halo);
  fragColor = vec4(col, clamp(core + halo, 0.0, 1.0));
}` },
    }),

  layer('progress-ring', ['extensionLayer', 'iProgress', 'iMouse', 'color'],
    'Circular progress ring layer driven by the layer progress, with a mouse-reactive highlight.', {
      params: [
        { k: 'thickness', label: 'Thickness', def: 0.04, min: 0.005, max: 0.2 },
        { k: 'color', label: 'Color', type: 'color', def: '#30D158' },
      ],
      renderer: { kind: 'fragment', fragment: `
void main(){
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);
  float a = atan(p.y, p.x) / 6.2831853 + 0.5;
  float ring = smoothstep(u_thickness, u_thickness - 0.005, abs(length(p) - 0.3));
  float filled = step(a, iProgress);
  float hover = 1.0 - smoothstep(0.0, 0.2, length(uv * iResolution - iMouse) / iResolution.y);
  vec3 col = mix(vec3(0.2), u_color, filled) + hover * 0.1;
  fragColor = vec4(col * ring, ring);
}` },
    }),

  layer('starfield-layers', ['extensionLayer', 'dataDependentLoop', 'helpers', 'iTime'],
    'Parallax starfield layer where the number of depth layers is a slider.', {
      params: [
        { k: 'layers', label: 'Layers', def: 4, min: 1, max: 8, step: 1 },
        { k: 'speed', label: 'Speed', def: 0.3, min: 0, max: 2 },
      ],
      renderer: { kind: 'fragment', fragment: `
void main(){
  vec2 p = uv * vec2(iResolution.x / iResolution.y, 1.0);
  vec3 col = vec3(0.0, 0.0, 0.02);
  int n = int(u_layers);
  for (int i = 0; i < n; i++) {
    float depth = float(i + 1);
    vec2 q = p * (8.0 * depth) + vec2(iTime * u_speed * depth, hash1(depth) * 10.0);
    vec2 cell = floor(q);
    vec2 f = fract(q) - 0.5;
    float h = hash(cell + depth);
    if (h > 0.92) {
      float s = smoothstep(0.08, 0.0, length(f - (vec2(hash(cell * 1.7), hash(cell * 2.3)) - 0.5) * 0.6));
      col += vec3(s) / depth;
    }
  }
  fragColor = vec4(col, 1.0);
}` },
    }),

  layer('bevel-height', ['extensionLayer', 'dFdy', 'dFdx', 'sdf'],
    'Beveled button layer lit from the top using derivatives of a height field.', {
      params: [
        { k: 'bevel', label: 'Bevel', def: 0.05, min: 0.005, max: 0.2 },
        { k: 'base', label: 'Base', type: 'color', def: '#3A7BFF' },
        { k: 'flat', label: 'Flat', type: 'toggle', def: false },
      ],
      renderer: { kind: 'fragment', fragment: `
void main(){
  vec2 p = (uv - 0.5) * vec2(iResolution.x / iResolution.y, 1.0);
  float d = sdBox(p, vec2(0.35, 0.18)) - 0.05;
  float h = clamp(-d / u_bevel, 0.0, 1.0);
  vec3 n = normalize(vec3(-dFdx(h) * 40.0, -dFdy(h) * 40.0, 1.0));
  float light = u_flat ? 1.0 : clamp(dot(n, normalize(vec3(-0.3, 0.6, 0.75))), 0.0, 1.0);
  float a = 1.0 - smoothstep(0.0, 2.0 / iResolution.y, d);
  fragColor = vec4(u_base * (0.4 + 0.7 * light) * a, a);
}` },
    }),
];
