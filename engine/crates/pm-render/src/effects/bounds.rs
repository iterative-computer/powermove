//! Effect passes scissored to the pixels they can change.
//!
//! A layer with effects renders each pass into a target covering the whole
//! pass rectangle (the whole frame in a final frame), and every pass shaded
//! every pixel of it, although a small layer's input is transparent almost
//! everywhere. Targets stay full size here (a smaller target is not the same
//! program output: interpolants, `u_texel` and grids follow its size); a
//! pass only gets a scissor rectangle, and its attachment is still cleared
//! whole (`LoadOp::Clear(TRANSPARENT)`).
//!
//! That is exact when the unscissored program writes exactly `(0, 0, 0, 0)`
//! (positive zeros, the clear's bits) at every pixel outside the scissor:
//! - the step is *zero-preserving*: a pixel whose every input texel in its
//!   reach is `+0` comes out `+0`, whatever its position, for the parameter
//!   values given ([`reach`] checks the source and the parameters);
//! - the scissor holds every pixel within the step's reach of a texel that
//!   can be non-zero: the chain's input content bounds grown by every
//!   bounded step's reach so far ([`reach`] is the padding the region
//!   machinery already proves sufficient, `plan::region_padding`, plus a
//!   margin). Samplers clamp to the edge, which only moves a tap closer.
//!
//! Inside the scissor the same program runs on the same inputs (outside it
//! both runs hold zeros, by induction over the passes), so every pixel is
//! bit-identical. Only reviewed built-in sources qualify: each is pinned by
//! its SHA-256, so an extension's fork of `blur` or a changed catalog shader
//! is never scissored until it is reviewed again.

use super::plan::{Binding, EffectStep};
use pm_protocol::definitions::{DefinitionOrigin, DefinitionSnapshot};
use sha2::Digest;

/// How a reviewed source preserves zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// Legacy blur (`blur` v1 / builtins.json): passthrough below r = .25,
    /// else a normalized sum of `texture × w` with `w > 0`.
    LegacyBlur,
    /// Main blur v2: passthrough below σ = .01; else, without repeated
    /// edges, a normalized sum of `samplePixel × w`, zero outside the
    /// source. Repeated edges clamp taps to the layer's box: not bounded.
    Blur2,
    /// Glow v1: pass 0 `c × smoothstep(…) ≥ 0`, passes 1–2 a blur, pass 2
    /// `orig + blur × intensity`.
    Glow1,
    /// Color v1 and Levels v1: `c.a < .0005` keeps `c`.
    KeepsClear,
    /// Levels v2: zero outside the layer; inside, a zero pixel maps through
    /// `level` (checked on the parameters).
    Levels2,
}

/// The reviewed sources: catalog id, revision, SHA-256 of the source.
const REVIEWED: &[(&str, u64, &str, Kind)] = &[
    (
        "blur",
        1,
        "02fab3bed9d687ad08188ba676c9a584e29031eb1675ad2f9fa38a1a960e3ead",
        Kind::LegacyBlur,
    ),
    (
        "blur",
        2,
        "95dfc33745c07710df5d197c7915393f733f9401c2b6c6737e3205e0de034f87",
        Kind::Blur2,
    ),
    (
        "glow",
        1,
        "e5e3edb2f67ac53748458154f31c773756cc1300d61fae10602412a9495b6db3",
        Kind::Glow1,
    ),
    (
        "color",
        1,
        "e4b9ac14bd3122f99ae30f3e0d84ac344ed3a9958d1d0a1f98a31c0e77a18f69",
        Kind::KeepsClear,
    ),
    (
        "levels",
        1,
        "1e8c48509deba8fb4c73bb37d524227e86f9babc8b45ab33f161dbf6cb1bfc63",
        Kind::KeepsClear,
    ),
    (
        "levels",
        2,
        "29cef8b783bb856e0a199a2a3f20f74c1c3f0b5fba3b7b32f8b7f8e11fb2b3c0",
        Kind::Levels2,
    ),
];

fn sha256_hex(text: &str) -> String {
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The reviewed catalog sources whose pins still match, with their kinds.
/// (Built-in, saved and current registrations of these effects carry these
/// exact sources: `versions::builtin_snapshot`, and builtins.json's legacy
/// programs equal the catalog's revision 1.)
fn reviewed() -> &'static [(&'static str, Kind)] {
    static LIST: std::sync::OnceLock<Vec<(&'static str, Kind)>> = std::sync::OnceLock::new();
    LIST.get_or_init(|| {
        let shaders = &pm_fx_defs::catalog::main_effects().shaders;
        REVIEWED
            .iter()
            .filter_map(|(id, version, pin, kind)| {
                let shader = shaders
                    .iter()
                    .find(|s| s.id == *id && s.version == *version)?;
                (sha256_hex(&shader.source) == *pin).then_some((shader.source.as_str(), *kind))
            })
            .collect()
    })
}

fn kind_of(definition: &DefinitionSnapshot) -> Option<Kind> {
    if definition.origin != DefinitionOrigin::BuiltIn {
        return None;
    }
    let source = definition.glsl.as_ref()?.source.as_str();
    reviewed()
        .iter()
        .find(|(reviewed, _)| *reviewed == source)
        .map(|(_, kind)| *kind)
}

fn value(step: &EffectStep, key: &str) -> Option<f64> {
    step.params
        .iter()
        .find(|p| p.key == key)
        .and_then(|p| match p.value {
            Binding::Float(v) => Some(v),
            _ => None,
        })
}

/// Whether a zero pixel of Levels v2 comes out `+0`: with both clips on,
/// every `level` output lies between its finite, non-negative out values,
/// and alpha maps 0 to `mix(0, outWhite, 0) = +0` (a span below 1e-7 would
/// turn it into a step: refused).
fn levels2_keeps_zero(step: &EffectStep) -> bool {
    let v = |key: &str| value(step, key);
    let on = |key: &str| v(key).is_some_and(|x| x > 0.5);
    let non_negative = |key: &str| v(key).is_some_and(|x| x.is_finite() && x >= 0.);
    on("clipBlack")
        && on("clipWhite")
        && ["", "red", "green", "blue"].iter().all(|channel| {
            let key = |name: &str| {
                if channel.is_empty() {
                    let mut k = name.to_owned();
                    k[..1].make_ascii_lowercase();
                    k
                } else {
                    format!("{channel}{name}")
                }
            };
            non_negative(&key("OutBlack")) && non_negative(&key("OutWhite"))
        })
        && v("alphaOutBlack") == Some(0.)
        && non_negative("alphaOutWhite")
        && non_negative("alphaInBlack")
        && v("alphaInWhite")
            .zip(v("alphaInBlack"))
            .is_some_and(|(white, black)| white - black >= 1e-6)
}

/// The scissor growth of a zero-preserving step (output pixels on every
/// side), or `None` when the step must shade its whole target (and every
/// step after it, the chain's bounds being lost).
pub(crate) fn reach(step: &EffectStep, definition: &DefinitionSnapshot) -> Option<u32> {
    let kind = kind_of(definition)?;
    // Inputs other than the layer itself, a different-size grid target,
    // or output-raster addressing: not covered by the argument above.
    if step.backdrop
        || step.temporal
        || step.raster_space
        || !step.viewport_safe
        || !step.references.is_empty()
        || !step.assets.is_empty()
        || step.spatial.is_some_and(|s| s.grid.is_some())
    {
        return None;
    }
    // A non-finite value can turn a zero into NaN (`0 × ∞`).
    if !step.params.iter().all(|p| match &p.value {
        Binding::Float(v) => v.is_finite(),
        _ => true,
    }) {
        return None;
    }
    let keeps_zero = match kind {
        Kind::LegacyBlur | Kind::Glow1 | Kind::KeepsClear => true,
        Kind::Blur2 => value(step, "repeatEdgePixels").is_some_and(|v| v <= 0.5),
        Kind::Levels2 => levels2_keeps_zero(step),
    };
    if !keeps_zero {
        return None;
    }
    let crate::composite::effects::Padding::Pixels(padding) =
        super::plan::region_padding(std::slice::from_ref(step))
    else {
        return None;
    };
    // A reduced-resolution tap reads a mip texel of 2^lod pixels and its
    // bilinear neighbour: margin for both.
    let lod = step.spatial.map_or(0, |s| s.lod.min(16));
    let margin = 2 + if lod > 0 { 4u32 << lod } else { 0 };
    Some(padding.saturating_add(margin))
}

/// `rect` (x, y, width, height in a target's texels) grown by `by` on every
/// side within a `size` target.
pub(crate) fn grow(rect: (u32, u32, u32, u32), by: u32, size: (u32, u32)) -> (u32, u32, u32, u32) {
    let (x, y, w, h) = rect;
    let x0 = x.saturating_sub(by).min(size.0);
    let y0 = y.saturating_sub(by).min(size.1);
    let x1 = x.saturating_add(w).saturating_add(by).min(size.0);
    let y1 = y.saturating_add(h).saturating_add(by).min(size.1);
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

/// The part of two scissor rectangles both hold (empty: zero size).
pub(crate) fn intersect(
    a: (u32, u32, u32, u32),
    b: (u32, u32, u32, u32),
) -> (u32, u32, u32, u32) {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every pin matches the catalog this build carries: a changed shader
    /// must be reviewed for zero preservation before it is scissored again.
    #[test]
    fn every_reviewed_source_is_still_the_catalogs() {
        assert_eq!(reviewed().len(), REVIEWED.len(), "a reviewed source changed");
    }

    /// builtins.json's legacy programs are the catalog's revision 1.
    #[test]
    fn legacy_builtins_are_reviewed_sources() {
        let set = crate::effects::builtin_definitions();
        for id in ["blur", "glow", "color", "levels"] {
            let def = set
                .definitions
                .iter()
                .find(|d| d.id.as_str() == id)
                .unwrap();
            assert!(kind_of(def).is_some(), "{id}");
        }
        // Others are never bounded.
        let shadow = set
            .definitions
            .iter()
            .find(|d| d.id.as_str() == "shadow")
            .unwrap();
        assert_eq!(kind_of(shadow), None);
    }

    #[test]
    fn growth_and_intersection_stay_in_the_target() {
        assert_eq!(grow((10, 5, 20, 10), 8, (100, 100)), (2, 0, 36, 23));
        assert_eq!(grow((90, 90, 10, 10), 50, (100, 100)), (40, 40, 60, 60));
        assert_eq!(grow((0, 0, 0, 0), 3, (2, 2)), (0, 0, 2, 2));
        assert_eq!(intersect((0, 0, 10, 10), (5, 5, 10, 10)), (5, 5, 5, 5));
        assert_eq!(intersect((0, 0, 4, 4), (6, 6, 2, 2)), (6, 6, 0, 0));
    }
}
