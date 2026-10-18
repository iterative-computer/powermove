//! WGSL source shared by every shader that encodes or decodes E (section
//! 9). One definition keeps the GPU and [`crate::transfer`]'s f32 functions
//! on the same formula; `pm-render`'s GPU tests compare the two.
//!
//! Prepend [`COLOR`] to a WGSL module to get:
//!
//! - `pm_e_encode(x: f32) -> f32`, `pm_e_encode3(c: vec3<f32>)`: E, extended;
//! - `pm_e_decode(x: f32) -> f32`, `pm_e_decode3`: E⁻¹, extended;
//! - `pm_luma709(c: vec3<f32>) -> f32`: Rec.709 luma (luma mattes);
//! - `pm_premultiply(c: vec4<f32>)`, `pm_unpremultiply(c: vec4<f32>)`: the
//!   section 9 rules with the 1/4096 guard (pixels below it are kept);
//! - `pm_premultiplied_e_to_linear(c)`, `pm_premultiplied_linear_to_e(c)`:
//!   [`crate::linear::premultiplied_e_to_linear`] and its inverse.
//!
//! [`BLEND`] adds the nine blend modes in both domains.

/// The shared WGSL functions.
pub const COLOR: &str = r#"
const PM_E_LINEAR_BREAK: f32 = 0.0031308;
const PM_E_ENCODED_BREAK: f32 = 0.04045;
const PM_UNPREMULTIPLY_FLOOR: f32 = 0.000244140625;

fn pm_e_encode(x: f32) -> f32 {
    let m = abs(x);
    var e: f32;
    if (m <= PM_E_LINEAR_BREAK) {
        e = 12.92 * m;
    } else {
        e = 1.055 * pow(m, 1.0 / 2.4) - 0.055;
    }
    return select(e, -e, x < 0.0);
}

fn pm_e_decode(x: f32) -> f32 {
    let m = abs(x);
    var l: f32;
    if (m <= PM_E_ENCODED_BREAK) {
        l = m / 12.92;
    } else {
        l = pow((m + 0.055) / 1.055, 2.4);
    }
    return select(l, -l, x < 0.0);
}

fn pm_e_encode3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(pm_e_encode(c.x), pm_e_encode(c.y), pm_e_encode(c.z));
}

fn pm_e_decode3(c: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(pm_e_decode(c.x), pm_e_decode(c.y), pm_e_decode(c.z));
}

fn pm_luma709(c: vec3<f32>) -> f32 {
    return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

fn pm_premultiply(c: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(c.rgb * c.a, c.a);
}

fn pm_may_unpremultiply(a: f32) -> bool {
    return a >= PM_UNPREMULTIPLY_FLOOR;
}

fn pm_unpremultiply(c: vec4<f32>) -> vec4<f32> {
    if (pm_may_unpremultiply(c.a)) {
        return vec4<f32>(c.rgb / c.a, c.a);
    }
    return c;
}

fn pm_premultiplied_e_to_linear(c: vec4<f32>) -> vec4<f32> {
    if (pm_may_unpremultiply(c.a)) {
        return vec4<f32>(pm_e_decode3(c.rgb / c.a) * c.a, c.a);
    }
    return c;
}

fn pm_premultiplied_linear_to_e(c: vec4<f32>) -> vec4<f32> {
    if (pm_may_unpremultiply(c.a)) {
        return vec4<f32>(pm_e_encode3(c.rgb / c.a) * c.a, c.a);
    }
    return c;
}
"#;

/// WGSL for the nine blend modes (section 9, "Blend modes and mattes"),
/// to be prepended after [`COLOR`]. Mode numbers are today's `BLEND_ID`
/// (0 normal, 1 add, 2 screen, 3 multiply, 4 overlay, 5 soft light,
/// 6 difference, 7 lighten, 8 darken):
///
/// - `pm_blend_legacy(b, s, m)`: today's `FRAG_COMPOSITE` `blend()`,
///   statement for statement (Standard keeps it);
/// - `pm_blend_hdr_safe(b, s, m)`: screen `b + s − bs` unclamped; overlay
///   and soft light switch on values clamped to 0..1 and do not clamp
///   otherwise; every other mode as `pm_blend_legacy`;
/// - `pm_blend_working(b, s, m, linear, in_e)`: the mode in a project's
///   setup: Standard runs the legacy formula on E values; linear light runs
///   normal, add, multiply, difference, lighten, and darken in linear
///   light, and screen, overlay, and soft light with the HDR-safe formulas,
///   or in E when `in_e` (the per-project "run in E" option).
///
/// [`crate::linear`] holds the f64 references and their f32 twins.
pub const BLEND: &str = r#"
fn pm_blend_legacy(b: vec3<f32>, s: vec3<f32>, m: i32) -> vec3<f32> {
    if (m == 1) { return b + s; }
    if (m == 2) { return 1.0 - (1.0 - b) * (1.0 - s); }
    if (m == 3) { return b * s; }
    if (m == 4) { return mix(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), step(vec3<f32>(0.5), b)); }
    if (m == 5) {
        return mix(2.0 * b * s + b * b * (1.0 - 2.0 * s), sqrt(b) * (2.0 * s - 1.0) + 2.0 * b * (1.0 - s), step(vec3<f32>(0.5), s));
    }
    if (m == 6) { return abs(b - s); }
    if (m == 7) { return max(b, s); }
    if (m == 8) { return min(b, s); }
    return s;
}

fn pm_blend_hdr_safe(b: vec3<f32>, s: vec3<f32>, m: i32) -> vec3<f32> {
    if (m == 2) { return b + s - b * s; }
    if (m == 4) {
        return select(1.0 - 2.0 * (1.0 - b) * (1.0 - s), 2.0 * b * s, clamp(b, vec3<f32>(0.0), vec3<f32>(1.0)) < vec3<f32>(0.5));
    }
    if (m == 5) {
        return select(
            sqrt(max(b, vec3<f32>(0.0))) * (2.0 * s - 1.0) + 2.0 * b * (1.0 - s),
            2.0 * b * s + b * b * (1.0 - 2.0 * s),
            clamp(s, vec3<f32>(0.0), vec3<f32>(1.0)) < vec3<f32>(0.5));
    }
    return pm_blend_legacy(b, s, m);
}

fn pm_blend_working(b: vec3<f32>, s: vec3<f32>, m: i32, linear: bool, in_e: bool) -> vec3<f32> {
    if (!linear) { return pm_blend_legacy(b, s, m); }
    if (m == 2 || m == 4 || m == 5) {
        if (in_e) { return pm_e_decode3(pm_blend_legacy(pm_e_encode3(b), pm_e_encode3(s), m)); }
        return pm_blend_hdr_safe(b, s, m);
    }
    return pm_blend_legacy(b, s, m);
}
"#;

#[cfg(test)]
mod tests {
    #[test]
    fn floor_constant_matches_the_contract() {
        assert_eq!(
            0.000_244_140_625_f64,
            pm_protocol::color::UNPREMULTIPLY_ALPHA_FLOOR
        );
        assert!(super::COLOR.contains("fn pm_e_encode("));
    }
}
