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
//!   section 9 rules with the 1/4096 guard (pixels below it are kept).

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
