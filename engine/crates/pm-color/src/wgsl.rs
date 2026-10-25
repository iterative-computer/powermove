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
//! [`CSS_GAMUT`] adds the CSS Color 4 sRGB boundary mapping independently of
//! [`COLOR`], with `pm_css_*` names so it can be appended to any shader.

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

/// Self-contained CSS Color 4 §14.2.2 binary-search gamut mapping with local
/// MINDE. `pm_css_gamut_srgb` accepts and returns straight, encoded sRGB; the
/// caller chooses the SDR output boundary and handles alpha separately.
pub const CSS_GAMUT: &str = r#"
const PM_CSS_JND: f32 = 0.02;
const PM_CSS_EPSILON: f32 = 0.0001;

fn pm_css_e_decode(x: f32) -> f32 {
    let magnitude = abs(x);
    if (magnitude <= 0.04045) { return x / 12.92; }
    let linear = pow((magnitude + 0.055) / 1.055, 2.4);
    return select(linear, -linear, x < 0.0);
}

fn pm_css_e_encode(x: f32) -> f32 {
    let magnitude = abs(x);
    if (magnitude <= 0.0031308) { return 12.92 * x; }
    let encoded = 1.055 * pow(magnitude, 1.0 / 2.4) - 0.055;
    return select(encoded, -encoded, x < 0.0);
}

fn pm_css_srgb_to_oklab(rgb: vec3<f32>) -> vec3<f32> {
    let linear = vec3<f32>(
        pm_css_e_decode(rgb.x), pm_css_e_decode(rgb.y), pm_css_e_decode(rgb.z));
    let xyz = vec3<f32>(
        dot(vec3<f32>(506752.0 / 1228815.0, 87881.0 / 245763.0, 12673.0 / 70218.0), linear),
        dot(vec3<f32>(87098.0 / 409605.0, 175762.0 / 245763.0, 12673.0 / 175545.0), linear),
        dot(vec3<f32>(7918.0 / 409605.0, 87881.0 / 737289.0, 1001167.0 / 1053270.0), linear));
    let lms = vec3<f32>(
        dot(vec3<f32>(0.8190224380, 0.3619062601, -0.1288737815), xyz),
        dot(vec3<f32>(0.0329836539, 0.9292868616, 0.0361446664), xyz),
        dot(vec3<f32>(0.0481771894, 0.2642395318, 0.6335478285), xyz));
    let root = sign(lms) * pow(abs(lms), vec3<f32>(1.0 / 3.0));
    return vec3<f32>(
        dot(vec3<f32>(0.2104542683, 0.7936177747, -0.0040720430), root),
        dot(vec3<f32>(1.9779985324, -2.4285922420, 0.4505937096), root),
        dot(vec3<f32>(0.0259040425, 0.7827717125, -0.8086757549), root));
}

fn pm_css_oklab_to_srgb(lab: vec3<f32>) -> vec3<f32> {
    let lms = vec3<f32>(
        dot(vec3<f32>(1.0, 0.3963377774, 0.2158037573), lab),
        dot(vec3<f32>(1.0, -0.1055613458, -0.0638541728), lab),
        dot(vec3<f32>(1.0, -0.0894841775, -1.2914855480), lab));
    let cubed = lms * lms * lms;
    let xyz = vec3<f32>(
        dot(vec3<f32>(1.2268798758, -0.5578149945, 0.2813910457), cubed),
        dot(vec3<f32>(-0.0405757452, 1.1122868033, -0.0717110581), cubed),
        dot(vec3<f32>(-0.0763729367, -0.4214933324, 1.5869240198), cubed));
    let linear = vec3<f32>(
        dot(vec3<f32>(12831.0 / 3959.0, -329.0 / 214.0, -1974.0 / 3959.0), xyz),
        dot(vec3<f32>(-851781.0 / 878810.0, 1648619.0 / 878810.0, 36519.0 / 878810.0), xyz),
        dot(vec3<f32>(705.0 / 12673.0, -2585.0 / 12673.0, 705.0 / 667.0), xyz));
    return vec3<f32>(
        pm_css_e_encode(linear.x), pm_css_e_encode(linear.y), pm_css_e_encode(linear.z));
}

fn pm_css_in_gamut(rgb: vec3<f32>) -> bool {
    return all(rgb >= vec3<f32>(0.0)) && all(rgb <= vec3<f32>(1.0));
}

fn pm_css_delta_ok(clipped_rgb: vec3<f32>, lab: vec3<f32>) -> f32 {
    return length(pm_css_srgb_to_oklab(clipped_rgb) - lab);
}

fn pm_css_gamut_srgb(rgb: vec3<f32>) -> vec3<f32> {
    if (pm_css_in_gamut(rgb)) { return rgb; }
    let origin = pm_css_srgb_to_oklab(rgb);
    if (origin.x >= 1.0) { return vec3<f32>(1.0); }
    if (origin.x <= 0.0) { return vec3<f32>(0.0); }

    let chroma = length(origin.yz);
    var clipped = clamp(pm_css_oklab_to_srgb(origin), vec3<f32>(0.0), vec3<f32>(1.0));
    if (pm_css_delta_ok(clipped, origin) < PM_CSS_JND || chroma == 0.0) {
        return clipped;
    }
    let direction = origin.yz / chroma;
    var low = 0.0;
    var high = chroma;
    var low_in_gamut = true;
    for (var step = 0; step < 32 && high - low > PM_CSS_EPSILON; step = step + 1) {
        let current_chroma = (low + high) * 0.5;
        let current_lab = vec3<f32>(
            origin.x, current_chroma * direction.x, current_chroma * direction.y);
        let current = pm_css_oklab_to_srgb(current_lab);
        if (low_in_gamut && pm_css_in_gamut(current)) {
            low = current_chroma;
            continue;
        }
        clipped = clamp(current, vec3<f32>(0.0), vec3<f32>(1.0));
        let difference = pm_css_delta_ok(clipped, current_lab);
        if (difference < PM_CSS_JND) {
            if (PM_CSS_JND - difference < PM_CSS_EPSILON) { return clipped; }
            low_in_gamut = false;
            low = current_chroma;
        } else {
            high = current_chroma;
        }
    }
    return clipped;
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
        assert!(super::CSS_GAMUT.contains("fn pm_css_gamut_srgb("));
        assert!(super::CSS_GAMUT.contains("const PM_CSS_JND: f32 = 0.02"));
    }
}
