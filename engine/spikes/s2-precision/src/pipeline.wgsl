// Spike S2 prototype pipeline: source conversion → working texture → encoder input.
// Plan sections 9 (color rule, alpha), 13 (normalization), 17 (encoder input).
//
// Working values: premultiplied RGB, Rec.709 primaries, display encoding E
// (Standard) or linear light (linear blending on).

struct Params {
    // 1 = linear blending on (E⁻¹ on input, E on output)
    linear: u32,
    // 1 = premultiply on input / unpremultiply on output
    premultiply: u32,
    // 1 = clip Y′CbCr codes to the codable range at output
    clip: u32,
    // α below this is treated as zero when unpremultiplying
    alpha_guard: f32,
    // 1 = blue-noise RPDF dither of ±0.5 LSB in the output domain (section 17)
    dither: u32,
    // frame index seeding the dither offset
    frame: u32,
    _pad0: u32,
    _pad1: u32,
};

@group(0) @binding(0) var src0: texture_2d<f32>;
@group(0) @binding(1) var src1: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -3.0), vec2(-1.0, 1.0), vec2(3.0, 1.0));
    var o: VsOut;
    o.pos = vec4(p[i], 0.0, 1.0);
    return o;
}

const KR: f32 = 0.2126;
const KB: f32 = 0.0722;
const KG: f32 = 0.7152;

fn ycbcr_to_rgb(y: f32, cb: f32, cr: f32) -> vec3<f32> {
    let r = y + 2.0 * (1.0 - KR) * cr;
    let b = y + 2.0 * (1.0 - KB) * cb;
    let g = (y - KR * r - KB * b) / KG;
    return vec3(r, g, b);
}

fn rgb_to_ycbcr(c: vec3<f32>) -> vec3<f32> {
    let y = KR * c.r + KG * c.g + KB * c.b;
    let cb = (c.b - y) / (2.0 * (1.0 - KB));
    let cr = (c.r - y) / (2.0 * (1.0 - KR));
    return vec3(y, cb, cr);
}

// E: sRGB piecewise curve extended to all reals (odd-symmetric, power continued above 1).
fn e_encode1(x: f32) -> f32 {
    let a = abs(x);
    var v: f32;
    if (a <= 0.0031308) {
        v = 12.92 * a;
    } else {
        v = 1.055 * pow(a, 1.0 / 2.4) - 0.055;
    }
    return select(v, -v, x < 0.0);
}

fn e_decode1(x: f32) -> f32 {
    let a = abs(x);
    var v: f32;
    if (a <= 0.04045) {
        v = a / 12.92;
    } else {
        v = pow((a + 0.055) / 1.055, 2.4);
    }
    return select(v, -v, x < 0.0);
}

fn e_encode(c: vec3<f32>) -> vec3<f32> {
    return vec3(e_encode1(c.r), e_encode1(c.g), e_encode1(c.b));
}

fn e_decode(c: vec3<f32>) -> vec3<f32> {
    return vec3(e_decode1(c.r), e_decode1(c.g), e_decode1(c.b));
}

// Encoded (E) straight RGB + α → working value.
fn to_working(rgb_e: vec3<f32>, a: f32) -> vec4<f32> {
    var c = rgb_e;
    if (params.linear == 1u) {
        c = e_decode(c);
    }
    if (params.premultiply == 1u) {
        c = c * a;
    }
    return vec4(c, a);
}

// Working value → encoded (E) straight RGB + α.
fn from_working(w: vec4<f32>) -> vec4<f32> {
    var c = w.rgb;
    let a = w.a;
    if (params.premultiply == 1u) {
        if (a < params.alpha_guard) {
            c = vec3(0.0);
        } else {
            c = c / a;
        }
    }
    if (params.linear == 1u) {
        c = e_encode(c);
    }
    return vec4(c, a);
}

fn unorm16(t: vec4<f32>) -> vec4<f32> {
    // Exact integer code values from a unorm16 texel.
    return round(t * 65535.0);
}

// ---- Source conversion (render into the working texture) ----

@fragment
fn fs_from_y416(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    // y416 is A, Y′, Cb, Cr imported as RGBA16Unorm.
    let v = unorm16(textureLoad(src0, p, 0));
    let a = v.x / 65535.0;
    let y = (v.y - 4096.0) / 56064.0;
    let cb = (v.z - 32768.0) / 57344.0;
    let cr = (v.w - 32768.0) / 57344.0;
    return to_working(ycbcr_to_rgb(y, cb, cr), a);
}

@fragment
fn fs_from_x422(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let yv = unorm16(textureLoad(src0, p, 0)).x / 64.0;
    let cw = i32(textureDimensions(src1).x);
    // Co-sited 4:2:2: even columns carry chroma; odd columns interpolate linearly.
    let cx = p.x / 2;
    var c = unorm16(textureLoad(src1, vec2(cx, p.y), 0)).xy / 64.0;
    if ((p.x & 1) == 1) {
        let c2 = unorm16(textureLoad(src1, vec2(min(cx + 1, cw - 1), p.y), 0)).xy / 64.0;
        c = (c + c2) * 0.5;
    }
    let y = (yv - 64.0) / 876.0;
    let cb = (c.x - 512.0) / 896.0;
    let cr = (c.y - 512.0) / 896.0;
    return to_working(ycbcr_to_rgb(y, cb, cr), 1.0);
}

// 16-bit straight RGBA (for example a 16-bit PNG or a Blender surface), E-encoded.
@fragment
fn fs_from_rgba16(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let v = unorm16(textureLoad(src0, p, 0)) / 65535.0;
    return to_working(v.rgb, v.a);
}

// ---- Encoder input (render into the sink buffer's planes) ----
// In output passes, src1 is the 64×64 blue-noise rank texture (values in (0, 1)).

fn hash_u32(x: u32) -> u32 {
    // PCG output permutation (O'Neill), deterministic on every device.
    let state = x * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// RPDF offset in (−0.5, 0.5) for pixel p and channel ch, seeded by frame; zero when off.
// Values already on the output grid are unchanged because |offset| < 0.5.
fn dither_offset(p: vec2<i32>, ch: u32) -> f32 {
    if (params.dither == 0u) {
        return 0.0;
    }
    let h = hash_u32(params.frame * 4u + ch);
    let o = vec2<i32>(i32(h & 63u), i32((h >> 6u) & 63u));
    let q = (p + o) & vec2<i32>(63, 63);
    return textureLoad(src1, q, 0).x - 0.5;
}

fn code_d(v: f32, lo: f32, hi: f32, p: vec2<i32>, ch: u32) -> f32 {
    return code(v + dither_offset(p, ch), lo, hi);
}

fn code(v: f32, lo: f32, hi: f32) -> f32 {
    let c = round(v);
    if (params.clip == 1u) {
        return clamp(c, lo, hi);
    }
    return clamp(c, 0.0, 65535.0);
}

@fragment
fn fs_to_y416(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let w = from_working(textureLoad(src0, p, 0));
    let ycc = rgb_to_ycbcr(w.rgb);
    // 12-bit codable range 16..4079, expressed in y416 16-bit units (× 16).
    // Dither (when on) at 12-bit LSB = 16 units of y416.
    let y = code(ycc.x * 56064.0 + 4096.0 + 16.0 * dither_offset(p, 0u), 256.0, 65264.0);
    let cb = code(ycc.y * 57344.0 + 32768.0 + 16.0 * dither_offset(p, 1u), 256.0, 65264.0);
    let cr = code(ycc.z * 57344.0 + 32768.0 + 16.0 * dither_offset(p, 2u), 256.0, 65264.0);
    let a = clamp(round(w.a * 65535.0), 0.0, 65535.0);
    return vec4(a, y, cb, cr) / 65535.0;
}

@fragment
fn fs_to_x422_luma(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let w = from_working(textureLoad(src0, p, 0));
    let ycc = rgb_to_ycbcr(w.rgb);
    let y = code_d(ycc.x * 876.0 + 64.0, 4.0, 1019.0, p, 0u);
    return vec4(y * 64.0 / 65535.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_to_x422_chroma(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    // Point decimation at the co-sited (even) luma position.
    let w = from_working(textureLoad(src0, vec2(p.x * 2, p.y), 0));
    let ycc = rgb_to_ycbcr(w.rgb);
    let cb = code_d(ycc.y * 896.0 + 512.0, 4.0, 1019.0, p, 1u);
    let cr = code_d(ycc.z * 896.0 + 512.0, 4.0, 1019.0, p, 2u);
    return vec4(cb * 64.0 / 65535.0, cr * 64.0 / 65535.0, 0.0, 1.0);
}

// 4:2:0, chroma siting type 0 (left): horizontally co-sited with even luma columns,
// vertically between rows 2y and 2y + 1. Declared filter: point decimation
// horizontally, two-tap average vertically, computed on Y′CbCr.
fn chroma420(p: vec2<i32>) -> vec2<f32> {
    let h = i32(textureDimensions(src0).y);
    let a = rgb_to_ycbcr(from_working(textureLoad(src0, vec2(p.x * 2, p.y * 2), 0)).rgb);
    let b = rgb_to_ycbcr(from_working(textureLoad(src0, vec2(p.x * 2, min(p.y * 2 + 1, h - 1)), 0)).rgb);
    return (a.yz + b.yz) * 0.5;
}

@fragment
fn fs_to_x420_chroma(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let c = chroma420(p);
    let cb = code_d(c.x * 896.0 + 512.0, 4.0, 1019.0, p, 1u);
    let cr = code_d(c.y * 896.0 + 512.0, 4.0, 1019.0, p, 2u);
    return vec4(cb * 64.0 / 65535.0, cr * 64.0 / 65535.0, 0.0, 1.0);
}

// 8-bit limited range ('420v'): codable range 1..254.
@fragment
fn fs_to_420v_luma(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let w = from_working(textureLoad(src0, p, 0));
    let ycc = rgb_to_ycbcr(w.rgb);
    let y = code_d(ycc.x * 219.0 + 16.0, 1.0, 254.0, p, 0u);
    return vec4(y / 255.0, 0.0, 0.0, 1.0);
}

@fragment
fn fs_to_420v_chroma(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let c = chroma420(p);
    let cb = code_d(c.x * 224.0 + 128.0, 1.0, 254.0, p, 1u);
    let cr = code_d(c.y * 224.0 + 128.0, 1.0, 254.0, p, 2u);
    return vec4(cb / 255.0, cr / 255.0, 0.0, 1.0);
}

@fragment
fn fs_to_rgba16(in: VsOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(in.pos.xy);
    let w = from_working(textureLoad(src0, p, 0));
    let d = vec4(dither_offset(p, 0u), dither_offset(p, 1u), dither_offset(p, 2u), 0.0);
    let c = clamp(round(vec4(w.rgb, w.a) * 65535.0 + d), vec4(0.0), vec4(65535.0));
    return c / 65535.0;
}
