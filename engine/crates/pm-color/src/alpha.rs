//! Premultiplication rules (section 9, "Alpha"; contract 4 invariant 5).
//!
//! Working values are premultiplied. Sources and outputs follow:
//!
//! - straight sources: input transform, then premultiply;
//! - premultiplied sources: used as they are when the input transform is the
//!   identity; otherwise unpremultiplied (guarded below α = 1/4096),
//!   transformed, and re-premultiplied;
//! - "premultiplied with color": the matte color is removed first;
//! - additive pixels (α = 0, RGB > 0) are never unpremultiplied;
//! - outputs: an identity output transform writes premultiplied working
//!   values directly to premultiplied outputs; otherwise unpremultiply,
//!   transform, and write straight or re-premultiplied in the encoded domain.

use pm_protocol::color::{AlphaMode, UNPREMULTIPLY_ALPHA_FLOOR, may_unpremultiply};

/// How a source's samples become premultiplied working values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SourceAlpha {
    /// No alpha: α = 1, color transformed.
    Opaque,
    /// Straight alpha: transform the color, then premultiply.
    TransformThenPremultiply,
    /// Premultiplied with an identity input transform: used as stored.
    AsIs,
    /// Premultiplied with a non-identity input: unpremultiply (guarded),
    /// transform, re-premultiply.
    Unpremultiply,
    /// Premultiplied against a matte color: remove the matte, then as
    /// [`SourceAlpha::Unpremultiply`] (or as is when the input is the identity).
    RemoveMatte { matte: [f64; 3], identity: bool },
}

/// The plan for a source's alpha mode and whether its input transform is
/// the identity (`ColorDescription::input_is_identity`).
pub fn source_plan(mode: AlphaMode, input_is_identity: bool) -> SourceAlpha {
    match mode {
        AlphaMode::Opaque => SourceAlpha::Opaque,
        AlphaMode::Straight => SourceAlpha::TransformThenPremultiply,
        AlphaMode::Premultiplied if input_is_identity => SourceAlpha::AsIs,
        AlphaMode::Premultiplied => SourceAlpha::Unpremultiply,
        AlphaMode::PremultipliedWithColor { matte } => SourceAlpha::RemoveMatte {
            matte,
            identity: input_is_identity,
        },
    }
}

/// The plan as the integer the source shader switches on (`pm-render`).
pub fn shader_code(plan: SourceAlpha) -> u32 {
    match plan {
        SourceAlpha::Opaque => 0,
        SourceAlpha::TransformThenPremultiply => 1,
        SourceAlpha::AsIs => 2,
        SourceAlpha::Unpremultiply => 3,
        SourceAlpha::RemoveMatte { .. } => 4,
    }
}

/// Premultiplies straight RGBA.
pub fn premultiply(rgba: [f64; 4]) -> [f64; 4] {
    [
        rgba[0] * rgba[3],
        rgba[1] * rgba[3],
        rgba[2] * rgba[3],
        rgba[3],
    ]
}

/// Unpremultiplies, guarded: below the floor (and for additive pixels with
/// α = 0) the color is returned unchanged and flagged, so a transform never
/// divides by a near-zero alpha.
pub fn unpremultiply(rgba: [f64; 4]) -> Unpremultiplied {
    if may_unpremultiply(rgba[3]) {
        let a = rgba[3];
        Unpremultiplied::Straight([rgba[0] / a, rgba[1] / a, rgba[2] / a, a])
    } else {
        Unpremultiplied::Kept(rgba)
    }
}

/// The result of [`unpremultiply`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unpremultiplied {
    /// Straight color and alpha.
    Straight([f64; 4]),
    /// α below [`UNPREMULTIPLY_ALPHA_FLOOR`]: the premultiplied value, to be
    /// passed through a color transform as is (additive pixels stay additive).
    Kept([f64; 4]),
}

/// Applies a per-channel color transform to a premultiplied pixel following
/// the rules above: identity inputs are untouched; otherwise unpremultiply,
/// transform, re-premultiply, keeping pixels below the floor as they are.
pub fn transform_premultiplied(
    rgba: [f64; 4],
    identity: bool,
    transform: impl Fn(f64) -> f64,
) -> [f64; 4] {
    if identity {
        return rgba;
    }
    match unpremultiply(rgba) {
        Unpremultiplied::Straight(s) => {
            premultiply([transform(s[0]), transform(s[1]), transform(s[2]), s[3]])
        }
        Unpremultiplied::Kept(k) => k,
    }
}

/// A source sample (stored values, then alpha) to premultiplied working
/// values through `transform` (the input transform: transfer and primaries).
pub fn source_to_working(
    stored: [f64; 4],
    plan: SourceAlpha,
    transform: impl Fn(f64) -> f64,
) -> [f64; 4] {
    match plan {
        SourceAlpha::Opaque => [
            transform(stored[0]),
            transform(stored[1]),
            transform(stored[2]),
            1.0,
        ],
        SourceAlpha::TransformThenPremultiply => premultiply([
            transform(stored[0]),
            transform(stored[1]),
            transform(stored[2]),
            stored[3],
        ]),
        SourceAlpha::AsIs => stored,
        SourceAlpha::Unpremultiply => transform_premultiplied(stored, false, transform),
        SourceAlpha::RemoveMatte { matte, identity } => {
            let a = stored[3];
            let premultiplied = [
                stored[0] - (1.0 - a) * matte[0],
                stored[1] - (1.0 - a) * matte[1],
                stored[2] - (1.0 - a) * matte[2],
                a,
            ];
            transform_premultiplied(premultiplied, identity, transform)
        }
    }
}

/// An output's alpha convention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputAlpha {
    /// No alpha channel: premultiplied values, i.e. composited over black.
    Opaque,
    Straight,
    Premultiplied,
}

/// Premultiplied working RGBA to output RGBA (before quantization) through
/// `transform` (working encoding to the output encoding; `identity` when
/// it is the identity, as in Standard to an E-encoded output).
pub fn working_to_output(
    working: [f64; 4],
    alpha: OutputAlpha,
    identity: bool,
    transform: impl Fn(f64) -> f64,
) -> [f64; 4] {
    match alpha {
        OutputAlpha::Opaque => {
            if identity {
                [working[0], working[1], working[2], 1.0]
            } else {
                // Composited over black in the working space, then encoded.
                [
                    transform(working[0]),
                    transform(working[1]),
                    transform(working[2]),
                    1.0,
                ]
            }
        }
        OutputAlpha::Premultiplied if identity => working,
        OutputAlpha::Premultiplied => transform_premultiplied(working, false, transform),
        OutputAlpha::Straight => match unpremultiply(working) {
            Unpremultiplied::Straight(s) => {
                [transform(s[0]), transform(s[1]), transform(s[2]), s[3]]
            }
            // Nothing visible: straight color under α ≈ 0 is zero.
            Unpremultiplied::Kept(k) => [0.0, 0.0, 0.0, k[3]],
        },
    }
}

/// The unpremultiply floor (1/4096, safe at half float).
pub const FLOOR: f64 = UNPREMULTIPLY_ALPHA_FLOOR;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transfer::{display_decode, display_encode};

    #[test]
    fn plans_follow_section_9() {
        assert_eq!(
            source_plan(AlphaMode::Straight, true),
            SourceAlpha::TransformThenPremultiply
        );
        assert_eq!(
            source_plan(AlphaMode::Premultiplied, true),
            SourceAlpha::AsIs
        );
        assert_eq!(
            source_plan(AlphaMode::Premultiplied, false),
            SourceAlpha::Unpremultiply
        );
        assert_eq!(source_plan(AlphaMode::Opaque, false), SourceAlpha::Opaque);
    }

    #[test]
    fn identity_premultiplied_sources_are_untouched() {
        let px = [0.2, 0.1, 0.05, 0.25];
        assert_eq!(source_to_working(px, SourceAlpha::AsIs, display_decode), px);
    }

    #[test]
    fn additive_pixels_are_never_unpremultiplied() {
        let additive = [0.3, 0.2, 0.1, 0.0];
        let out = source_to_working(additive, SourceAlpha::Unpremultiply, |v| v * 2.0);
        assert_eq!(out, additive);
        let tiny = [0.0001, 0.0, 0.0, 1.0 / 8192.0];
        assert_eq!(transform_premultiplied(tiny, false, |v| v * 2.0), tiny);
        assert!(matches!(
            unpremultiply([0.1, 0.1, 0.1, FLOOR]),
            Unpremultiplied::Straight(_)
        ));
    }

    #[test]
    fn non_identity_premultiplied_round_trips() {
        let straight = [0.6, 0.3, 0.9, 0.5];
        let stored = premultiply(straight);
        let working = source_to_working(stored, SourceAlpha::Unpremultiply, display_decode);
        let back = working_to_output(working, OutputAlpha::Straight, false, display_encode);
        for i in 0..4 {
            assert!((back[i] - straight[i]).abs() < 1e-12);
        }
    }

    #[test]
    fn matte_color_is_removed() {
        // Stored = a·F + (1 − a)·M with F = 0.8, M = white, a = 0.25.
        let (f, m, a) = (0.8, 1.0, 0.25);
        let stored = a * f + (1.0 - a) * m;
        let plan = source_plan(AlphaMode::PremultipliedWithColor { matte: [m; 3] }, true);
        let w = source_to_working([stored, stored, stored, a], plan, |v| v);
        assert!((w[0] - a * f).abs() < 1e-15);
    }

    #[test]
    fn opaque_outputs_composite_over_black() {
        let w = [0.25, 0.5, 0.0, 0.5];
        assert_eq!(
            working_to_output(w, OutputAlpha::Opaque, true, |v| v),
            [0.25, 0.5, 0.0, 1.0]
        );
        assert_eq!(
            working_to_output(w, OutputAlpha::Premultiplied, true, |v| v),
            w
        );
        assert_eq!(
            working_to_output([0.1, 0.0, 0.0, 0.0], OutputAlpha::Straight, true, |v| v),
            [0.0, 0.0, 0.0, 0.0]
        );
    }
}
