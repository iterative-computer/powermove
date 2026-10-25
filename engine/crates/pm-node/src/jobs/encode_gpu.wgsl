@group(0) @binding(0) var source: texture_2d<f32>;
@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {
    let p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    return vec4(p[i], 0.0, 1.0);
}

// Round the exact binary32 value times 255, ties to even, as the CPU's f64
// quantizer does. A 24-bit significand times 255 fits in u32; no f64 WGSL,
// fused multiply-add, or near-half epsilon is needed.
fn rgba8_code(x: f32) -> u32 {
    let bits = bitcast<u32>(x);
    // An all-one exponent with a nonzero fraction is NaN (either sign).
    if ((bits & 0x7fffffffu) > 0x7f800000u || x <= 0.0) { return 0u; }
    if (x >= 1.0) { return 255u; }

    let exponent = (bits >> 23u) & 255u;
    var significand = bits & 0x7fffffu;
    var shift = 149u; // subnormal: significand × 2^-149
    if (exponent != 0u) {
        significand |= 0x800000u;
        shift = 150u - exponent; // normal: significand × 2^(exponent-150)
    }
    let product = significand * 255u;
    if (shift > 32u) { return 0u; }
    if (shift == 32u) { return select(0u, 1u, product > 0x80000000u); }
    if (shift == 0u) { return min(product, 255u); }

    let whole = product >> shift;
    let remainder = product & ((1u << shift) - 1u);
    let half = 1u << (shift - 1u);
    let up = remainder > half || (remainder == half && (whole & 1u) != 0u);
    return min(whole + select(0u, 1u, up), 255u);
}

@fragment fn fs(@builtin(position) p: vec4<f32>) -> @location(0) vec4<f32> {
    let v = textureLoad(source, vec2<i32>(p.xy), 0);
    // Match the RGBA8 compositor attachment / exact CPU ties-even boundary.
    // Opaque WebM composites premultiplied RGB over black and ignores alpha.
    let mapped = pm_css_gamut_srgb(v.rgb);
    let codes = vec3<u32>(rgba8_code(mapped.r), rgba8_code(mapped.g), rgba8_code(mapped.b));
    return vec4(vec3<f32>(codes) / 255.0, 1.0);
}
