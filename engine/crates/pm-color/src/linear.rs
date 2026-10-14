//! Linear-light helpers and blend formulas (section 9, "Blend modes and
//! mattes"; section 8, "Working encoding and precision").
//!
//! With linear blending on, sources are read with E⁻¹, intermediates are
//! premultiplied linear light, and outputs encode with E. Normal, add,
//! multiply, difference, lighten, and darken run in the active encoding;
//! screen, overlay, and soft light use HDR-safe formulas (screen
//! `a + b − ab` unclamped; overlay and soft light on values clamped to 0..1
//! for their switch condition only), or run in E with the per-project
//! option. These are the f64 references the Phase 3a blend shaders are
//! tested against; [`legacy_blend`] is today's formula (the compositor's
//! `FRAG_COMPOSITE`) that Standard keeps.

use crate::transfer::{display_decode, display_encode};
use pm_protocol::color::{BlendDomain, BlendMode, WorkingEncoding};

/// A premultiplied E-encoded pixel into premultiplied linear light:
/// unpremultiply (guarded), E⁻¹, re-premultiply. Also the rule for the
/// resolved 3D output with linear blending on (section 9, "3D layers").
pub fn premultiplied_e_to_linear(rgba: [f64; 4]) -> [f64; 4] {
    crate::alpha::transform_premultiplied(rgba, false, display_decode)
}

/// The inverse of [`premultiplied_e_to_linear`].
pub fn premultiplied_linear_to_e(rgba: [f64; 4]) -> [f64; 4] {
    crate::alpha::transform_premultiplied(rgba, false, display_encode)
}

/// A hex or E-encoded straight color into the working encoding.
pub fn color_to_working(rgb: [f64; 3], working: WorkingEncoding) -> [f64; 3] {
    match working {
        WorkingEncoding::Standard => rgb,
        WorkingEncoding::LinearLight => rgb.map(display_decode),
    }
}

/// Today's separable blend formulas on straight colors in the active
/// encoding (`FRAG_COMPOSITE`'s `blend(b, s, m)`): base `b`, source `s`.
pub fn legacy_blend(mode: BlendMode, b: f64, s: f64) -> f64 {
    let step = |edge: f64, x: f64| if x < edge { 0.0 } else { 1.0 };
    let mix = |x: f64, y: f64, a: f64| x * (1.0 - a) + y * a;
    match mode {
        BlendMode::Normal => s,
        BlendMode::Add => b + s,
        BlendMode::Screen => 1.0 - (1.0 - b) * (1.0 - s),
        BlendMode::Multiply => b * s,
        BlendMode::Overlay => mix(2.0 * b * s, 1.0 - 2.0 * (1.0 - b) * (1.0 - s), step(0.5, b)),
        BlendMode::SoftLight => mix(
            2.0 * b * s + b * b * (1.0 - 2.0 * s),
            libm::sqrt(b) * (2.0 * s - 1.0) + 2.0 * b * (1.0 - s),
            step(0.5, s),
        ),
        BlendMode::Difference => (b - s).abs(),
        BlendMode::Lighten => b.max(s),
        BlendMode::Darken => b.min(s),
    }
}

/// HDR-safe formulas for screen, overlay, and soft light in linear light:
/// screen unclamped; the switch conditions of overlay and soft light read
/// values clamped to 0..1, the formulas themselves do not clamp.
pub fn hdr_safe_blend(mode: BlendMode, b: f64, s: f64) -> f64 {
    let c = |x: f64| x.clamp(0.0, 1.0);
    match mode {
        BlendMode::Screen => b + s - b * s,
        BlendMode::Overlay => {
            if c(b) < 0.5 {
                2.0 * b * s
            } else {
                1.0 - 2.0 * (1.0 - b) * (1.0 - s)
            }
        }
        BlendMode::SoftLight => {
            if c(s) < 0.5 {
                2.0 * b * s + b * b * (1.0 - 2.0 * s)
            } else {
                libm::sqrt(b.max(0.0)) * (2.0 * s - 1.0) + 2.0 * b * (1.0 - s)
            }
        }
        other => legacy_blend(other, b, s),
    }
}

/// Where a mode runs for a project: `true` when the blend is evaluated on
/// E-encoded values (Standard, or the per-project option for screen,
/// overlay, and soft light), `false` in linear light with the HDR-safe
/// formulas.
pub fn blends_in_e(mode: BlendMode, working: WorkingEncoding, domain: BlendDomain) -> bool {
    match working {
        WorkingEncoding::Standard => true,
        WorkingEncoding::LinearLight => {
            mode.uses_blend_domain() && domain == BlendDomain::DisplayEncoded
        }
    }
}

/// One channel of a blend in a project's color setup: base and source are
/// straight values in the working encoding; the result is in the working
/// encoding.
pub fn blend_channel(
    mode: BlendMode,
    working: WorkingEncoding,
    domain: BlendDomain,
    b: f64,
    s: f64,
) -> f64 {
    match working {
        WorkingEncoding::Standard => legacy_blend(mode, b, s),
        WorkingEncoding::LinearLight if blends_in_e(mode, working, domain) => {
            display_decode(legacy_blend(mode, display_encode(b), display_encode(s)))
        }
        WorkingEncoding::LinearLight if mode.uses_blend_domain() => hdr_safe_blend(mode, b, s),
        WorkingEncoding::LinearLight => legacy_blend(mode, b, s),
    }
}

/// Today's composite of a premultiplied source over a premultiplied
/// destination with a blend mode and layer alpha (`FRAG_COMPOSITE`):
/// unpremultiply both (guarded at α > 0.0001), blend, mix by source alpha,
/// and re-premultiply by the union alpha.
pub fn legacy_composite(mode: BlendMode, src: [f64; 4], dst: [f64; 4], alpha: f64) -> [f64; 4] {
    let s = src.map(|v| v * alpha);
    let un = |p: [f64; 4]| -> [f64; 3] {
        if p[3] > 0.0001 {
            [p[0] / p[3], p[1] / p[3], p[2] / p[3]]
        } else {
            [0.0; 3]
        }
    };
    let sc = un(s);
    let dc = un(dst);
    let a = s[3] + dst[3] * (1.0 - s[3]);
    let mut out = [0.0; 4];
    for i in 0..3 {
        let bc = legacy_blend(mode, dc[i], sc[i]);
        let res = dc[i] * (1.0 - s[3]) + bc * s[3];
        out[i] = res * a;
    }
    out[3] = a;
    out
}

/// Premultiplied source-over (the fixed-function `ONE, ONE_MINUS_SRC_ALPHA`
/// blend the normal-mode fast path uses).
pub fn over(src: [f64; 4], dst: [f64; 4]) -> [f64; 4] {
    let k = 1.0 - src[3];
    [
        src[0] + dst[0] * k,
        src[1] + dst[1] * k,
        src[2] + dst[2] * k,
        src[3] + dst[3] * k,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_blending_bar_value() {
        // Section 2: 50% white over black in linear light is 188 at 8 bits.
        let white = premultiplied_e_to_linear([1.0, 1.0, 1.0, 1.0]);
        let src = white.map(|v| v * 0.5);
        let out = over(src, [0.0, 0.0, 0.0, 1.0]);
        let e = premultiplied_linear_to_e(out);
        assert_eq!(crate::transfer::unit_to_code(e[0], 8), 188);
    }

    #[test]
    fn normal_composite_over_opaque_matches_over() {
        let src = [0.3, 0.1, 0.2, 0.5];
        let dst = [0.2, 0.4, 0.6, 1.0];
        let a = legacy_composite(BlendMode::Normal, src, dst, 1.0);
        let b = over(src, dst);
        for i in 0..4 {
            assert!((a[i] - b[i]).abs() < 1e-12);
        }
    }

    #[test]
    fn hdr_safe_screen_keeps_superwhites() {
        assert_eq!(hdr_safe_blend(BlendMode::Screen, 2.0, 0.5), 2.0 + 0.5 - 1.0);
        assert!(legacy_blend(BlendMode::Screen, 2.0, 0.5) < 2.0);
    }

    #[test]
    fn blend_domain_selection() {
        use BlendDomain::*;
        use WorkingEncoding::*;
        assert!(blends_in_e(BlendMode::Screen, Standard, HdrSafe));
        assert!(!blends_in_e(BlendMode::Screen, LinearLight, HdrSafe));
        assert!(blends_in_e(BlendMode::Screen, LinearLight, DisplayEncoded));
        assert!(!blends_in_e(
            BlendMode::Multiply,
            LinearLight,
            DisplayEncoded
        ));
        let v = blend_channel(BlendMode::Overlay, LinearLight, DisplayEncoded, 0.2, 0.4);
        let e = display_decode(legacy_blend(
            BlendMode::Overlay,
            display_encode(0.2),
            display_encode(0.4),
        ));
        assert_eq!(v, e);
    }
}
