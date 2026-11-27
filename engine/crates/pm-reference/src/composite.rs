//! Compositing on premultiplied RGBA in f64, from W3C Compositing and
//! Blending Level 1 (§5 "Compositing", §9 "Advanced compositing
//! features", §10 "Blending") and Porter & Duff, "Compositing Digital
//! Images" (1984).
//!
//! Colors are premultiplied `[r, g, b, a]` in whatever encoding the gate
//! composites in (a project's working encoding); nothing here clamps,
//! quantizes or changes encoding.

/// Premultiplied RGBA.
pub type Rgba = [f64; 4];

/// The Porter-Duff operators (W3C §9.1): `co = cs·Fa + cb·Fb`,
/// `αo = αs·Fa + αb·Fb`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operator {
    Clear,
    Copy,
    Destination,
    SourceOver,
    DestinationOver,
    SourceIn,
    DestinationIn,
    SourceOut,
    DestinationOut,
    SourceAtop,
    DestinationAtop,
    Xor,
    /// `plus-lighter` without its clamp (W3C §9.1.12's `lighter`).
    Lighter,
}

/// Source `s` composited onto backdrop `b` with `op`.
pub fn porter_duff(op: Operator, s: Rgba, b: Rgba) -> Rgba {
    let (sa, ba) = (s[3], b[3]);
    let (fa, fb) = match op {
        Operator::Clear => (0.0, 0.0),
        Operator::Copy => (1.0, 0.0),
        Operator::Destination => (0.0, 1.0),
        Operator::SourceOver => (1.0, 1.0 - sa),
        Operator::DestinationOver => (1.0 - ba, 1.0),
        Operator::SourceIn => (ba, 0.0),
        Operator::DestinationIn => (0.0, sa),
        Operator::SourceOut => (1.0 - ba, 0.0),
        Operator::DestinationOut => (0.0, 1.0 - sa),
        Operator::SourceAtop => (ba, 1.0 - sa),
        Operator::DestinationAtop => (1.0 - ba, sa),
        Operator::Xor => (1.0 - ba, 1.0 - sa),
        Operator::Lighter => (1.0, 1.0),
    };
    std::array::from_fn(|i| s[i] * fa + b[i] * fb)
}

/// Separable blend functions `B(Cb, Cs)` on straight colors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blend {
    Normal,
    Multiply,
    Screen,
    Overlay,
    Darken,
    Lighten,
    ColorDodge,
    ColorBurn,
    HardLight,
    /// W3C §10.2.11 (the `D(Cb)` form).
    SoftLight,
    Difference,
    Exclusion,
    /// Adobe's "Linear Dodge (Add)": `Cb + Cs`, unclamped. Powermove's
    /// `add` mode.
    LinearDodge,
    /// Adobe Photoshop's soft light, which Powermove's `softlight` mode
    /// defines (an authored formula, kept exact):
    /// `Cs < ½: 2·Cb·Cs + Cb²·(1 − 2·Cs)`, else `√Cb·(2·Cs − 1) + 2·Cb·(1 − Cs)`.
    SoftLightPhotoshop,
}

/// `B(Cb, Cs)` for one channel.
pub fn blend_channel(mode: Blend, cb: f64, cs: f64) -> f64 {
    let hard_light = |cb: f64, cs: f64| {
        if cs <= 0.5 {
            cb * 2.0 * cs
        } else {
            let s = 2.0 * cs - 1.0;
            cb + s - cb * s
        }
    };
    match mode {
        Blend::Normal => cs,
        Blend::Multiply => cb * cs,
        Blend::Screen => cb + cs - cb * cs,
        Blend::Overlay => hard_light(cs, cb),
        Blend::Darken => cb.min(cs),
        Blend::Lighten => cb.max(cs),
        Blend::ColorDodge => {
            if cb == 0.0 {
                0.0
            } else if cs >= 1.0 {
                1.0
            } else {
                (cb / (1.0 - cs)).min(1.0)
            }
        }
        Blend::ColorBurn => {
            if cb >= 1.0 {
                1.0
            } else if cs <= 0.0 {
                0.0
            } else {
                1.0 - ((1.0 - cb) / cs).min(1.0)
            }
        }
        Blend::HardLight => hard_light(cb, cs),
        Blend::SoftLight => {
            if cs <= 0.5 {
                cb - (1.0 - 2.0 * cs) * cb * (1.0 - cb)
            } else {
                let d = if cb <= 0.25 {
                    ((16.0 * cb - 12.0) * cb + 4.0) * cb
                } else {
                    cb.sqrt()
                };
                cb + (2.0 * cs - 1.0) * (d - cb)
            }
        }
        Blend::Difference => (cb - cs).abs(),
        Blend::Exclusion => cb + cs - 2.0 * cb * cs,
        Blend::LinearDodge => cb + cs,
        Blend::SoftLightPhotoshop => {
            if cs < 0.5 {
                2.0 * cb * cs + cb * cb * (1.0 - 2.0 * cs)
            } else {
                cb.sqrt() * (2.0 * cs - 1.0) + 2.0 * cb * (1.0 - cs)
            }
        }
    }
}

/// Source-over with blending (W3C §10, premultiplied form):
/// `co = cs·(1 − αb) + cb·(1 − αs) + αs·αb·B(Cb, Cs)`,
/// `αo = αs + αb·(1 − αs)`, with straight colors `C = c / α` (0 where
/// α = 0, where the blended term vanishes anyway).
pub fn blend(mode: Blend, s: Rgba, b: Rgba) -> Rgba {
    let (sa, ba) = (s[3], b[3]);
    let straight = |c: f64, a: f64| if a > 0.0 { c / a } else { 0.0 };
    let mut out = [0.0; 4];
    for i in 0..3 {
        let mixed = blend_channel(mode, straight(b[i], ba), straight(s[i], sa));
        out[i] = s[i] * (1.0 - ba) + b[i] * (1.0 - sa) + sa * ba * mixed;
    }
    out[3] = sa + ba * (1.0 - sa);
    out
}

/// A premultiplied color scaled by a coverage or opacity factor.
pub fn scale(c: Rgba, k: f64) -> Rgba {
    c.map(|v| v * k)
}

/// Rec. ITU-R BT.709 luma of (encoded) RGB: the luma matte's weights.
pub fn luma709(rgb: [f64; 3]) -> f64 {
    0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_blend_is_source_over() {
        let s = [0.2, 0.1, 0.0, 0.4];
        let b = [0.3, 0.6, 0.5, 0.8];
        let x = blend(Blend::Normal, s, b);
        let y = porter_duff(Operator::SourceOver, s, b);
        for i in 0..4 {
            assert!((x[i] - y[i]).abs() < 1e-15);
        }
    }

    #[test]
    fn porter_duff_identities() {
        let s = [0.2, 0.1, 0.05, 0.4];
        let b = [0.3, 0.6, 0.5, 0.8];
        assert_eq!(porter_duff(Operator::Copy, s, b), s);
        assert_eq!(porter_duff(Operator::Destination, s, b), b);
        assert_eq!(porter_duff(Operator::Clear, s, b), [0.0; 4]);
        let xor = porter_duff(Operator::Xor, s, b);
        assert!((xor[3] - (0.4 * 0.2 + 0.8 * 0.6)).abs() < 1e-15);
    }

    #[test]
    fn blend_spec_values() {
        // Values the W3C §10.2 formulas give.
        assert_eq!(blend_channel(Blend::Overlay, 0.25, 0.5), 0.25);
        assert_eq!(blend_channel(Blend::HardLight, 0.5, 0.75), 0.75);
        assert!((blend_channel(Blend::SoftLight, 0.25, 1.0) - 0.5).abs() < 1e-15);
        assert_eq!(blend_channel(Blend::ColorDodge, 0.5, 0.5), 1.0);
        assert_eq!(blend_channel(Blend::ColorBurn, 0.5, 0.5), 0.0);
        assert_eq!(blend_channel(Blend::Exclusion, 0.5, 0.5), 0.5);
        // Photoshop soft light is continuous at Cs = ½ and fixes Cb = 0, 1.
        let below = blend_channel(Blend::SoftLightPhotoshop, 0.3, 0.5 - 1e-12);
        let above = blend_channel(Blend::SoftLightPhotoshop, 0.3, 0.5);
        assert!((below - above).abs() < 1e-9);
        assert_eq!(blend_channel(Blend::SoftLightPhotoshop, 1.0, 0.8), 1.0);
    }
}
