// Derived from Skia, Copyright 2018, 2019 Google LLC.
// BSD 3-Clause license: video_upload.LICENSE.
// Chromium 152 Skia pin b6d106297ff9ef2ff8094033695d045e87775581:
// GrYUVtoRGBEffect.cpp, SkYUVMath.cpp and src/opts/SkRasterPipeline_opts.h.
//
// Skia's compatibility video upload: filter raw unorm planes first, then
// apply its affine matrix. Native normalized YCbCr uses source.wgsl.
struct Params {
    rows: array<vec4<f32>, 3>,
    primaries: array<vec4<f32>, 3>,
    // planar, direct BT.2020 accumulation, reserved, biplanar alpha.
    flags: vec4<u32>,
};
@group(0) @binding(0) var y_plane: texture_2d<f32>;
@group(0) @binding(1) var uv_plane: texture_2d<f32>;
@group(0) @binding(2) var v_plane: texture_2d<f32>;
@group(0) @binding(3) var linear_sampler: sampler;
@group(0) @binding(4) var<uniform> P: Params;
@group(0) @binding(5) var out: texture_storage_2d<rgba8unorm, write>;
@group(0) @binding(6) var<storage, read> decode_lut: array<f32, 256>;
@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(out);
    if (any(id.xy >= size)) { return; }
    // Software decoder planes can include coded rows beyond the displayed
    // crop. Filter over that coded extent before cropping the RGB result.
    let coord = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(textureDimensions(y_plane));
    let y = textureLoad(y_plane, vec2<i32>(id.xy), 0).r;
    var uv = textureSampleLevel(uv_plane, linear_sampler, coord, 0.0).rg;
    if (P.flags.x != 0u) {
        uv.y = textureSampleLevel(v_plane, linear_sampler, coord, 0.0).r;
    }
    let c0 = vec3<f32>(P.rows[0].x, P.rows[1].x, P.rows[2].x);
    let c1 = vec3<f32>(P.rows[0].y, P.rows[1].y, P.rows[2].y);
    let c2 = vec3<f32>(P.rows[0].z, P.rows[1].z, P.rows[2].z);
    let bias = vec3<f32>(P.rows[0].w, P.rows[1].w, P.rows[2].w);
    var rgb = fma(vec3<f32>(uv.y), c2,
        fma(vec3<f32>(uv.x), c1, fma(vec3<f32>(y), c0, bias)));
    if (P.flags.y != 0u) {
        // Direct shared-image conversion applies the translation after the
        // matrix product; the Canvas hardware source has a different fused
        // accumulation. These differ at exact half-code boundaries.
        rgb = fma(vec3<f32>(y), c0, fma(vec3<f32>(uv.x), c1, uv.y * c2)) + bias;
    }
    rgb = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    var alpha = 1.0;
    if (P.flags.w != 0u) {
        alpha = textureLoad(v_plane, vec2<i32>(id.xy), 0).r;
        rgb *= alpha;
    }
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(rgb, alpha));
}

@compute @workgroup_size(8, 8)
fn gamut(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(out);
    if (any(id.xy >= size)) { return; }
    // Match Canvas readPixels after the GPU's real RGBA8 source store.
    let code = vec3<u32>(round(textureLoad(y_plane, vec2<i32>(id.xy), 0).rgb * 255.0));
    let l = vec3<f32>(decode_lut[code.r], decode_lut[code.g], decode_lut[code.b]);
    let transformed = fma(vec3<f32>(l.r), P.primaries[0].xyz,
        fma(vec3<f32>(l.g), P.primaries[1].xyz, l.b * P.primaries[2].xyz));
    let rgb = round(clamp(sk_encode(transformed), vec3<f32>(0.0), vec3<f32>(1.0)) * 255.0) * (1.0 / 255.0);
    textureStore(out, vec2<i32>(id.xy), vec4<f32>(rgb, 1.0));
}

fn sk_pow(v: f32, exponent: f32) -> f32 {
    if (v == 0.0 || v == 1.0) { return v; }
    let bits = bitcast<u32>(v);
    let e = f32(bits) * (1.0 / 8388608.0);
    let m = bitcast<f32>((bits & 0x007fffffu) | 0x3f000000u);
    let lg = fma(-m, 1.498030302, e - 124.225514990) - precise_div(1.725879990, 0.3520887068 + m);
    let x = lg * exponent;
    let f = fract(x);
    let b = (fma(-f, 1.490129070, x + 121.274057500) + precise_div(27.728023300, 4.84252568 - f)) * 8388608.0;
    return bitcast<f32>(u32(clamp(b, 0.0, 2139095040.0)));
}
fn sk_encode(c: vec3<f32>) -> vec3<f32> {
    let m = abs(c);
    let v = vec3<f32>(sk_pow(m.r * 1.137283325, 0.4166666567), sk_pow(m.g * 1.137283325, 0.4166666567), sk_pow(m.b * 1.137283325, 0.4166666567)) - 0.054969788;
    return sign(c) * select(v, m * 12.92, m <= vec3<f32>(0.003130805));
}

// Exact positive-normal f32 division for Skia's two approximation terms.
// GPU division permits a few ulps of error; here they affect a float bitcast
// in pow2 and can change the final byte. The 24-bit remainder fits in u32.
fn precise_div(n: f32, d: f32) -> f32 {
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
