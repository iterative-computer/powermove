// Source conversion (plan sections 9 and 13): a decoded frame or image,
// already normalized by pm-gpu's decode-in pass, to premultiplied working
// values at the source's size.
//
// - Y′CbCr: siting-aware chroma upsampling, exact at co-sited samples
//   (pm_color::chroma): horizontal linear for 4:2:2, Catmull-Rom
//   horizontally and linear vertically for 4:2:0; then the matrix.
// - Transfer and primaries into the working encoding. In Standard an
//   E-encoded Rec.709 source is the identity: code values pass through.
//   HDR sources (transfer 5 PQ, 6 HLG) are tone mapped with BT.2446 Method A
//   to Rec.709 SDR code values, which Standard reads as E
//   (pm_color::tonemap; `PM_TONEMAP` is prepended too).
// - Alpha by pm_color::alpha's plan.
//
// `PM_COLOR` (pm_color::wgsl::COLOR) is prepended; `OUT_FORMAT` is replaced
// with the working storage format.

struct Params {
    // kind (0 RGB, 1 Y′CbCr), transfer, working (0 Standard, 1 linear), alpha plan.
    a: vec4<u32>,
    // chroma factors x, y; horizontal and vertical filters (0 none, 1 linear, 2 Catmull-Rom).
    b: vec4<u32>,
    // chroma plane width, height; quantize to f16; convert primaries.
    c: vec4<u32>,
    // chroma origin on the luma grid (xy).
    origin: vec4<f32>,
    // Y′CbCr → R′G′B′ columns.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // Primaries conversion columns (linear light).
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    // Matte color for "premultiplied with color".
    matte: vec4<f32>,
    // HDR tone mapping: x the content peak in cd/m².
    hdr: vec4<f32>,
};

@group(0) @binding(0) var luma: texture_2d<f32>;
@group(0) @binding(1) var chroma: texture_2d<f32>;
@group(0) @binding(2) var<uniform> P: Params;
@group(0) @binding(3) var out: texture_storage_2d<OUT_FORMAT, write>;

fn chroma_at(i: i32, j: i32) -> vec2<f32> {
    let w = i32(P.c.x);
    let h = i32(P.c.y);
    return textureLoad(chroma, vec2<i32>(clamp(i, 0, w - 1), clamp(j, 0, h - 1)), 0).rg;
}

fn catmull_rom(t: f32) -> vec4<f32> {
    let t2 = t * t;
    let t3 = t2 * t;
    return vec4<f32>(
        (-t3 + 2.0 * t2 - t) * 0.5,
        (3.0 * t3 - 5.0 * t2 + 2.0) * 0.5,
        (-3.0 * t3 + 4.0 * t2 + t) * 0.5,
        (t3 - t2) * 0.5,
    );
}

// One chroma row j sampled horizontally at luma column x.
fn chroma_row(x: u32, j: i32) -> vec2<f32> {
    let kind = P.b.z;
    if (kind == 0u) {
        return chroma_at(i32(x), j);
    }
    let u = (f32(x) - P.origin.x) / f32(P.b.x);
    let fi = floor(u);
    let t = u - fi;
    let i = i32(fi);
    if (kind == 1u) {
        if (t == 0.0) {
            return chroma_at(i, j);
        }
        return chroma_at(i, j) * (1.0 - t) + chroma_at(i + 1, j) * t;
    }
    if (t == 0.0) {
        return chroma_at(i, j);
    }
    let w = catmull_rom(t);
    return chroma_at(i - 1, j) * w.x + chroma_at(i, j) * w.y + chroma_at(i + 1, j) * w.z
        + chroma_at(i + 2, j) * w.w;
}

fn chroma_sample(x: u32, y: u32) -> vec2<f32> {
    if (P.b.w == 0u) {
        return chroma_row(x, i32(y));
    }
    let v = (f32(y) - P.origin.y) / f32(P.b.y);
    let fj = floor(v);
    let t = v - fj;
    let j = i32(fj);
    if (t == 0.0) {
        return chroma_row(x, j);
    }
    return chroma_row(x, j) * (1.0 - t) + chroma_row(x, j + 1) * t;
}

fn transfer_to_linear(c: vec3<f32>) -> vec3<f32> {
    switch P.a.y {
        case 0u: {
            return pm_e_decode3(c);
        }
        case 1u: {
            // BT.1886, zero black level: gamma 2.4, odd-symmetric.
            return sign(c) * pow(abs(c), vec3<f32>(2.4));
        }
        case 2u: {
            // Inverse BT.709 OETF, odd-symmetric.
            let m = abs(c);
            let lo = m / 4.5;
            let hi = pow((m + 0.099) / 1.099, vec3<f32>(1.0 / 0.45));
            return sign(c) * select(hi, lo, m < vec3<f32>(0.081));
        }
        case 4u: {
            return sign(c) * pow(abs(c), vec3<f32>(2.2));
        }
        default: {
            return c;
        }
    }
}

// The input transform: source encoding and primaries to the working encoding.
fn to_working(c: vec3<f32>) -> vec3<f32> {
    let identity = P.a.y == 0u && P.a.z == 0u && P.c.w == 0u;
    if (identity) {
        return c;
    }
    if (P.a.y == 5u || P.a.y == 6u) {
        // BT.2020 HDR signal to Rec.709 SDR codes (E in Standard); the
        // primaries columns hold BT.2020 to Rec.709.
        let sdr = pm_hdr_to_sdr(c, P.a.y == 6u, P.hdr.x, mat3x3<f32>(P.p0.xyz, P.p1.xyz, P.p2.xyz));
        if (P.a.z == 0u) {
            return sdr;
        }
        return pm_e_decode3(sdr);
    }
    var l = transfer_to_linear(c);
    if (P.c.w == 1u) {
        l = mat3x3<f32>(P.p0.xyz, P.p1.xyz, P.p2.xyz) * l;
    }
    if (P.a.z == 0u) {
        return pm_e_encode3(l);
    }
    return l;
}

fn transform_premultiplied(c: vec4<f32>) -> vec4<f32> {
    if (!pm_may_unpremultiply(c.a)) {
        // Below the floor (and additive pixels): kept as they are.
        return c;
    }
    let s = c.rgb / c.a;
    return vec4<f32>(to_working(s) * c.a, c.a);
}

// BT.2020 constant luminance, H.273 matrix 10 (BT.2020-2, table 4).
// Recover G in linear light, then return R'G'B' to the source transfer
// reading. The piecewise chroma scale cannot be expressed by P.m0..m2.
fn bt2020_cl_decode(v: f32) -> f32 {
    let m = abs(v);
    var l: f32;
    if (m < 0.0812428582986315) { l = m / 4.5; }
    else { l = pow((m + 0.09929682680944) / 1.09929682680944, 1.0 / 0.45); }
    return select(l, -l, v < 0.0);
}
fn bt2020_cl_encode(v: f32) -> f32 {
    let m = abs(v);
    var e: f32;
    if (m < 0.018053968510807) { e = 4.5 * m; }
    else { e = 1.09929682680944 * pow(m, 0.45) - 0.09929682680944; }
    return select(e, -e, v < 0.0);
}
fn bt2020_cl_rgb(ycc: vec3<f32>) -> vec3<f32> {
    let b = ycc.x + ycc.y * select(1.5816, 1.9404, ycc.y < 0.0);
    let r = ycc.x + ycc.z * select(0.9936, 1.7184, ycc.z < 0.0);
    let g = (bt2020_cl_decode(ycc.x) - 0.2627 * bt2020_cl_decode(r)
        - 0.0593 * bt2020_cl_decode(b)) / 0.6780;
    return vec3<f32>(r, bt2020_cl_encode(g), b);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let dims = textureDimensions(out);
    if (id.x >= dims.x || id.y >= dims.y) {
        return;
    }
    let xy = vec2<i32>(id.xy);
    let v = textureLoad(luma, xy, 0);
    var stored: vec4<f32>;
    if (P.a.x == 1u) {
        let cbcr = chroma_sample(id.x, id.y);
        let m = mat3x3<f32>(P.m0.xyz, P.m1.xyz, P.m2.xyz);
        let ycc = vec3<f32>(v.r, cbcr.x, cbcr.y);
        if (P.hdr.y == 1.0) { stored = vec4<f32>(bt2020_cl_rgb(ycc), v.a); }
        else { stored = vec4<f32>(m * ycc, v.a); }
    } else {
        stored = v;
    }
    var w: vec4<f32>;
    switch P.a.w {
        case 0u: {
            w = vec4<f32>(to_working(stored.rgb), 1.0);
        }
        case 1u: {
            w = vec4<f32>(to_working(stored.rgb) * stored.a, stored.a);
        }
        case 2u: {
            w = stored;
        }
        case 3u: {
            w = transform_premultiplied(stored);
        }
        default: {
            let removed = vec4<f32>(stored.rgb - (1.0 - stored.a) * P.matte.rgb, stored.a);
            let identity = P.a.y == 0u && P.a.z == 0u && P.c.w == 0u;
            w = select(transform_premultiplied(removed), removed, identity);
        }
    }
    if (P.c.z == 1u) {
        w = vec4<f32>(quantizeToF16(w.r), quantizeToF16(w.g), quantizeToF16(w.b), quantizeToF16(w.a));
    }
    textureStore(out, xy, w);
}
