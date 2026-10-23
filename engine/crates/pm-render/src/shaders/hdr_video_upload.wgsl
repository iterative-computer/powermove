// Derived from Skia (Google LLC), BSD 3-Clause; see hdr_video_upload.LICENSE.
// Chromium 152 / Skia b6d1062 default PQ/HLG -> SDR Canvas color path.
// The source is the RGBA8 Skia video upload, including its real byte store.
// Curve points are Skia's eight RWTMO Hermite points for 1000/203 nits.
// SkRasterPipeline from_byte: cast(u8) * binary32(1/255), pre-rounded.
// All 256 possible channel values are covered; no content-specific entries.
const FROM_BYTE: array<u32, 256> = array<u32, 256>(
    0x00000000u, 0x3b808081u, 0x3c008081u, 0x3c40c0c2u, 0x3c808081u, 0x3ca0a0a1u, 0x3cc0c0c2u, 0x3ce0e0e2u,
    0x3d008081u, 0x3d109091u, 0x3d20a0a1u, 0x3d30b0b1u, 0x3d40c0c2u, 0x3d50d0d2u, 0x3d60e0e2u, 0x3d70f0f2u,
    0x3d808081u, 0x3d888889u, 0x3d909091u, 0x3d989899u, 0x3da0a0a1u, 0x3da8a8a9u, 0x3db0b0b1u, 0x3db8b8b9u,
    0x3dc0c0c2u, 0x3dc8c8cau, 0x3dd0d0d2u, 0x3dd8d8dau, 0x3de0e0e2u, 0x3de8e8eau, 0x3df0f0f2u, 0x3df8f8fau,
    0x3e008081u, 0x3e048485u, 0x3e088889u, 0x3e0c8c8du, 0x3e109091u, 0x3e149495u, 0x3e189899u, 0x3e1c9c9du,
    0x3e20a0a1u, 0x3e24a4a5u, 0x3e28a8a9u, 0x3e2cacadu, 0x3e30b0b1u, 0x3e34b4b5u, 0x3e38b8b9u, 0x3e3cbcbdu,
    0x3e40c0c2u, 0x3e44c4c6u, 0x3e48c8cau, 0x3e4cccceu, 0x3e50d0d2u, 0x3e54d4d6u, 0x3e58d8dau, 0x3e5cdcdeu,
    0x3e60e0e2u, 0x3e64e4e6u, 0x3e68e8eau, 0x3e6ceceeu, 0x3e70f0f2u, 0x3e74f4f6u, 0x3e78f8fau, 0x3e7cfcfeu,
    0x3e808081u, 0x3e828283u, 0x3e848485u, 0x3e868687u, 0x3e888889u, 0x3e8a8a8bu, 0x3e8c8c8du, 0x3e8e8e8fu,
    0x3e909091u, 0x3e929293u, 0x3e949495u, 0x3e969697u, 0x3e989899u, 0x3e9a9a9bu, 0x3e9c9c9du, 0x3e9e9e9fu,
    0x3ea0a0a1u, 0x3ea2a2a3u, 0x3ea4a4a5u, 0x3ea6a6a7u, 0x3ea8a8a9u, 0x3eaaaaabu, 0x3eacacadu, 0x3eaeaeafu,
    0x3eb0b0b1u, 0x3eb2b2b3u, 0x3eb4b4b5u, 0x3eb6b6b7u, 0x3eb8b8b9u, 0x3ebababbu, 0x3ebcbcbdu, 0x3ebebebfu,
    0x3ec0c0c2u, 0x3ec2c2c4u, 0x3ec4c4c6u, 0x3ec6c6c8u, 0x3ec8c8cau, 0x3ecacaccu, 0x3eccccceu, 0x3ececed0u,
    0x3ed0d0d2u, 0x3ed2d2d4u, 0x3ed4d4d6u, 0x3ed6d6d8u, 0x3ed8d8dau, 0x3edadadcu, 0x3edcdcdeu, 0x3ededee0u,
    0x3ee0e0e2u, 0x3ee2e2e4u, 0x3ee4e4e6u, 0x3ee6e6e8u, 0x3ee8e8eau, 0x3eeaeaecu, 0x3eececeeu, 0x3eeeeef0u,
    0x3ef0f0f2u, 0x3ef2f2f4u, 0x3ef4f4f6u, 0x3ef6f6f8u, 0x3ef8f8fau, 0x3efafafcu, 0x3efcfcfeu, 0x3efeff00u,
    0x3f008081u, 0x3f018182u, 0x3f028283u, 0x3f038384u, 0x3f048485u, 0x3f058586u, 0x3f068687u, 0x3f078788u,
    0x3f088889u, 0x3f09898au, 0x3f0a8a8bu, 0x3f0b8b8cu, 0x3f0c8c8du, 0x3f0d8d8eu, 0x3f0e8e8fu, 0x3f0f8f90u,
    0x3f109091u, 0x3f119192u, 0x3f129293u, 0x3f139394u, 0x3f149495u, 0x3f159596u, 0x3f169697u, 0x3f179798u,
    0x3f189899u, 0x3f19999au, 0x3f1a9a9bu, 0x3f1b9b9cu, 0x3f1c9c9du, 0x3f1d9d9eu, 0x3f1e9e9fu, 0x3f1f9fa0u,
    0x3f20a0a1u, 0x3f21a1a2u, 0x3f22a2a3u, 0x3f23a3a4u, 0x3f24a4a5u, 0x3f25a5a6u, 0x3f26a6a7u, 0x3f27a7a8u,
    0x3f28a8a9u, 0x3f29a9aau, 0x3f2aaaabu, 0x3f2babacu, 0x3f2cacadu, 0x3f2dadaeu, 0x3f2eaeafu, 0x3f2fafb0u,
    0x3f30b0b1u, 0x3f31b1b2u, 0x3f32b2b3u, 0x3f33b3b4u, 0x3f34b4b5u, 0x3f35b5b6u, 0x3f36b6b7u, 0x3f37b7b8u,
    0x3f38b8b9u, 0x3f39b9bau, 0x3f3ababbu, 0x3f3bbbbcu, 0x3f3cbcbdu, 0x3f3dbdbeu, 0x3f3ebebfu, 0x3f3fbfc0u,
    0x3f40c0c2u, 0x3f41c1c3u, 0x3f42c2c4u, 0x3f43c3c5u, 0x3f44c4c6u, 0x3f45c5c7u, 0x3f46c6c8u, 0x3f47c7c9u,
    0x3f48c8cau, 0x3f49c9cbu, 0x3f4acaccu, 0x3f4bcbcdu, 0x3f4cccceu, 0x3f4dcdcfu, 0x3f4eced0u, 0x3f4fcfd1u,
    0x3f50d0d2u, 0x3f51d1d3u, 0x3f52d2d4u, 0x3f53d3d5u, 0x3f54d4d6u, 0x3f55d5d7u, 0x3f56d6d8u, 0x3f57d7d9u,
    0x3f58d8dau, 0x3f59d9dbu, 0x3f5adadcu, 0x3f5bdbddu, 0x3f5cdcdeu, 0x3f5ddddfu, 0x3f5edee0u, 0x3f5fdfe1u,
    0x3f60e0e2u, 0x3f61e1e3u, 0x3f62e2e4u, 0x3f63e3e5u, 0x3f64e4e6u, 0x3f65e5e7u, 0x3f66e6e8u, 0x3f67e7e9u,
    0x3f68e8eau, 0x3f69e9ebu, 0x3f6aeaecu, 0x3f6bebedu, 0x3f6ceceeu, 0x3f6dedefu, 0x3f6eeef0u, 0x3f6feff1u,
    0x3f70f0f2u, 0x3f71f1f3u, 0x3f72f2f4u, 0x3f73f3f5u, 0x3f74f4f6u, 0x3f75f5f7u, 0x3f76f6f8u, 0x3f77f7f9u,
    0x3f78f8fau, 0x3f79f9fbu, 0x3f7afafcu, 0x3f7bfbfdu, 0x3f7cfcfeu, 0x3f7dfdffu, 0x3f7eff00u, 0x3f800000u,
);

struct Params {
    source_to_srgb: array<vec4<f32>, 3>,
    srgb_to_2020: array<vec4<f32>, 3>,
    to_709: array<vec4<f32>, 3>,
    curve: array<vec4<f32>, 8>,
    hlg_y: vec4<f32>,
    signal: vec4<u32>,
};
@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var<uniform> P: Params;
@group(0) @binding(2) var output: texture_storage_2d<rgba8unorm, write>;

fn pq_to_light(code: f32) -> f32 {
    let p = sk_pow(max(code, 0.0), 32.0 / 2523.0);
    let numerator = max(fma(p, 1.0, -107.0 / 128.0), 0.0);
    let denominator = fma(p, -2392.0 / 128.0, 2413.0 / 128.0);
    return sk_pow(precise_div(numerator, denominator), 8192.0 / 1305.0);
}

fn hlg_to_scene(code: f32) -> f32 {
    let k = bitcast<f32>(0x3daaaaa8u);
    if (code <= 0.5) { return sk_pow(code * 2.0, 2.0) * k; }
    return (sk_exp((code - 0.55991073) * bitcast<f32>(0x40b2f029u)) + 0.28466892) * k;
}

fn hermite_gain(max_rgb: f32) -> f32 {
    let first = P.curve[0];
    let last = P.curve[7];
    if (max_rgb <= first.x) { return first.y; }
    if (max_rgb >= last.x) { return last.y + sk_log2(last.x / max_rgb); }
    var lo = 0u;
    var hi = 7u;
    for (var step = 0u; step < 3u; step += 1u) {
        let mid = (lo + hi + 1u) / 2u;
        if (max_rgb < P.curve[mid].x) { hi = mid; }
        else { lo = mid; }
    }
    let a = P.curve[lo];
    let b = P.curve[hi];
    let h = b.x - a.x;
    let ma = a.z * h;
    let mb = b.z * h;
    let t = (max_rgb - a.x) / h;
    let c3 = 2.0 * a.y + ma - 2.0 * b.y + mb;
    let c2 = -3.0 * a.y + 3.0 * b.y - 2.0 * ma - mb;
    return ((c3 * t + c2) * t + ma) * t + a.y;
}

fn decode_srgb(code: f32) -> f32 {
    if (abs(code) <= 0.04045) { return (1.0 / 12.92) * code; }
    return sign(code) * sk_pow(fma(abs(code), 1.0 / 1.055, 0.055 / 1.055), 2.4);
}
fn encode_srgb(light: f32) -> f32 {
    if (abs(light) <= 0.003130805) { return 12.92 * light; }
    return sign(light) * (sk_pow(abs(light) * 1.137283325, 0.4166666567) - 0.054969788);
}

// SkRasterPipeline_opts.h's ARM64 vector approximations. The positive-normal
// divisions need binary32 rounding before their results enter float bitcasts.
fn sk_log2(v: f32) -> f32 {
    let bits = bitcast<u32>(v);
    let e = f32(bits) * (1.0 / 8388608.0);
    let m = bitcast<f32>((bits & 0x007fffffu) | 0x3f000000u);
    return fma(-m, 1.498030302, e - 124.225514990) - precise_div(1.725879990, 0.3520887068 + m);
}
fn sk_pow2(x: f32) -> f32 {
    let f = fract(x);
    let b = (fma(-f, 1.490129070, x + 121.274057500) + precise_div(27.728023300, 4.84252568 - f)) * 8388608.0;
    return bitcast<f32>(u32(round(clamp(b, 0.0, 2139095040.0))));
}
fn sk_pow(v: f32, exponent: f32) -> f32 {
    if (v == 0.0 || v == 1.0) { return v; }
    return sk_pow2(sk_log2(v) * exponent);
}
fn sk_exp(x: f32) -> f32 { return sk_pow2(1.4426950408889634 * x); }

fn precise_div(n: f32, d: f32) -> f32 {
    if (n == 0.0) { return 0.0; }
    let nb = bitcast<u32>(n);
    let db = bitcast<u32>(d);
    var exponent = i32((nb >> 23u) & 255u) - i32((db >> 23u) & 255u) + 127;
    var rem = (nb & 0x7fffffu) | 0x800000u;
    let den = (db & 0x7fffffu) | 0x800000u;
    if (rem < den) { rem <<= 1u; exponent -= 1; }
    rem -= den;
    var q = 0x800000u;
    for (var i = 22; i >= 0; i -= 1) {
        rem <<= 1u;
        if (rem >= den) { rem -= den; q |= 1u << u32(i); }
    }
    if (rem * 2u > den || (rem * 2u == den && (q & 1u) != 0u)) { q += 1u; }
    if (q == 0x1000000u) { q >>= 1u; exponent += 1; }
    return bitcast<f32>((u32(exponent) << 23u) | (q & 0x7fffffu));
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if (any(id.xy >= textureDimensions(output))) { return; }
    let sampled = textureLoad(source, vec2<i32>(id.xy), 0).rgb;
    var code = sampled;
    if (P.signal.x != 0u) {
        let byte = vec3<u32>(round(sampled * 255.0));
        code = vec3<f32>(bitcast<f32>(FROM_BYTE[byte.r]),
                         bitcast<f32>(FROM_BYTE[byte.g]),
                         bitcast<f32>(FROM_BYTE[byte.b]));
    }
    var light: vec3<f32>;
    if (P.signal.x == 0u) {
        light = vec3<f32>(pq_to_light(code.r), pq_to_light(code.g), pq_to_light(code.b));
    } else {
        let scene = vec3<f32>(hlg_to_scene(code.r), hlg_to_scene(code.g), hlg_to_scene(code.b));
        let y = max(fma(scene.x, P.hlg_y.x,
            fma(scene.y, P.hlg_y.y, scene.z * P.hlg_y.z)), 0.0);
        light = scene * sk_pow(y, bitcast<f32>(0x3e4cccd0u));
    }
    let rgb709_encoded_linear = fma(vec3<f32>(light.r), P.source_to_srgb[0].xyz,
        fma(vec3<f32>(light.g), P.source_to_srgb[1].xyz,
            light.b * P.source_to_srgb[2].xyz));
    let srgb_code = vec3<f32>(encode_srgb(rgb709_encoded_linear.r),
        encode_srgb(rgb709_encoded_linear.g), encode_srgb(rgb709_encoded_linear.b));
    let srgb_light = vec3<f32>(decode_srgb(srgb_code.r),
        decode_srgb(srgb_code.g), decode_srgb(srgb_code.b));
    let rgb2020 = fma(vec3<f32>(srgb_light.r), P.srgb_to_2020[0].xyz,
        fma(vec3<f32>(srgb_light.g), P.srgb_to_2020[1].xyz,
            srgb_light.b * P.srgb_to_2020[2].xyz));
    let max_rgb = max(max(rgb2020.r, rgb2020.g), rgb2020.b);
    let mapped = rgb2020 * sk_pow2(hermite_gain(max_rgb));
    let rgb709 = fma(vec3<f32>(mapped.r), P.to_709[0].xyz,
        fma(vec3<f32>(mapped.g), P.to_709[1].xyz, mapped.b * P.to_709[2].xyz));
    let encoded = vec3<f32>(encode_srgb(rgb709.r), encode_srgb(rgb709.g), encode_srgb(rgb709.b));
    textureStore(output, vec2<i32>(id.xy), vec4<f32>(round(clamp(encoded, vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) * (1.0 / 255.0), 1.0));
}
