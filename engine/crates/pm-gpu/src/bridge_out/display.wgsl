// The viewer display transform (plan section 9 "Viewer", section 18 item 2):
// working values to the pool surface's format.
//
// - Working values are premultiplied and opaque over the composition's
//   background; display encoding E unless linear blending is on.
// - On a display-p3 canvas, BT.709 primaries are converted to P3 in linear
//   light, then encoded with E.
// - rgbaf16 pools keep values above 1.0 (EDR displays). The result is
//   rounded to the nearest f16 before the write: Apple GPUs truncate on
//   render-target writes, which would add up to one f16 step after the
//   P3 conversion (0.5 CV10 near 1.0, measured on a grey ramp).
// - bgra pools clamp to 0..1 and quantize to 8 bits with a blue-noise RPDF
//   dither of ±0.5 LSB. Values on the 8-bit grid stay exact: a working value
//   within one f16 step of a grid code is that code (an 8-bit color stored in
//   an rgba16float working texture lands up to one step off; Apple GPUs
//   truncate on render-target writes), so flat 8-bit colors are not
//   dithered.

struct Params {
    flags: u32,          // bit0: 8-bit target, bit1: linear-light input, bit2: P3 canvas
    _pad: u32,
    noise_off: vec2u,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var noise: texture_2d<f32>;

@vertex
fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let q = vec2f(f32((i << 1u) & 2u), f32(i & 2u));
    return vec4f(q * 2.0 - 1.0, 0.0, 1.0);
}

fn e_encode1(v: f32) -> f32 {
    let m = abs(v);
    var e: f32;
    if m <= 0.0031308 { e = 12.92 * m; } else { e = 1.055 * pow(m, 1.0 / 2.4) - 0.055; }
    return select(e, -e, v < 0.0);
}

fn e_decode1(v: f32) -> f32 {
    let m = abs(v);
    var l: f32;
    if m <= 0.04045 { l = m / 12.92; } else { l = pow((m + 0.055) / 1.055, 2.4); }
    return select(l, -l, v < 0.0);
}

// The spacing of f16 values at |c| (one ulp), per channel.
fn f16_step(c: vec3f) -> vec3f {
    let m = max(abs(c), vec3f(6.1035156e-5));
    return exp2(floor(log2(m)) - 10.0);
}

fn e_encode(c: vec3f) -> vec3f { return vec3f(e_encode1(c.r), e_encode1(c.g), e_encode1(c.b)); }
fn e_decode(c: vec3f) -> vec3f { return vec3f(e_decode1(c.r), e_decode1(c.g), e_decode1(c.b)); }

// Linear BT.709 to linear Display P3 (both D65); the arguments are columns.
const BT709_TO_P3 = mat3x3f(
    vec3f(0.8224621, 0.0331941, 0.0170827),
    vec3f(0.1775380, 0.9668058, 0.0723974),
    vec3f(0.0000000, 0.0000000, 0.9105199),
);

@fragment
fn fs(@builtin(position) pos: vec4f) -> @location(0) vec4f {
    let xy = vec2u(pos.xy);
    var c = textureLoad(src, vec2i(xy), 0).rgb;
    let eight_bit = (p.flags & 1u) != 0u;
    let linear_in = (p.flags & 2u) != 0u;
    let p3 = (p.flags & 4u) != 0u;
    if p3 {
        var l = c;
        if !linear_in { l = e_decode(c); }
        c = e_encode(BT709_TO_P3 * l);
    } else if linear_in {
        c = e_encode(c);
    }
    if eight_bit {
        c = clamp(c, vec3f(0.0), vec3f(1.0));
        let t = (xy + p.noise_off) % vec2u(64u);
        let n = textureLoad(noise, vec2i(t), 0).r - 0.5;
        let s = c * 255.0;
        let grid = round(s);
        let on_grid = abs(s - grid) <= f16_step(c) * 255.0;
        c = select(floor(s + 0.5 + n), grid, on_grid) / 255.0;
    } else {
        c = quantizeToF16(c);
    }
    return vec4f(c, 1.0);
}
