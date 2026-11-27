// Compositing passes (plan sections 8 and 9; legacy `legacy/gl/shaders.ts`
// and `compositor.ts`). Prepended with `pm_color::wgsl::COLOR` and
// `pm_color::wgsl::BLEND`.
//
// Every pass draws today's full-frame quad (`fullQuad(W, H)` through
// `PM.VERT`) in full-frame output pixels; a region of interest moves the
// viewport, so `v_px` (composition pixels × output scale, top-left origin)
// is the same at every pixel as in the full frame. Textures are read with
// `textureLoad` at the fragment's pixel plus a per-texture offset (each
// texture may cover a different rectangle of the frame); today's passes
// sample their inputs at texel centers with no filtering, which is the
// same value. Targets are in GL row order (see shaders/composite.wgsl).

struct Op {
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // xy: full output size in pixels.
    res: vec4<f32>,
    // Target pixel → texel of `ta` (xy) and of `tb` (zw).
    off: vec4<i32>,
    // x: composite alpha (`u_alpha`).
    f: vec4<f32>,
    // x: blend mode (`BLEND_ID`), y: flags, z: mask count, w: 1 when a mask adds.
    i: vec4<i32>,
    // `u_inv`: output pixels → layer pixels, columns.
    inv0: vec4<f32>,
    inv1: vec4<f32>,
    inv2: vec4<f32>,
    // `u_g[8]`: cx, cy, w, h (layer pixels).
    g: array<vec4<f32>, 8>,
    // `u_q[8]`: rotation (radians), feather, shape (0 rect, 1 ellipse), mode (0 add, 1 subtract).
    q: array<vec4<f32>, 8>,
};

const FLAG_MASKS: i32 = 1;
const FLAG_MATTE: i32 = 2;
const FLAG_MATTE_LUMA: i32 = 4;
const FLAG_MATTE_INVERTED: i32 = 8;
const FLAG_LINEAR: i32 = 16;
const FLAG_BLEND_IN_E: i32 = 32;
const FLAG_SRGB8: i32 = 64;
// Bezier path masks: the layer times today's coverage canvas (`tcov`).
const FLAG_PATH_MASKS: i32 = 128;
const FLAG_FLIP: i32 = 256;
const FLAG_TONE_MAP: i32 = 512;

@group(0) @binding(0) var<uniform> op: Op;
@group(0) @binding(1) var ta: texture_2d<f32>;
@group(0) @binding(2) var tb: texture_2d<f32>;
// The path mask coverage canvas (top-down, output size) and today's
// `texFor` filtering (linear, clamp to edge).
@group(0) @binding(3) var tcov: texture_2d<f32>;
@group(0) @binding(4) var scov: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // `v_px`: output pixels, top-left origin.
    @location(0) px: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) i: u32) -> VsOut {
    let a = vec2<f32>(f32(i & 1u), f32(i >> 1u));
    let m = mat3x3<f32>(op.m0.xyz, op.m1.xyz, op.m2.xyz);
    let p = m * vec3<f32>(a, 1.0);
    var o: VsOut;
    o.px = p.xy / p.z;
    o.pos = vec4<f32>((p.x / op.res.x) * 2.0 - p.z, (p.y / op.res.y) * 2.0 - p.z, 0.0, p.z);
    return o;
}

fn has(flag: i32) -> bool {
    return (op.i.y & flag) != 0;
}

fn load_a(pos: vec4<f32>) -> vec4<f32> {
    return textureLoad(ta, vec2<i32>(pos.xy) + op.off.xy, 0);
}

fn load_b(pos: vec4<f32>) -> vec4<f32> {
    return textureLoad(tb, vec2<i32>(pos.xy) + op.off.zw, 0);
}

fn sd_box(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let d = abs(p) - b;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

// `FRAG_MASK`: analytic coverage of up to 8 parametric masks.
fn coverage(px: vec2<f32>) -> f32 {
    let inv = mat3x3<f32>(op.inv0.xyz, op.inv1.xyz, op.inv2.xyz);
    let local = inv * vec3<f32>(px, 1.0);
    let lp = local.xy / local.z;
    var cov = select(1.0, 0.0, op.i.w == 1);
    for (var i = 0; i < 8; i++) {
        if (i >= op.i.z) {
            break;
        }
        let g = op.g[i];
        let q = op.q[i];
        if (g.z < 0.01 || g.w < 0.01) {
            continue;
        }
        var p = lp - g.xy;
        let r = q.x;
        if (abs(r) > 1e-4) {
            let c = cos(r);
            let s = sin(r);
            p = mat2x2<f32>(c, -s, s, c) * p;
        }
        let h = max(g.zw * 0.5, vec2<f32>(0.001));
        var d: f32;
        if (q.z < 0.5) {
            d = sd_box(p, h);
        } else {
            d = (length(p / h) - 1.0) * min(h.x, h.y);
        }
        let f = max(q.y, 0.0);
        var a: f32;
        if (f > 0.01) {
            a = 1.0 - smoothstep(-f * 0.5, f * 0.5, d);
        } else {
            a = step(d, 0.0);
        }
        if (q.w < 0.5) {
            cov = max(cov, a);
        } else {
            cov = min(cov, 1.0 - a);
        }
    }
    return cov;
}

// `pathMaskApply`: the coverage canvas sampled through `u_covTransform`
// (output pixels to canvas pixels; `inv` holds it on this route), zero
// outside the canvas.
fn path_coverage(px: vec2<f32>) -> f32 {
    let t = mat3x3<f32>(op.inv0.xyz, op.inv1.xyz, op.inv2.xyz);
    let q = t * vec3<f32>(px, 1.0);
    let uv = q.xy / q.z / op.res.xy;
    let c = textureSampleLevel(tcov, scov, uv, 0.0).r;
    if (uv.x < 0.0 || uv.y < 0.0 || uv.x > 1.0 || uv.y > 1.0) {
        return 0.0;
    }
    return c;
}

// Masks (`FRAG_MASK` then `FRAG_MASK_APPLY`, or `pathMaskApply`) and the track matte
// (`applyTrackMatte`), in that order, on the layer in `ta`; the matte pass
// is in `tb`.
fn fs_layer_value(in: VsOut) -> vec4<f32> {
    var c = load_a(in.pos);
    if (has(FLAG_MASKS)) {
        c = c * coverage(in.px);
    }
    if (has(FLAG_PATH_MASKS)) {
        c = c * path_coverage(in.px);
    }
    if (has(FLAG_MATTE)) {
        var m = load_b(in.pos);
        if (has(FLAG_LINEAR)) {
            // Luma mattes read E-encoded values (contract 4, invariant 7).
            m = pm_premultiplied_linear_to_e(m);
        }
        var a: f32;
        if (has(FLAG_MATTE_LUMA)) {
            a = dot(m.rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
        } else {
            a = m.a;
        }
        if (has(FLAG_MATTE_INVERTED)) {
            a = 1.0 - a;
        }
        c = c * clamp(a, 0.0, 1.0);
    }
    return c;
}

// `FRAG_COMPOSITE`: the layer (`ta`) over the accumulator (`tb`) with a
// blend mode, in the working encoding. Both textures contain premultiplied
// color; retain each source's unblended contribution wherever the other
// source is translucent (docs/engine/composite.md).
fn fs_composite_value(in: VsOut) -> vec4<f32> {
    return composite_color(load_a(in.pos), load_b(in.pos));
}

// A single solid can use its actual quad as this pass's geometry. The
// destination is still snapped to covered bounds, but no source texture is
// needed. This shares the blend arithmetic with all other layer sources.
fn fs_solid_composite_value(in: VsOut) -> vec4<f32> {
    return composite_color(op.g[0], load_b(in.pos));
}

fn composite_color(source: vec4<f32>, d: vec4<f32>) -> vec4<f32> {
    var s = source;
    s.a *= op.f.x;
    s = vec4<f32>(s.rgb * op.f.x, s.a);
    // A transparent source is the exact identity, including additive RGB.
    if (all(s == vec4<f32>(0.0))) {
        return d;
    }
    var sc = vec3<f32>(0.0);
    if (s.a > 0.0001) {
        sc = s.rgb / s.a;
    }
    var dc = vec3<f32>(0.0);
    if (d.a > 0.0001) {
        dc = d.rgb / d.a;
    }
    let bc = pm_blend_working(dc, sc, op.i.x, has(FLAG_LINEAR), has(FLAG_BLEND_IN_E));
    let res = (1.0 - s.a) * d.rgb + (1.0 - d.a) * s.rgb + s.a * d.a * bc;
    let a = s.a + d.a * (1.0 - s.a);
    return vec4<f32>(res, a);
}

// A rectangle of `ta` (a padded pass) into a smaller target.
fn fs_copy_value(in: VsOut) -> vec4<f32> {
    return load_a(in.pos);
}

// The finished frame from GL row order to top-down rows, encoded with E
// when the working encoding is linear light (section 9: E on output), and
// tone mapped when the target holds less light than the frame.
// Color outside coverage (alpha above 1 or below the unpremultiply floor:
// light a legacy effect wrote, `crate::effects::SimpleOp::DecodeE`) is
// encoded per channel, as the effect passes decode it.
fn fs_encode_flip_value(in: VsOut) -> vec4<f32> {
    let h = i32(textureDimensions(ta).y);
    var at = vec2<i32>(in.pos.xy);
    if (has(FLAG_FLIP)) { at.y = h - 1 - at.y; }
    var c = textureLoad(ta, at, 0);
    if (has(FLAG_LINEAR)) {
        if (c.a > 1.0 || !pm_may_unpremultiply(c.a)) {
            c = vec4<f32>(pm_e_encode3(c.rgb), c.a);
        } else {
            c = pm_premultiplied_linear_to_e(c);
        }
    }
    if (has(FLAG_TONE_MAP)) {
        c = pm_tone_map_output(c, op.f.x, op.f.y);
    }
    if (has(FLAG_SRGB8)) {
        var rgb = pm_css_gamut_srgb(c.rgb);
        if (c.a <= 1.0 && pm_may_unpremultiply(c.a)) {
            rgb = pm_css_gamut_srgb(c.rgb / c.a) * c.a;
        }
        c = vec4<f32>(rgb, c.a);
    }
    return c;
}

fn fs_scene_output_value(in: VsOut) -> vec4<f32> {
    return scene_output(load_a(in.pos), op.f.x, op.g[0], has(FLAG_LINEAR), op.f.y > 0.5);
}


// An effect/shader's authored E-space output joins linear filtering only
// after decoding; decoding a filtered E-space value would change the kernel.
fn fs_decode_source_value(in: VsOut) -> vec4<f32> {
    let c = load_a(in.pos);
    if (c.a > 1.0 || !pm_may_unpremultiply(c.a)) {
        return vec4<f32>(pm_e_decode3(c.rgb), c.a);
    }
    return pm_premultiplied_e_to_linear(c);
}

// Each op's entry point stores its value as is; the `_half` entry for an
// RGBA16F target stores the nearest half (Metal's conversion truncates).
@fragment fn fs_layer(in: VsOut) -> @location(0) vec4<f32> { return fs_layer_value(in); }
@fragment fn fs_layer_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_layer_value(in)); }
@fragment fn fs_composite(in: VsOut) -> @location(0) vec4<f32> { return fs_composite_value(in); }
@fragment fn fs_composite_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_composite_value(in)); }
@fragment fn fs_solid_composite(in: VsOut) -> @location(0) vec4<f32> { return fs_solid_composite_value(in); }
@fragment fn fs_solid_composite_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_solid_composite_value(in)); }
@fragment fn fs_copy(in: VsOut) -> @location(0) vec4<f32> { return fs_copy_value(in); }
@fragment fn fs_copy_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_copy_value(in)); }
@fragment fn fs_encode_flip(in: VsOut) -> @location(0) vec4<f32> { return fs_encode_flip_value(in); }
@fragment fn fs_encode_flip_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_encode_flip_value(in)); }
@fragment fn fs_scene_output(in: VsOut) -> @location(0) vec4<f32> { return fs_scene_output_value(in); }
@fragment fn fs_scene_output_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_scene_output_value(in)); }
@fragment fn fs_decode_source(in: VsOut) -> @location(0) vec4<f32> { return fs_decode_source_value(in); }
@fragment fn fs_decode_source_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_decode_source_value(in)); }

fn dof_coc_at(px: vec2<f32>) -> vec2<f32> {
    let inv_w = op.inv0.x * px.x + op.inv0.y * px.y + op.inv0.z;
    let w = 1.0 / max(inv_w, 1e-4);
    let coc = clamp(op.f.y * abs(w - op.f.x) / max(w, 0.1), 0.0, op.f.z) * op.inv1.z;
    return vec2<f32>(w, coc);
}

// 2.5D depth-of-field bokeh pass (16-tap golden-angle Vogel disk with IGN jitter
// and analytical depth-aware foreground/background separation). Works across all
// working formats (Rgba8Unorm, Rgba16Float, Rgba32Float) via textureLoad.
fn fs_dof_value(in: VsOut) -> vec4<f32> {
    let base_pos = vec2<i32>(in.pos.xy) + op.off.xy;
    let dims = vec2<i32>(i32(op.inv1.x), i32(op.inv1.y));
    let c0 = textureLoad(ta, clamp(base_pos, vec2<i32>(0), dims - vec2<i32>(1)), 0);
    let wc0 = dof_coc_at(in.px);
    let w0 = wc0.x;
    let coc0 = wc0.y;
    if (coc0 < 0.35) {
        return c0;
    }
    let ign = fract(52.9829189 * fract(dot(in.pos.xy, vec2<f32>(0.06711056, 0.00583715))));
    let base_angle = ign * 6.2831853 + op.f.w;
    let golden = 2.39996323;
    let taps = 16u;
    let inv_taps = 1.0 / 16.0;
    var acc = c0;
    var wsum = 1.0;
    for (var t = 0u; t < taps; t = t + 1u) {
        let r = sqrt((f32(t) + 0.5) * inv_taps) * coc0;
        let a = base_angle + f32(t) * golden;
        let delta = vec2<f32>(cos(a), sin(a)) * r;
        let qpos = clamp(base_pos + vec2<i32>(round(delta)), vec2<i32>(0), dims - vec2<i32>(1));
        // in.px is top-down while in.pos is GL bottom-up.
        let qpx = in.px + vec2<f32>(delta.x, -delta.y);
        let wcq = dof_coc_at(qpx);
        let in_front = wcq.x < w0 - 0.01;
        let eff_coc = select(min(coc0, max(wcq.y, coc0 * 0.6)), wcq.y, in_front);
        let wt = smoothstep(r - 0.75, r + 0.75, eff_coc);
        if (wt > 0.0) {
            let sc = textureLoad(ta, qpos, 0);
            acc += sc * wt;
            wsum += wt;
        }
    }
    return acc / max(wsum, 1e-6);
}

@fragment fn fs_dof(in: VsOut) -> @location(0) vec4<f32> { return fs_dof_value(in); }
@fragment fn fs_dof_half(in: VsOut) -> @location(0) vec4<f32> { return pm_scene_half_color(fs_dof_value(in)); }
