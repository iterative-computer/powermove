//! The nearest-even binary16 of a computed value, before a half-float
//! render-target store.
//!
//! Metal's render-target conversion (and `quantizeToF16`) rounds f32 toward
//! zero, which biases every stored half down by up to one ulp. Shaders that
//! write RGBA16F or R16Float targets round the f32 bit pattern explicitly
//! first; the store is then exact. Hardware-blended draws still round their
//! blend result in hardware.
//!
//! This is the one copy of the conversion: the composite, sampling, 3D,
//! effect, vector, path-mask, SVG and text shaders all include it.

pub(crate) const NEAREST_HALF_WGSL: &str = r#"
fn pm_scene_nearest_half(value: f32) -> f32 {
    let bits = bitcast<u32>(value);
    let sign_bit = bits & 0x80000000u;
    let magnitude = bits & 0x7fffffffu;
    let exponent = magnitude >> 23u;
    if (exponent == 255u) { return value; }
    if (exponent > 142u) { return bitcast<f32>(sign_bit | 0x7f800000u); }
    if (exponent < 102u) { return bitcast<f32>(sign_bit); }
    if (exponent < 113u) {
        let significand = (magnitude & 0x007fffffu) | 0x00800000u;
        let shift = 126u - exponent;
        var kept = significand >> shift;
        let lost = significand & ((1u << shift) - 1u);
        let halfway = 1u << (shift - 1u);
        if (lost > halfway || (lost == halfway && (kept & 1u) != 0u)) { kept += 1u; }
        let rounded = f32(kept) * 5.9604644775390625e-8;
        return bitcast<f32>(bitcast<u32>(rounded) | sign_bit);
    }
    var kept = magnitude >> 13u;
    let lost = magnitude & 0x1fffu;
    if (lost > 0x1000u || (lost == 0x1000u && (kept & 1u) != 0u)) { kept += 1u; }
    var rounded = kept << 13u;
    if (rounded >= 0x47800000u) { rounded = 0x7f800000u; }
    return bitcast<f32>(sign_bit | rounded);
}

fn pm_scene_half_color(color: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(pm_scene_nearest_half(color.r),
                     pm_scene_nearest_half(color.g),
                     pm_scene_nearest_half(color.b),
                     pm_scene_nearest_half(color.a));
}
"#;
