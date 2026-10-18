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
// Intermediates are stored at half precision (each legacy pass writes an
// RGBA16F target): round between fused steps.
const FLAG_HALF: i32 = 64;

@group(0) @binding(0) var<uniform> op: Op;
@group(0) @binding(1) var ta: texture_2d<f32>;
@group(0) @binding(2) var tb: texture_2d<f32>;

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

// An RGBA16F render target stores a fragment's f32 output rounded toward
// zero on this GPU (measured: 0.75 − 1 f32 ulp stores as 0.74951), so each
// of today's intermediate passes truncates. Fused steps reproduce it.
fn half_rtz(v: f32) -> f32 {
    let a = abs(v);
    if (a != a || a >= 65536.0) {
        return v;
    }
    if (a >= 65504.0) {
        return sign(v) * 65504.0;
    }
    if (a < 6.103515625e-05) {
        // Subnormal halves: multiples of 2^-24, truncated.
        return sign(v) * floor(a * 16777216.0) / 16777216.0;
    }
    return bitcast<f32>(bitcast<u32>(v) & 0xffffe000u);
}

fn half(v: f32) -> f32 {
    if (has(FLAG_HALF)) {
        return half_rtz(v);
    }
    return v;
}

fn half4(v: vec4<f32>) -> vec4<f32> {
    if (has(FLAG_HALF)) {
        return vec4<f32>(half_rtz(v.x), half_rtz(v.y), half_rtz(v.z), half_rtz(v.w));
    }
    return v;
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

// Masks (`FRAG_MASK` then `FRAG_MASK_APPLY`) and the track matte
// (`applyTrackMatte`), in that order, on the layer in `ta`; the matte pass
// is in `tb`.
@fragment
fn fs_layer(in: VsOut) -> @location(0) vec4<f32> {
    var c = load_a(in.pos);
    if (has(FLAG_MASKS)) {
        c = c * half(coverage(in.px));
    }
    if (has(FLAG_MATTE)) {
        if (has(FLAG_MASKS)) {
            c = half4(c);
        }
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
// blend mode, in the working encoding, evaluated as today's program
// evaluates it at every pixel of the frame: outside the layer the
// destination goes through `(d.rgb / d.a) · d.a`, whose result the half
// float target truncates, so a translucent stack drifts by up to an ulp per
// blend layer exactly as today's frames do (docs/engine/composite.md).
@fragment
fn fs_composite(in: VsOut) -> @location(0) vec4<f32> {
    var s = load_a(in.pos);
    let d = load_b(in.pos);
    s.a *= op.f.x;
    s = vec4<f32>(s.rgb * op.f.x, s.a);
    var sc = vec3<f32>(0.0);
    if (s.a > 0.0001) {
        sc = s.rgb / s.a;
    }
    var dc = vec3<f32>(0.0);
    if (d.a > 0.0001) {
        dc = d.rgb / d.a;
    }
    let bc = pm_blend_working(dc, sc, op.i.x, has(FLAG_LINEAR), has(FLAG_BLEND_IN_E));
    let res = mix(dc, bc, s.a);
    let a = s.a + d.a * (1.0 - s.a);
    return vec4<f32>(res * a, a);
}

// A rectangle of `ta` (a padded pass) into a smaller target.
@fragment
fn fs_copy(in: VsOut) -> @location(0) vec4<f32> {
    return load_a(in.pos);
}

// The finished frame from GL row order to top-down rows, encoded with E
// when the working encoding is linear light (section 9: E on output).
// Color outside coverage (alpha above 1 or below the unpremultiply floor:
// light a legacy effect wrote, `crate::effects::SimpleOp::DecodeE`) is
// encoded per channel, as the effect passes decode it.
@fragment
fn fs_encode_flip(in: VsOut) -> @location(0) vec4<f32> {
    let h = i32(textureDimensions(ta).y);
    let at = vec2<i32>(i32(in.pos.x), h - 1 - i32(in.pos.y));
    let c = textureLoad(ta, at, 0);
    if (has(FLAG_LINEAR)) {
        if (c.a > 1.0 || !pm_may_unpremultiply(c.a)) {
            return vec4<f32>(pm_e_encode3(c.rgb), c.a);
        }
        return pm_premultiplied_linear_to_e(c);
    }
    return c;
}
