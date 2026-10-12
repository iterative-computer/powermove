/*
 * Generated agent-style effects (kernel `api.effects.register` definitions).
 * Each entry records the request it answers; see README.md for provenance.
 */
import type { GeneratedShader } from './index';

const fx = (name: string, covers: string[], prompt: string, def: Record<string, any>): GeneratedShader => ({
  kind: 'effect', name, covers, prompt, def: { id: `gen.${name}`, label: name, group: 'Generated', params: [], ...def },
});

export const EFFECTS: GeneratedShader[] = [
  fx('crt-scanlines', ['toggle', 'time', 'helpers'],
    'Make a CRT effect with scanlines, a slight flicker, and a toggle for barrel curvature.', {
      params: [
        { k: 'lines', label: 'Lines', def: 220, min: 40, max: 600 },
        { k: 'strength', label: 'Strength', def: 0.35, min: 0, max: 1 },
        { k: 'curve', label: 'Curvature', type: 'toggle', def: true },
      ],
      frag: `
    vec2 uv = v_st;
    if (u_curve > 0.5) {
      vec2 c = uv * 2.0 - 1.0;
      c *= 1.0 + 0.08 * dot(c.yx, c.yx);
      uv = c * 0.5 + 0.5;
    }
    vec4 c = src(uv);
    if (uv.x < 0.0 || uv.x > 1.0 || uv.y < 0.0 || uv.y > 1.0) c = vec4(0.0);
    float scan = 0.5 + 0.5 * sin(uv.y * u_lines * 3.14159265);
    float flicker = 0.97 + 0.03 * sin(u_time * 60.0);
    c.rgb *= mix(1.0, scan, u_strength) * flicker;
    o = c;`,
    }),

  fx('two-pass-gaussian', ['multipass', 'loop'],
    'Write a separable Gaussian blur that does horizontal then vertical in two passes.', {
      passes: 2,
      params: [{ k: 'radius', label: 'Radius', def: 6, min: 0, max: 24 }],
      frag: `
    vec2 dir = u_pass == 0 ? vec2(u_texel.x, 0.0) : vec2(0.0, u_texel.y);
    float sigma = max(u_radius, 0.001) * 0.5;
    vec4 acc = vec4(0.0);
    float wsum = 0.0;
    for (int i = -12; i <= 12; i++) {
      float x = float(i) * max(u_radius, 0.0) / 12.0;
      float w = exp(-0.5 * x * x / (sigma * sigma));
      acc += texture(u_tex, v_st + dir * x) * w;
      wsum += w;
    }
    o = acc / wsum;`,
    }),

  fx('bloom-keep-original', ['keepOrig', 'multipass', 'loop'],
    'Add a bloom: threshold the bright parts, blur them over two passes, and add them back on top of the original.', {
      passes: 3, keepOrig: true,
      params: [
        { k: 'threshold', label: 'Threshold', def: 0.6, min: 0, max: 1 },
        { k: 'intensity', label: 'Intensity', def: 1.2, min: 0, max: 4 },
        { k: 'spread', label: 'Spread', def: 8, min: 1, max: 32 },
      ],
      frag: `
    if (u_pass == 0) {
      vec4 c = texture(u_tex, v_st);
      vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
      float k = smoothstep(u_threshold, u_threshold + 0.1, luma(rgb));
      o = vec4(rgb * k, 1.0) * c.a;
      return;
    }
    vec2 dir = u_pass == 1 ? vec2(u_texel.x, 0.0) : vec2(0.0, u_texel.y);
    vec4 acc = vec4(0.0);
    float wsum = 0.0;
    for (int i = -8; i <= 8; i++) {
      float w = exp(-float(i * i) / 18.0);
      acc += texture(u_tex, v_st + dir * float(i) * u_spread / 8.0) * w;
      wsum += w;
    }
    vec4 glow = acc / wsum;
    if (u_pass == 1) { o = glow; return; }
    vec4 base = texture(u_orig, v_st);
    o = vec4(base.rgb + glow.rgb * u_intensity, max(base.a, min(glow.a * u_intensity, 1.0)));`,
    }),

  fx('frosted-glass', ['backdrop', 'loop', 'helpers'],
    'I want a frosted glass card: blur whatever is behind the layer inside the layer shape and tint it.', {
      backdrop: true,
      params: [
        { k: 'blur', label: 'Blur', def: 10, min: 0, max: 40 },
        { k: 'tint', label: 'Tint', type: 'color', def: '#E8F0FF' },
        { k: 'tintAmount', label: 'Tint Amount', def: 0.25, min: 0, max: 1 },
      ],
      frag: `
    vec4 shape = texture(u_tex, v_st);
    vec2 bd = vec2(v_uv.x, 1.0 - v_uv.y);
    vec3 acc = vec3(0.0);
    float wsum = 0.0;
    for (int i = 0; i < 24; i++) {
      float a = float(i) * 2.39996323;
      float r = sqrt((float(i) + 0.5) / 24.0) * u_blur;
      vec2 off = vec2(cos(a), sin(a)) * r / u_coordRes;
      float w = exp(-2.0 * r * r / max(u_blur * u_blur, 1e-3));
      acc += texture(u_backdrop, bd + off).rgb * w;
      wsum += w;
    }
    vec3 glass = mix(acc / wsum, u_tint, u_tintAmount);
    o = vec4(glass * shape.a, shape.a);`,
    }),

  fx('backdrop-refraction', ['backdrop', 'keepOrig', 'multipass'],
    'Make the layer look like a lens that refracts the background; first pass computes an edge map, second pass bends the backdrop.', {
      passes: 2, keepOrig: true, backdrop: true,
      params: [{ k: 'strength', label: 'Strength', def: 18, min: 0, max: 60 }],
      frag: `
    if (u_pass == 0) {
      float l = texture(u_tex, v_st - vec2(u_texel.x, 0.0)).a;
      float r = texture(u_tex, v_st + vec2(u_texel.x, 0.0)).a;
      float d = texture(u_tex, v_st - vec2(0.0, u_texel.y)).a;
      float u = texture(u_tex, v_st + vec2(0.0, u_texel.y)).a;
      o = vec4(0.5 + 0.5 * (r - l), 0.5 + 0.5 * (u - d), texture(u_backdrop, vec2(v_uv.x, 1.0 - v_uv.y)).g, 1.0);
      return;
    }
    vec2 n = texture(u_tex, v_st).xy * 2.0 - 1.0;
    vec4 shape = texture(u_orig, v_st);
    vec2 bd = vec2(v_uv.x, 1.0 - v_uv.y) - n * u_strength / u_coordRes;
    vec3 bg = texture(u_backdrop, bd).rgb;
    o = vec4(mix(bg, shape.rgb / max(shape.a, 1e-4), 0.15) * shape.a, shape.a);`,
    }),

  fx('edge-glow-derivatives', ['dFdx', 'dFdy', 'fwidth', 'helpers'],
    'Outline the edges of the layer with a glow using screen-space derivatives instead of extra texture taps.', {
      params: [
        { k: 'width', label: 'Width', def: 2, min: 0.5, max: 8 },
        { k: 'color', label: 'Color', type: 'color', def: '#38E1FF' },
      ],
      frag: `
    vec4 c = src(v_st);
    float a = c.a;
    float g = length(vec2(dFdx(a), dFdy(a)));
    float edge = smoothstep(0.0, 0.5, g * u_width);
    float w = fwidth(luma(c.rgb));
    o = vec4(c.rgb + u_color * edge + vec3(w), max(a, edge));`,
    }),

  fx('emboss-dfdy-sign', ['dFdy', 'dFdx'],
    'Emboss the layer so edges facing up are lit and edges facing down are dark.', {
      params: [{ k: 'depth', label: 'Depth', def: 4, min: 0, max: 16 }],
      frag: `
    vec4 c = texture(u_tex, v_st);
    float h = luma(c.rgb) * c.a;
    // Positive dFdy means "brighter above" in GL's bottom-left window space.
    float lit = 0.5 + dFdy(h) * u_depth - dFdx(h) * u_depth * 0.5;
    float y = dFdy(v_px.y);
    o = vec4(vec3(lit) * c.a + vec3(0.0, 0.0, step(0.0, y) * 0.05), c.a);`,
    }),

  fx('ray-march-shadow', ['dataDependentLoop', 'loop'],
    'Cast a long shadow from the layer: march away from the light until you hit something opaque.', {
      params: [
        { k: 'angle', label: 'Angle', def: 35, min: -180, max: 180 },
        { k: 'length', label: 'Length', def: 40, min: 0, max: 120 },
        { k: 'threshold', label: 'Threshold', def: 0.5, min: 0.05, max: 1 },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    float a = radians(u_angle);
    vec2 dir = vec2(cos(a), sin(a)) * u_texel;
    vec2 p = v_st;
    float hit = 0.0;
    float travelled = 0.0;
    for (int i = 0; i < 128; i++) {
      p += dir;
      travelled += 1.0;
      if (travelled > u_length) break;
      if (texture(u_tex, p).a > u_threshold) { hit = 1.0 - travelled / max(u_length, 1.0); break; }
    }
    vec4 shadow = vec4(0.0, 0.0, 0.0, 0.6 * hit);
    o = c + shadow * (1.0 - c.a);`,
    }),

  fx('adaptive-sample-blur', ['dataDependentLoop', 'loop'],
    'Box blur where the number of samples is a slider.', {
      params: [{ k: 'samples', label: 'Samples', def: 9, min: 1, max: 31, step: 1 }],
      frag: `
    int n = int(clamp(u_samples, 1.0, 31.0));
    vec4 acc = vec4(0.0);
    for (int i = 0; i < n; i++) {
      float t = float(i) - float(n - 1) * 0.5;
      acc += texture(u_tex, v_st + vec2(t * u_texel.x, t * 0.5 * u_texel.y));
    }
    o = acc / float(n);`,
    }),

  fx('kuwahara-paint', ['dataDependentLoop', 'loop', 'array'],
    'Painterly Kuwahara filter with an adjustable radius.', {
      params: [{ k: 'radius', label: 'Radius', def: 3, min: 1, max: 6, step: 1 }],
      frag: `
    int r = int(u_radius);
    vec3 mean[4];
    vec3 sq[4];
    for (int k = 0; k < 4; k++) { mean[k] = vec3(0.0); sq[k] = vec3(0.0); }
    float count = float((r + 1) * (r + 1));
    for (int j = -r; j <= 0; j++) {
      for (int i = -r; i <= 0; i++) {
        vec3 c0 = src(v_st + vec2(float(i), float(j)) * u_texel).rgb;
        vec3 c1 = src(v_st + vec2(float(-i), float(j)) * u_texel).rgb;
        vec3 c2 = src(v_st + vec2(float(i), float(-j)) * u_texel).rgb;
        vec3 c3 = src(v_st + vec2(float(-i), float(-j)) * u_texel).rgb;
        mean[0] += c0; sq[0] += c0 * c0;
        mean[1] += c1; sq[1] += c1 * c1;
        mean[2] += c2; sq[2] += c2 * c2;
        mean[3] += c3; sq[3] += c3 * c3;
      }
    }
    float best = 1e9;
    vec3 outc = vec3(0.0);
    for (int k = 0; k < 4; k++) {
      vec3 m = mean[k] / count;
      vec3 v = abs(sq[k] / count - m * m);
      float s = v.r + v.g + v.b;
      if (s < best) { best = s; outc = m; }
    }
    o = vec4(outc, src(v_st).a);`,
    }),

  fx('positional-alias-body', ['positionalAlias', 'color'],
    'Port this old shader body that uses u_p0, u_p1 and u_c2 to the new effect API without renaming anything.', {
      params: [
        { k: 'gain', label: 'Gain', def: 1.2, min: 0, max: 3 },
        { k: 'bias', label: 'Bias', def: 0.05, min: -0.5, max: 0.5 },
        { k: 'shadow', label: 'Shadow', type: 'color', def: '#202040' },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    rgb = rgb * u_p0 + u_p1;
    rgb = mix(u_c2, rgb, smoothstep(0.0, 0.4, luma(rgb)));
    o = vec4(clamp(rgb, 0.0, 1.0) * c.a, c.a);`,
    }),

  fx('radial-zoom-full-frame', ['viewportSafeFalse', 'loop'],
    'Zoom blur from the center of the composition.', {
      viewportSafe: false,
      params: [{ k: 'amount', label: 'Amount', def: 0.15, min: 0, max: 0.5 }],
      frag: `
    vec2 center = vec2(0.5);
    vec2 d = v_st - center;
    vec4 acc = vec4(0.0);
    for (int i = 0; i < 24; i++) {
      float s = 1.0 - u_amount * float(i) / 23.0;
      acc += texture(u_tex, center + d * s);
    }
    o = acc / 24.0;`,
    }),

  fx('pixel-vignette', ['viewportSafe', 'coordinates'],
    'Vignette that stays correct when I zoom into the viewer (use composition pixels).', {
      viewportSafe: true,
      params: [
        { k: 'size', label: 'Size', def: 0.6, min: 0.1, max: 1.5 },
        { k: 'soft', label: 'Softness', def: 0.4, min: 0.01, max: 1 },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec2 p = v_px / u_coordRes - 0.5;
    p.x *= u_coordRes.x / u_coordRes.y;
    float v = smoothstep(u_size, u_size - u_soft, length(p));
    o = vec4(c.rgb * v, c.a);`,
    }),

  fx('hue-shift-invert', ['toggle', 'helpers'],
    'Hue shift and saturation with a checkbox that inverts the colors.', {
      params: [
        { k: 'hue', label: 'Hue', def: 0.2, min: -1, max: 1 },
        { k: 'saturation', label: 'Saturation', def: 1.3, min: 0, max: 3 },
        { k: 'invert', label: 'Invert', type: 'toggle', def: false },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    vec3 hsv = rgb2hsv(rgb);
    hsv.x = fract(hsv.x + u_hue);
    hsv.y = clamp(hsv.y * u_saturation, 0.0, 1.0);
    rgb = hsv2rgb(hsv);
    if (u_invert > 0.5) rgb = 1.0 - rgb;
    o = vec4(rgb * c.a, c.a);`,
    }),

  fx('duotone-map', ['color', 'helpers'],
    'Duotone with a shadow color and a highlight color.', {
      params: [
        { k: 'dark', label: 'Dark', type: 'color', def: '#1B1464' },
        { k: 'light', label: 'Light', type: 'color', def: '#FFC312' },
        { k: 'contrast', label: 'Contrast', def: 1.1, min: 0.5, max: 2 },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    float l = clamp((luma(rgb) - 0.5) * u_contrast + 0.5, 0.0, 1.0);
    o = vec4(mix(u_dark, u_light, l) * c.a, c.a);`,
    }),

  fx('rgb-split-texel-fetch', ['textureSize', 'texelFetch'],
    'RGB split that offsets channels by whole pixels using texelFetch.', {
      params: [{ k: 'offset', label: 'Offset', def: 4, min: 0, max: 20, step: 1 }],
      frag: `
    ivec2 size = textureSize(u_tex, 0);
    ivec2 p = ivec2(v_st * vec2(size));
    int d = int(u_offset);
    ivec2 lo = ivec2(0);
    ivec2 hi = size - 1;
    vec4 r = texelFetch(u_tex, clamp(p + ivec2(d, 0), lo, hi), 0);
    vec4 g = texelFetch(u_tex, clamp(p, lo, hi), 0);
    vec4 b = texelFetch(u_tex, clamp(p - ivec2(d, 0), lo, hi), 0);
    o = vec4(r.r, g.g, b.b, max(max(r.a, g.a), b.a));`,
    }),

  fx('luma-pixel-sort', ['whileLoop', 'dataDependentLoop'],
    'Fake pixel sorting: smear bright pixels downward until you reach a darker one.', {
      params: [
        { k: 'threshold', label: 'Threshold', def: 0.55, min: 0, max: 1 },
        { k: 'maxRun', label: 'Max Run', def: 40, min: 1, max: 100 },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec2 p = v_st;
    int steps = 0;
    while (steps < int(u_maxRun)) {
      vec4 n = texture(u_tex, p + vec2(0.0, u_texel.y));
      if (luma(n.rgb) < u_threshold) break;
      p.y += u_texel.y;
      steps++;
    }
    vec4 s = texture(u_tex, p);
    o = mix(c, s, float(steps) / max(u_maxRun, 1.0));`,
    }),

  fx('halftone-fragcoord', ['gl_FragCoord', 'helpers'],
    'Halftone dots aligned to the screen pixel grid.', {
      params: [{ k: 'cell', label: 'Cell', def: 6, min: 2, max: 24 }],
      frag: `
    vec2 cell = floor(gl_FragCoord.xy / u_cell);
    vec2 center = (cell + 0.5) * u_cell;
    vec4 c = texture(u_tex, center / u_res);
    float r = sqrt(luma(c.rgb / max(c.a, 1e-4))) * u_cell * 0.6;
    float d = length(gl_FragCoord.xy - center);
    float dotMask = 1.0 - smoothstep(r - 0.75, r + 0.75, d);
    o = vec4(vec3(dotMask), 1.0) * c.a;`,
    }),

  fx('glitch-bitwise-hash', ['bitwise', 'uint', 'gl_FragCoord', 'time'],
    'Digital glitch with integer hash noise on blocks that changes over time.', {
      params: [
        { k: 'block', label: 'Block', def: 16, min: 2, max: 64, step: 1 },
        { k: 'amount', label: 'Amount', def: 0.4, min: 0, max: 1 },
      ],
      frag: `
    uvec2 b = uvec2(gl_FragCoord.xy) / uint(max(u_block, 1.0));
    uint h = b.x * 1664525u + b.y * 1013904223u + uint(u_time * 12.0);
    h ^= h >> 16;
    h *= 0x7feb352du;
    h ^= h >> 15;
    float r = float(h & 0xffffu) / 65535.0;
    vec2 shift = r < u_amount ? vec2((float((h >> 16) & 0xffu) / 255.0 - 0.5) * 0.1, 0.0) : vec2(0.0);
    vec4 c = texture(u_tex, v_st + shift);
    if ((h & 3u) == 1u && r < u_amount) c.rgb = c.gbr;
    o = c;`,
    }),

  fx('outline-struct-array', ['struct', 'array', 'loop', 'color'],
    'Stroke around the layer alpha with 8-direction sampling.', {
      params: [
        { k: 'size', label: 'Size', def: 3, min: 0, max: 12 },
        { k: 'color', label: 'Color', type: 'color', def: '#FF2D55' },
      ],
      frag: `
    struct Tap { vec2 dir; float weight; };
    const Tap taps[8] = Tap[8](
      Tap(vec2(1.0, 0.0), 1.0), Tap(vec2(-1.0, 0.0), 1.0), Tap(vec2(0.0, 1.0), 1.0), Tap(vec2(0.0, -1.0), 1.0),
      Tap(vec2(0.7071, 0.7071), 0.8), Tap(vec2(-0.7071, 0.7071), 0.8), Tap(vec2(0.7071, -0.7071), 0.8), Tap(vec2(-0.7071, -0.7071), 0.8));
    vec4 c = texture(u_tex, v_st);
    float a = 0.0;
    for (int i = 0; i < 8; i++) a = max(a, texture(u_tex, v_st + taps[i].dir * u_size * u_texel).a * taps[i].weight);
    vec4 stroke = vec4(u_color, 1.0) * a;
    o = c + stroke * (1.0 - c.a);`,
    }),

  fx('switch-blend-mode', ['switch', 'backdrop'],
    'Blend the layer onto what is behind it with a mode dropdown (0 multiply, 1 screen, 2 overlay, 3 difference).', {
      backdrop: true,
      params: [{ k: 'mode', label: 'Mode', def: 2, min: 0, max: 3, step: 1 }],
      frag: `
    vec4 s = texture(u_tex, v_st);
    vec3 b = texture(u_backdrop, vec2(v_uv.x, 1.0 - v_uv.y)).rgb;
    vec3 sc = s.a > 0.0 ? s.rgb / s.a : vec3(0.0);
    vec3 r;
    switch (int(u_mode + 0.5)) {
      case 0: r = b * sc; break;
      case 1: r = 1.0 - (1.0 - b) * (1.0 - sc); break;
      case 2: r = mix(2.0 * b * sc, 1.0 - 2.0 * (1.0 - b) * (1.0 - sc), step(0.5, b)); break;
      default: r = abs(b - sc); break;
    }
    o = vec4(r * s.a, s.a);`,
    }),

  fx('matrix-color-grade', ['inverse', 'matrix'],
    'Color grade with a 3x3 matrix; also undo a white balance matrix using its inverse.', {
      params: [
        { k: 'warmth', label: 'Warmth', def: 0.2, min: -1, max: 1 },
        { k: 'mixAmt', label: 'Mix', def: 0.8, min: 0, max: 1 },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    mat3 wb = mat3(1.0 + u_warmth * 0.2, 0.02, 0.0,
                   0.01, 1.0, 0.01,
                   0.0, 0.03, 1.0 - u_warmth * 0.2);
    mat3 grade = transpose(mat3(0.9, 0.1, 0.0, 0.05, 0.9, 0.05, 0.0, 0.15, 0.85));
    vec3 graded = grade * (inverse(wb) * rgb);
    mat2 sq = inverse(mat2(2.0, 0.5, 0.25, 1.5));
    graded.rg += (sq * vec2(0.01)) * u_warmth;
    o = vec4(mix(rgb, graded, u_mixAmt) * c.a, c.a);`,
    }),

  fx('outer-product-twist', ['outerProduct', 'matrix'],
    'Color twist that adds a rank-one color projection.', {
      params: [
        { k: 'axis', label: 'Axis', type: 'color', def: '#8040FF' },
        { k: 'amount', label: 'Amount', def: 0.3, min: 0, max: 1 },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec3 a = normalize(u_axis + 1e-3);
    mat3 proj = outerProduct(a, a);
    mat2x3 side = outerProduct(a, vec2(0.5, -0.25));
    vec3 rgb = mix(c.rgb, proj * c.rgb + side * vec2(c.a * 0.1), u_amount);
    o = vec4(rgb, c.a);`,
    }),

  fx('lod-and-bias', ['textureLod', 'textureBias'],
    'Soft sample using explicit LOD and a bias (the source has no mipmaps, check it still looks right).', {
      params: [{ k: 'bias', label: 'Bias', def: 1.5, min: 0, max: 4 }],
      frag: `
    vec4 a = textureLod(u_tex, v_st, 0.0);
    vec4 b = texture(u_tex, v_st + u_texel * 0.5, u_bias);
    vec4 g = textureGrad(u_tex, v_st, dFdx(v_st), dFdy(v_st));
    o = (a + b + g) / 3.0;`,
    }),

  fx('alpha-cutout-discard', ['discard'],
    'Cut out everything below an alpha threshold.', {
      params: [{ k: 'cut', label: 'Cut', def: 0.5, min: 0, max: 1 }],
      frag: `
    vec4 c = texture(u_tex, v_st);
    if (c.a < u_cut) discard;
    o = vec4(c.rgb / c.a, 1.0);`,
    }),

  fx('out-params-tonemap', ['outParam', 'function', 'rawShader'],
    'Filmic tonemap written with helper functions (use out parameters and early returns).', {
      rawShader: true,
      params: [
        { k: 'exposure', label: 'Exposure', def: 1.4, min: 0, max: 4 },
        { k: 'lift', label: 'Lift', def: 0.02, min: 0, max: 0.2 },
      ],
      frag: `#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform float u_exposure;
uniform float u_lift;
out vec4 o;
void unpackColor(vec4 c, out vec3 rgb, out float a) {
  a = c.a;
  rgb = a > 0.0 ? c.rgb / a : vec3(0.0);
}
void grade(inout vec3 rgb, float exposure, float lift) {
  rgb *= exposure;
  if (max(rgb.r, max(rgb.g, rgb.b)) <= 0.0) return;
  rgb = (rgb * (2.51 * rgb + 0.03)) / (rgb * (2.43 * rgb + 0.59) + 0.14);
  rgb = clamp(rgb + lift, 0.0, 1.0);
}
void main() {
  vec4 c = texture(u_tex, v_st);
  vec3 rgb;
  float alpha;
  unpackColor(c, rgb, alpha);
  if (alpha <= 0.0) { o = vec4(0.0); return; }
  grade(rgb, u_exposure, u_lift);
  o = vec4(rgb * alpha, alpha);
}`,
    }),

  fx('sampler-argument-sharpen', ['samplerArg', 'keepOrig', 'multipass', 'rawShader'],
    'Two-pass sharpen that compares against the untouched layer; factor the sampling into a function that takes the texture.', {
      rawShader: true, passes: 2, keepOrig: true,
      params: [{ k: 'amount', label: 'Amount', def: 0.8, min: 0, max: 3 }],
      frag: `#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform sampler2D u_orig;
uniform vec2 u_texel;
uniform int u_pass;
uniform float u_amount;
out vec4 o;
vec4 cross5(sampler2D s, vec2 p, vec2 t) {
  return (texture(s, p) * 4.0 + texture(s, p + vec2(t.x, 0.0)) + texture(s, p - vec2(t.x, 0.0))
    + texture(s, p + vec2(0.0, t.y)) + texture(s, p - vec2(0.0, t.y))) / 8.0;
}
void main() {
  vec4 orig = texture(u_orig, v_st);
  if (u_pass == 0) { o = cross5(u_tex, v_st, u_texel); return; }
  vec4 blurred = texture(u_tex, v_st);
  o = clamp(orig + (orig - blurred) * u_amount + (orig - cross5(u_orig, v_st, u_texel * 2.0)) * 0.25, 0.0, 1.0);
}`,
    }),

  fx('eight-pass-diffusion', ['multipass', 'loop'],
    'Ink diffusion that spreads over eight passes, growing each pass.', {
      passes: 8,
      params: [{ k: 'spread', label: 'Spread', def: 1.5, min: 0, max: 4 }],
      frag: `
    float s = u_spread * float(u_pass + 1);
    vec4 acc = texture(u_tex, v_st) * 2.0;
    acc += texture(u_tex, v_st + vec2(s, 0.0) * u_texel);
    acc += texture(u_tex, v_st - vec2(s, 0.0) * u_texel);
    acc += texture(u_tex, v_st + vec2(0.0, s) * u_texel);
    acc += texture(u_tex, v_st - vec2(0.0, s) * u_texel);
    o = acc / 6.0;`,
    }),

  fx('glsl450-reserved-names', ['renamedIdentifier'],
    'Soft light glow (agent output that happens to use identifiers reserved in desktop GLSL).', {
      params: [{ k: 'amount', label: 'Amount', def: 0.5, min: 0, max: 1 }],
      frag: `
    vec4 c = texture(u_tex, v_st);
    float shared = luma(c.rgb);
    vec3 buffer = vec3(shared) * u_amount;
    float precise = smoothstep(0.2, 0.8, shared);
    o = vec4(c.rgb + buffer * precise * c.a, c.a);`,
    }),

  fx('precision-qualifiers', ['precisionQualifiers'],
    'Posterize; write it mobile-friendly with mediump and lowp where possible.', {
      params: [{ k: 'levels', label: 'Levels', def: 5, min: 2, max: 16, step: 1 }],
      frag: `
    highp vec4 c = texture(u_tex, v_st);
    mediump float n = max(u_levels, 2.0) - 1.0;
    lowp vec3 rgb = c.a > 0.0 ? c.rgb / c.a : vec3(0.0);
    mediump vec3 q = floor(rgb * n + 0.5) / n;
    o = vec4(q * c.a, c.a);`,
    }),

  fx('raw-shader-legacy', ['rawShader', 'positionalAlias', 'color'],
    'Use this complete legacy shader as-is (rawShader), it reads u_p0 and u_c1.', {
      rawShader: true,
      params: [
        { k: 'amount', label: 'Amount', def: 0.6, min: 0, max: 1 },
        { k: 'tint', label: 'Tint', type: 'color', def: '#FF8800' },
      ],
      frag: `#version 300 es
precision highp float;
in vec2 v_st;
uniform sampler2D u_tex;
uniform float u_p0;
uniform vec3 u_c1;
uniform float u_time;
out vec4 o;
void main() {
  vec4 c = texture(u_tex, v_st);
  float l = dot(c.rgb, vec3(0.299, 0.587, 0.114));
  vec3 t = u_c1 * l * (0.9 + 0.1 * sin(u_time));
  o = vec4(mix(c.rgb, t, u_p0), c.a);
}`,
    }),

  fx('raw-shader-pixel-varyings', ['rawShader', 'varyingByName', 'positionalAlias'],
    'Raw shader for a checker overlay that only needs pixel coordinates.', {
      rawShader: true,
      params: [{ k: 'size', label: 'Size', def: 12, min: 2, max: 64 }],
      frag: `#version 300 es
precision highp float;
in vec2 v_px;
uniform vec2 u_res;
uniform float u_p0;
out vec4 o;
void main() {
  vec2 cell = floor(v_px / max(u_p0, 1.0));
  float k = mod(cell.x + cell.y, 2.0);
  o = vec4(vec3(k), 1.0) * 0.5 + vec4(v_px / u_res, 0.0, 0.0) * 0.5;
}`,
    }),

  // The first draft of the shader above: mediump default precision makes
  // u_res's precision differ from the vertex stage's, which WebGL refuses to
  // link. Kept to check the engine rejects it the same way.
  fx('raw-shader-precision-mismatch', ['rawShader', 'linkError'],
    'Raw shader for a checker overlay that only needs pixel coordinates (mobile precision).', {
      rawShader: true,
      params: [{ k: 'size', label: 'Size', def: 12, min: 2, max: 64 }],
      frag: `#version 300 es
precision mediump float;
in vec2 v_px;
uniform vec2 u_res;
uniform float u_p0;
out vec4 o;
void main() {
  vec2 cell = floor(v_px / max(u_p0, 1.0));
  float k = mod(cell.x + cell.y, 2.0);
  o = vec4(vec3(k), 1.0) * 0.5 + vec4(v_px / u_res, 0.0, 0.0) * 0.5;
}`,
    }),

  // An agent's first draft with a type error on the body's third line: WebGL
  // and the engine must both reject it, and the engine must report line 3.
  fx('type-error-first-draft', ['compileError'],
    'Tint the layer toward a color by an amount.', {
      params: [
        { k: 'color', label: 'Color', type: 'color', def: '#FF0080' },
        { k: 'amount', label: 'Amount', def: 0.5, min: 0, max: 1 },
      ],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec3 t = texture(u_tex, v_st) * u_color;
    o = vec4(mix(c.rgb, t, u_amount), c.a);`,
    }),

  fx('ub-pow-normalize-edge', ['ub-probe'],
    'Spotlight from the layer center (uses pow and normalize on values that can be zero or negative).', {
      params: [{ k: 'falloff', label: 'Falloff', def: 2.2, min: 0.5, max: 6 }],
      frag: `
    vec4 c = texture(u_tex, v_st);
    vec2 d = v_st - vec2(0.5);
    vec2 n = normalize(d);
    float r = pow(1.0 - length(d) * 2.0, u_falloff);
    o = vec4(c.rgb * clamp(r, 0.0, 1.0) + vec3(n * 0.1, 0.0), c.a);`,
    }),

  fx('ub-texel-fetch-out-of-range', ['ub-probe', 'texelFetch'],
    'Shift the layer by a whole number of pixels with texelFetch (no clamping).', {
      params: [{ k: 'shift', label: 'Shift', def: 6, min: 0, max: 40, step: 1 }],
      frag: `
    ivec2 p = ivec2(gl_FragCoord.xy) - ivec2(int(u_shift), 0);
    o = texelFetch(u_tex, p, 0);`,
    }),

  fx('ub-divide-by-zero', ['ub-probe'],
    'Normalize color by its max channel to get pure hues.', {
      params: [],
      frag: `
    vec4 c = texture(u_tex, v_st);
    float m = max(c.r, max(c.g, c.b));
    vec3 hue = c.rgb / m;
    o = vec4(clamp(hue, 0.0, 1.0) * c.a, c.a);`,
    }),
  fx('nearest-bright-pixel-infinity', ['nonFiniteConstant', 'dataDependentLoop'],
    'Glow toward the nearest bright pixel; start the search distance at infinity.', {
      params: [{ k: 'reach', label: 'Reach', def: 12, min: 2, max: 24, step: 1 }],
      frag: `
    const float FAR = 1e39;
    vec4 c = texture(u_tex, v_st);
    float best = FAR;
    for (int i = -6; i <= 6; i++) {
      for (int j = -6; j <= 6; j++) {
        vec2 off = vec2(float(i), float(j)) * u_reach / 6.0;
        if (luma(src(v_st + off * u_texel).rgb) > 0.6) best = min(best, length(off));
      }
    }
    float glow = isinf(best) ? 0.0 : exp(-best / max(u_reach, 1.0) * 3.0);
    o = vec4(c.rgb + glow * vec3(1.0, 0.8, 0.4) * (1.0 - c.a), max(c.a, glow));`,
    }),
];
