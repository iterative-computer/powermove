//! Effect passes scissored to the pixels they can change.
//!
//! A layer with effects renders each pass into a target covering the whole
//! pass rectangle (the whole frame in a final frame), and every pass shaded
//! every pixel of it, although a small layer's input is transparent almost
//! everywhere. Targets stay full size here (a smaller target is not the same
//! program output: interpolants, `u_texel` and grids follow its size); a
//! pass only gets scissor rectangles, and its attachment is still cleared
//! whole (`LoadOp::Clear(TRANSPARENT)`).
//!
//! That is exact when omitted texels are equivalent to clear for later
//! passes, and the unscissored final pass writes exact clear outside its
//! scissor:
//! - the step is *zero-preserving*: a pixel whose every input texel in its
//!   reach is `+0` comes out `+0`, whatever its position, for the parameter
//!   values given ([`reach`] checks the source and the parameters);
//! - the scissor holds every pixel within the step's reach of a texel that
//!   can be non-zero: the chain's input content bounds grown by every
//!   bounded step's reach so far. For built-ins this is the region padding
//!   plus a sampling margin. Reviewed extensions have their own bounds.
//!
//! Inside the scissor the same program runs on the same inputs (outside it
//! omitted texels have the same sampled values, by induction over the
//! passes), so every output pixel is bit-identical. Reviewed multi-pass
//! extensions can also omit intermediate channels the next pass ignores.
//! The SHA-256 pin limits this optimization to the reviewed source, including
//! the compat wrapper for extensions.
//!
//! Zero preservation alone ([`preserves_zero`], [`chain_preserves_zero`])
//! also lets the plan leave out a boundary group whose members draw
//! nothing (`plan::draw_layer`): its target is all `+0`, so a chain that
//! maps an all-`+0` input to an all-`+0` output leaves it so, and its
//! normal composite changes nothing beneath it. That needs no locality.

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
    /// Legacy Drop Shadow (`shadow` v1): pass 0 shifts alpha, passes 1–2 blur
    /// alpha, pass 2 composites `orig + (color * a, a) * (1 - orig.a)`.
    Shadow1,
    /// Main Drop Shadow (`shadow` v2): pass 0 shifts alpha via `samplePixel`
    /// (zero outside source), passes 1–2 blur alpha via `samplePixel`, pass 2
    /// composites `orig + shaded * (1 - orig.a)` (or `shaded` when shadowOnly).
    Shadow2,
    /// A reviewed extension (`dot-matrix-resolve` 1.2.0: one pass, reading
    /// only its input, `u_tex`), pinned by its SHA-256. Reviewed properties,
    /// for finite parameters:
    /// - zero preserving: an output pixel whose every input texel within
    ///   its reach is `+0` is `+0`, whatever its position (no time,
    ///   position, or parameter term reaches the output without a non-zero
    ///   input texel, and there is no NaN path);
    /// - finite reach: a pixel reads input texels at most
    ///   `0.83 · max(pitch, 2)` program pixels away from itself on each
    ///   axis, in the coordinates of its own target (`v_st`, `u_res`), so
    ///   the bound needs no `viewport_safe` or `raster_space` refusal
    ///   ([`dot_matrix_reach`]).
    DotMatrixResolve,
    /// Liquid Glass iOS 27. Its final pass writes clear wherever the
    /// original input's alpha is below its coverage threshold. The first
    /// two passes also have zero RGB outside the input; their alpha is
    /// ignored by later passes, so clearing it has no effect.
    LiquidGlassIos27,
    /// Deep Glow. The first seven passes carry RGB in two packed tiles
    /// and use no intermediate alpha in the final composite.
    DeepGlow,
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
    (
        "shadow",
        1,
        "6e62e717d61823f806bb0d2728ce5ebbfcd513fd2566f0709248678b055b5ee9",
        Kind::Shadow1,
    ),
    (
        "shadow",
        2,
        "1ddde3c5afbb931867a44229a273e240087a697643f25a07a962909295d0cea1",
        Kind::Shadow2,
    ),
];

/// Reviewed extension sources: definition id, SHA-256 of the wrapped GLSL
/// its definition snapshot carries (`DefinitionSnapshot::glsl.source`: the
/// compat realm's `legacyEffect(item).frag`, `apps/native/js/compat/src/
/// definitions.ts`, preamble and uniform declarations included), and its
/// kind. Any other byte, a param added, renamed or retyped (the uniform
/// declarations are part of the source), `keepOrig` or `backdrop` (they add
/// declarations), or another wrapper, and the source is not reviewed.
const REVIEWED_EXTENSIONS: &[(&str, &str, Kind)] = &[
    (
        "dot-matrix-resolve",
        // A user profile's v1.2.0 (manifest `apiVersion` 1). Only the digest
        // is committed for each extension. Tests pin synthetic stand-ins.
        "df74cebf7460c5a69de557336ee57fa1329a83c0325d38d2f0b95063ae6ca8f6",
        Kind::DotMatrixResolve,
    ),
    (
        "liquid-glass-ios27.glass",
        "f1b8449db321dc4eda9c1595ca203ffd97328dd5e0d986210dc04eda26332886",
        Kind::LiquidGlassIos27,
    ),
    (
        "deep-glow.effect",
        "022c89a2256e3f6b1e2db4312f9a23f92a62da06e0fbd46f12d189aee8561a3e",
        Kind::DeepGlow,
    ),
];

fn sha256_hex(text: &str) -> String {
    sha2::Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The kind of a reviewed extension from the id and the SHA-256 hex digest
/// of its wrapped source. The only place a pin is compared; tests also
/// accept the digest of a synthetic stand-in source, so no private shader
/// is needed to exercise the pinned path.
fn digest_kind(id: &str, digest: &str) -> Option<Kind> {
    let (_, pin, kind) = REVIEWED_EXTENSIONS.iter().find(|(p, ..)| *p == id)?;
    let matches = digest == *pin;
    #[cfg(test)]
    let matches = matches || tests::is_test_pin(id, digest);
    matches.then_some(*kind)
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

/// A reviewed extension source's kind: the id must be a reviewed one and
/// the source must hash to its pin. Sources already hashed are remembered
/// by content (a length check and a compare, not a hash, per lookup).
fn extension_kind(id: &str, source: &str) -> Option<Kind> {
    static SEEN: std::sync::Mutex<Vec<(&'static str, String, Option<Kind>)>> =
        std::sync::Mutex::new(Vec::new());
    let (pinned, ..) = REVIEWED_EXTENSIONS.iter().find(|(p, ..)| *p == id)?;
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((_, _, known)) = seen
        .iter()
        .find(|(p, s, _)| *p == *pinned && s.as_str() == source)
    {
        return *known;
    }
    let known = digest_kind(id, &sha256_hex(source));
    if seen.len() >= 16 {
        seen.clear();
    }
    seen.push((*pinned, source.to_owned(), known));
    known
}

fn kind_of(definition: &DefinitionSnapshot) -> Option<Kind> {
    let source = definition.glsl.as_ref()?.source.as_str();
    if definition.origin != DefinitionOrigin::BuiltIn {
        // A fork of a built-in registers under the built-in's id, which is
        // never a reviewed extension's: not reviewed.
        return extension_kind(definition.id.as_str(), source);
    }
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

/// Whether every parameter is finite (a non-finite value can turn a zero
/// into NaN: `0 × ∞`).
fn finite(step: &EffectStep) -> bool {
    step.params.iter().all(|p| match &p.value {
        Binding::Float(v) => v.is_finite(),
        Binding::Color(rgb) => rgb.iter().all(|v| v.is_finite()),
        Binding::Stops(_) => true,
    })
}

/// Whether `step` maps an input whose every texel is `+0` to an output
/// whose every texel is `+0` (or runs no pass), whatever the position, for
/// the parameter values given: a reviewed source ([`kind_of`]), reading
/// its input on a same-size target with finite parameters. A reviewed shader
/// may read the backdrop if its output is gated by the input's coverage.
pub(crate) fn preserves_zero(step: &EffectStep, definition: &DefinitionSnapshot) -> bool {
    let Some(kind) = kind_of(definition) else {
        return false;
    };
    keeps_zero(kind, step)
}

fn keeps_zero(kind: Kind, step: &EffectStep) -> bool {
    if (step.backdrop && kind != Kind::LiquidGlassIos27)
        || step.temporal
        || !step.references.is_empty()
        || !step.assets.is_empty()
        || step.spatial.is_some_and(|s| s.grid.is_some())
        || !finite(step)
    {
        return false;
    }
    match kind {
        Kind::LegacyBlur | Kind::Glow1 | Kind::KeepsClear | Kind::Shadow1 | Kind::Shadow2 => true,
        Kind::Blur2 => value(step, "repeatEdgePixels").is_some_and(|v| v <= 0.5),
        Kind::Levels2 => levels2_keeps_zero(step),
        // Reviewed as one pass of its own input (the pin already excludes
        // `u_orig` and `u_backdrop`).
        Kind::DotMatrixResolve => step.passes == 1 && !step.keep_orig,
        Kind::LiquidGlassIos27 => {
            step.passes == 3
                && step.keep_orig
                && step.backdrop
                && step.spatial.is_none()
                && step.params.iter().all(|param| match param.value {
                    Binding::Float(v) => (v as f32).is_finite(),
                    Binding::Color(rgb) => rgb.iter().all(|&v| (v as f32).is_finite()),
                    Binding::Stops(_) => false,
                })
        }
        Kind::DeepGlow => {
            step.passes == 8
                && step.keep_orig
                && step.spatial.is_none()
                && step.source_domain.is_none()
                && deep_glow_params(step)
        }
    }
}

/// Keep the reviewed kernel and zero-preserving arithmetic within the
/// extension's parameter ranges, including positive power exponents and
/// a nonnegative dispersion. A source pin alone does not pin saved values.
fn deep_glow_params(step: &EffectStep) -> bool {
    const NUMBERS: &[(&str, f64, f64)] = &[
        ("radius", 0., 500.),
        ("exposure", -6., 8.),
        ("threshold", 0., 1.),
        ("smoothness", 0., 1.),
        ("falloff", 0., 1.),
        ("gamma", 1., 3.),
        ("tintMix", 0., 1.),
        ("saturation", 0., 2.),
        ("aspect", 0.25, 4.),
        ("angle", -180., 180.),
        ("dispersion", 0., 0.5),
        ("sourceOpacity", 0., 1.),
        ("glowOnly", 0., 1.),
        ("screen", 0., 1.),
        ("highQuality", 0., 1.),
    ];
    step.params.len() == NUMBERS.len() + 1
        && NUMBERS
            .iter()
            .all(|&(key, lo, hi)| value(step, key).is_some_and(|v| (lo..=hi).contains(&v)))
        && step.params.iter().any(|p| {
            p.key == "tint"
                && matches!(p.value, Binding::Color(rgb)
                if rgb.iter().all(|v| (0.0..=1.0).contains(v)))
        })
}

/// Whether a whole chain preserves zero ([`preserves_zero`] for every
/// step; an empty chain does). `definition` looks a step's definition up
/// by [`EffectStep::definition`]; one it does not find refuses the chain.
pub(crate) fn chain_preserves_zero<'d>(
    steps: &[EffectStep],
    definition: impl Fn(&str) -> Option<&'d DefinitionSnapshot>,
) -> bool {
    steps
        .iter()
        .all(|step| definition(&step.definition).is_some_and(|d| preserves_zero(step, d)))
}

/// The scissor growth of a [`Kind::DotMatrixResolve`] step in output
/// pixels on every side, `None` when a parameter it depends on is missing,
/// repeated, or not finite.
///
/// A pixel reads input texels at most `s = 0.83 · max(pitch, 2)` program
/// pixels away from its centre per axis. The program sees its own target
/// (`v_st` spans it, `u_res` is its size over [`effect_density`]), so a
/// program pixel is at most `d` output pixels on an axis whose density is
/// `d` (fewer where `u_res` is raised to 1). Bilinear taps read texels
/// whose centres lie within one pixel of the tap; clamping a tap to the
/// edge only moves it towards the pixel. So every texel read is within
/// `ceil(s · max(d)) + 1` pixels of the output pixel; one more absorbs f32
/// rounding of the coordinates.
///
/// [`effect_density`]: super::exec::effect_density
fn dot_matrix_reach(step: &EffectStep) -> Option<u32> {
    let mut pitches = step.params.iter().filter(|p| p.key == "pitch");
    let pitch = match (pitches.next(), pitches.next()) {
        (Some(p), None) => match p.value {
            Binding::Float(v) if v.is_finite() => v,
            _ => return None,
        },
        _ => return None,
    };
    let [dx, dy] = super::exec::effect_density(step);
    let spread = (0.83 * pitch.max(2.0) * dx.max(dy)).ceil();
    if !spread.is_finite() || spread < 0. {
        return None;
    }
    // Saturating: a reach past the target is the whole target (`grow`).
    Some((spread as u32).saturating_add(2))
}

/// The scissor growth of a zero-preserving step (output pixels on every
/// side), or `None` when the step must shade its whole target (and every
/// step after it, the chain's bounds being lost).
fn reach(kind: Kind, step: &EffectStep) -> Option<u32> {
    if !keeps_zero(kind, step) {
        return None;
    }
    match kind {
        Kind::LiquidGlassIos27 => {
            // The distance passes produce nonzero RGB only at covered
            // input texels. The vertical pass reads the horizontal pass
            // only there. Allow one texel for bilinear coverage, one for
            // the final Sobel reads, and one for coordinate rounding.
            return Some(3);
        }
        Kind::DotMatrixResolve => {
            // Its reach is in target coordinates at any viewport or
            // density. Reduced-resolution input (mips) is not reviewed.
            if step.spatial.is_some() {
                return None;
            }
            return dot_matrix_reach(step);
        }
        Kind::DeepGlow => return None,
        // V2 shifts and blurs on its owner's plane; its padding is measured
        // with the 2D `pixel_scale`, which a 2.5D/3D plane's perspective
        // exceeds (a solid pushed back towards the camera magnifies its
        // shadow's offset). Not bounded until the reach comes from the
        // plane itself.
        Kind::Shadow2 if step.owner.as_ref().is_some_and(|o| !o.planar) => return None,
        _ => {}
    }
    // Output-raster addressing is not covered for these kernels.
    if step.raster_space || !step.viewport_safe {
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

pub(super) type Rect = (u32, u32, u32, u32);

pub(super) struct Scissor {
    pub output: Rect,
    pub packed: Option<[[Rect; 2]; 7]>,
}

/// Bounds for the final output and any packed intermediate passes.
pub(super) fn scissors(
    step: &EffectStep,
    definition: &DefinitionSnapshot,
    input: Rect,
    size: (u32, u32),
) -> Option<Scissor> {
    let kind = kind_of(definition)?;
    if kind == Kind::DeepGlow {
        return deep_glow_scissors(step, input, size);
    }
    Some(Scissor {
        output: grow(input, reach(kind, step)?, size),
        packed: None,
    })
}

/// The packed tiles have half the resolution of the output on each axis.
/// Grow the envelope in output texels, then map it into both tiles. It
/// contains both the running bloom and the highlights reused at passes 3,
/// 5 and 6. Layout and texture allocations stay at full size.
fn deep_glow_scissors(step: &EffectStep, input: Rect, size: (u32, u32)) -> Option<Scissor> {
    let [dx, dy] = if step.raster_texel {
        [1., 1.]
    } else {
        super::exec::effect_density(step)
    };
    if dx.max(dy) > 2. || size.0 < 4 || size.1 < 4 || !keeps_zero(Kind::DeepGlow, step) {
        return None;
    }

    let radius = value(step, "radius")?;
    let aspect = value(step, "aspect")?;
    let (sin, cos) = value(step, "angle")?.to_radians().sin_cos();
    let (sin, cos) = (sin.abs(), cos.abs());
    let axis_x = radius * aspect.min(1.);
    let axis_y = radius * aspect.recip().min(1.);

    // Cumulative projected support. The largest chromatic multiplier
    // cancels the dispersion divisor. The fused pass can move along
    // both axes, so add their projected extents.
    let mut horizontal = 0.;
    let mut vertical = 0.;
    let extents = [
        (0., 0.),
        (0.65, 0.),
        (0., 0.65),
        (0.22, 0.),
        (0., 0.22),
        (0.085, 0.),
        (0.025, 0.085_f64.hypot(0.025)),
    ];
    let mut intermediate = [input; 7];
    let mut spread = [0.; 2];
    for (pass, (x, y)) in extents.into_iter().enumerate() {
        horizontal += x * axis_x;
        vertical += y * axis_y;
        spread = [
            (cos * horizontal + sin * vertical) * dx,
            (sin * horizontal + cos * vertical) * dy,
        ];
        // Highlight preparation reads up to half a program texel plus
        // bilinear support. Later clamps move taps by up to one packed
        // texel; bilinear filtering adds one more. Five output texels per
        // pass cover both, plus coordinate rounding and odd dimensions.
        let margin = 3 + pass as u32 * 5;
        intermediate[pass] = grow_xy(input, spread.map(|v| v.ceil() as u32 + margin), size);
    }
    // The final pass unpacks with another clamped bilinear read.
    let mut output = grow_xy(input, spread.map(|v| v.ceil() as u32 + 38), size);
    let seam = output.0 == 0 || output.0 + output.2 == size.0;
    let keep_full_width = |r: Rect| if seam { (0, r.1, size.0, r.3) } else { r };
    // If support reaches either horizontal edge, packed taps can cross
    // the seam and reach the opposite edge after unpacking.
    output = keep_full_width(output);
    let packed = intermediate.map(|r| packed_tiles(keep_full_width(r), size.0));
    Some(Scissor {
        output,
        packed: Some(packed),
    })
}

/// Each tile covers half the target on both axes. Round the edges outward
/// in texels, including when the seam falls on a pixel center. Merge
/// the two scissors when the bounds already cover the full width.
fn packed_tiles((x, y, w, h): Rect, width: u32) -> [Rect; 2] {
    let y0 = y / 2;
    let height = (y + h).div_ceil(2) - y0;
    if w == width {
        return [(0, y0, width, height), (0, 0, 0, 0)];
    }
    let tile = |offset: u32| {
        let x0 = (x + offset) / 2;
        let x1 = (x + w + offset).div_ceil(2);
        (x0, y0, x1 - x0, height)
    };
    [tile(0), tile(width)]
}

/// `rect` (x, y, width, height in a target's texels) grown by `by` on every
/// side within a `size` target.
pub(crate) fn grow(rect: Rect, by: u32, size: (u32, u32)) -> Rect {
    grow_xy(rect, [by; 2], size)
}

fn grow_xy(rect: Rect, [dx, dy]: [u32; 2], size: (u32, u32)) -> Rect {
    let (x, y, w, h) = rect;
    let x0 = x.saturating_sub(dx).min(size.0);
    let y0 = y.saturating_sub(dy).min(size.1);
    let x1 = x.saturating_add(w).saturating_add(dx).min(size.0);
    let y1 = y.saturating_add(h).saturating_add(dy).min(size.1);
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

/// The part of two scissor rectangles both hold (empty: zero size).
pub(crate) fn intersect(a: (u32, u32, u32, u32), b: (u32, u32, u32, u32)) -> (u32, u32, u32, u32) {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x0, y0, x1.saturating_sub(x0), y1.saturating_sub(y0))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every pin matches the catalog this build carries: a changed shader
    /// must be reviewed for zero preservation before it is scissored again.
    #[test]
    fn every_reviewed_source_is_still_the_catalogs() {
        assert_eq!(
            reviewed().len(),
            REVIEWED.len(),
            "a reviewed source changed"
        );
    }

    /// builtins.json's legacy programs are the catalog's revision 1.
    #[test]
    fn legacy_builtins_are_reviewed_sources() {
        let set = crate::effects::builtin_definitions();
        for id in ["blur", "glow", "color", "levels", "shadow"] {
            let def = set
                .definitions
                .iter()
                .find(|d| d.id.as_str() == id)
                .unwrap();
            assert!(kind_of(def).is_some(), "{id}");
        }
        // Others are never bounded.
        let vignette = set
            .definitions
            .iter()
            .find(|d| d.id.as_str() == "vignette")
            .unwrap();
        assert_eq!(kind_of(vignette), None);
    }

    /// A synthetic stand-in for the wrapped source of a reviewed extension
    /// (`legacyEffect(item).frag`): same shape (preamble, uniform
    /// declarations, a one-pass `main`) and zero preserving, but written for
    /// this test. The real source is private and never committed; only its
    /// digest is (`REVIEWED_EXTENSIONS`).
    pub(crate) const DOT_MATRIX: &str = concat!(
        "#version 300 es\n",
        "precision highp float;\n",
        "in vec2 v_uv; in vec2 v_st; in vec2 v_px;\n",
        "uniform sampler2D u_tex;\n",
        "uniform vec2 u_res;\n",
        "uniform vec2 u_effectScale;\n",
        "uniform float u_pitch;\n",
        "uniform float u_detail;\n",
        "uniform float u_dot;\n",
        "uniform float u_heat;\n",
        "uniform float u_resolve;\n",
        "uniform vec3 u_color;\n",
        "uniform int u_pass;\n",
        "out vec4 o;\n",
        "void main(){\n",
        "  vec4 crisp = texture(u_tex, v_st);\n",
        "  o = crisp * 0.5;\n",
        "}\n",
    );

    /// Synthetic three-pass backdrop effect with alpha in its intermediate
    /// buffers. The final pass uses only the intermediate red channel.
    const GLASS: &str = concat!(
        "#version 300 es\n",
        "precision highp float;\n",
        "in vec2 v_st; out vec4 o;\n",
        "uniform sampler2D u_tex, u_orig, u_backdrop;\n",
        "uniform int u_pass; uniform float u_amount;\n",
        "void main() {\n",
        "  float coverage = texture(u_orig, v_st).a;\n",
        "  if (u_pass == 0) o = vec4(coverage, 0.0, 0.0, 1.0);\n",
        "  else if (u_pass == 1) o = vec4(texture(u_tex, v_st).r, 0.0, 0.0, 1.0);\n",
        "  else if (coverage == 0.0) o = vec4(0.0);\n",
        "  else o = texture(u_backdrop, v_st) * texture(u_tex, v_st).r * u_amount;\n",
        "}\n",
    );

    /// Synthetic packed effect. Its intermediate alpha marks the tiles,
    /// and the final pass reads only their RGB.
    const DEEP_GLOW: &str = concat!(
        "#version 300 es\n",
        "precision highp float;\n",
        "in vec2 v_st; out vec4 o;\n",
        "uniform sampler2D u_tex, u_orig; uniform int u_pass;\n",
        "void main() {\n",
        "  if (u_pass == 7) o = vec4(texture(u_tex, v_st * 0.5).rgb, 0.0);\n",
        "  else if (v_st.y >= 0.5) o = vec4(0.0);\n",
        "  else o = vec4(texture(u_orig, fract(v_st * 2.0)).rgb, 1.0);\n",
        "}\n",
    );

    /// Pin synthetic shaders only in test builds.
    pub(super) fn is_test_pin(id: &str, digest: &str) -> bool {
        let source = match id {
            "dot-matrix-resolve" => DOT_MATRIX,
            "liquid-glass-ios27.glass" => GLASS,
            "deep-glow.effect" => DEEP_GLOW,
            _ => return false,
        };
        digest == sha256_hex(source)
    }

    fn builtin(id: &str) -> DefinitionSnapshot {
        crate::effects::builtin_definitions()
            .definitions
            .into_iter()
            .find(|d| d.id.as_str() == id)
            .unwrap()
    }

    /// `dot-matrix-resolve` as its definition snapshot holds it.
    pub(crate) fn dot_matrix_definition(source: &str) -> DefinitionSnapshot {
        use pm_protocol::definitions::ParamSpec;
        let number = |key: &str, default: f64, min: f64, max: f64, step: f64| ParamSpec::Number {
            key: key.into(),
            default,
            min,
            max,
            step: Some(step),
        };
        let mut def = builtin("blur");
        def.id = pm_protocol::common::DefinitionId::from("dot-matrix-resolve");
        def.origin = DefinitionOrigin::Local;
        def.glsl.as_mut().unwrap().source = source.to_owned();
        def.params = vec![
            number("pitch", 16., 2., 64., 0.1),
            number("detail", 0., 0., 5., 0.01),
            number("dot", 1.1, 0.3, 6., 0.05),
            number("heat", 100., 0., 100., 1.),
            number("resolve", 0., 0., 100., 0.1),
            ParamSpec::Color {
                key: "color".into(),
                default: "#FF6019".into(),
            },
        ];
        def.flags = Default::default();
        def
    }

    fn reach(step: &EffectStep, def: &DefinitionSnapshot) -> Option<u32> {
        super::reach(kind_of(def)?, step)
    }

    fn step(def: &DefinitionSnapshot) -> EffectStep {
        super::super::plan::effect_step(def, "fx", &[])
    }

    /// The committed digest pins the reviewed id and nothing else: a digest
    /// one bit off, another id, or the empty digest is not reviewed. (The
    /// source behind the real digest is private: only the digest is tested.)
    #[test]
    fn the_committed_digest_pins_only_its_id() {
        for &(id, real, kind) in REVIEWED_EXTENSIONS {
            assert_eq!(real.len(), 64);
            assert!(real.bytes().all(|b| b.is_ascii_hexdigit()));
            assert_eq!(digest_kind(id, real), Some(kind));
            assert_eq!(digest_kind(&format!("{id}-other"), real), None);
            assert_eq!(digest_kind("blur", real), None);
            assert_eq!(digest_kind(id, ""), None);
            // Every single-bit change of the digest.
            for at in 0..real.len() {
                let nibble = u8::from_str_radix(&real[at..at + 1], 16).unwrap();
                for bit in 0..4 {
                    let mut altered = real.to_owned();
                    altered.replace_range(at..at + 1, &format!("{:x}", nibble ^ (1 << bit)));
                    assert_eq!(digest_kind(id, &altered), None, "{id}: {at}/{bit}");
                }
            }
        }
    }

    /// Production hashes the runtime source and compares it to the pin: the
    /// synthetic stand-in matches its (test-only) pin exactly and nothing
    /// else does: one byte more, less, or changed, another id, or the
    /// built-in origin (whose sources are the catalog's).
    #[test]
    fn dot_matrix_resolve_is_pinned_to_its_exact_source() {
        let pinned = dot_matrix_definition(DOT_MATRIX);
        assert_eq!(kind_of(&pinned), Some(Kind::DotMatrixResolve));
        // Looked up again from the remembered source.
        assert_eq!(kind_of(&pinned), Some(Kind::DotMatrixResolve));
        for origin in [
            DefinitionOrigin::Store,
            DefinitionOrigin::Fork,
            DefinitionOrigin::Local,
        ] {
            let mut def = pinned.clone();
            def.origin = origin;
            assert_eq!(kind_of(&def), Some(Kind::DotMatrixResolve), "{origin:?}");
        }
        let mut builtin_origin = pinned.clone();
        builtin_origin.origin = DefinitionOrigin::BuiltIn;
        assert_eq!(kind_of(&builtin_origin), None);
        let mut renamed = pinned.clone();
        renamed.id = pm_protocol::common::DefinitionId::from("dot-matrix-resolve-2");
        assert_eq!(kind_of(&renamed), None);
        // `0.5` becomes `0.6`: one byte.
        let at = DOT_MATRIX.find("* 0.5").unwrap() + "* 0.".len();
        let mut changed = DOT_MATRIX.to_owned();
        changed.replace_range(at..at + 1, "6");
        assert_eq!(changed.len(), DOT_MATRIX.len());
        let edits = [
            changed,
            format!("{DOT_MATRIX}\n"),
            DOT_MATRIX[..DOT_MATRIX.len() - 1].to_owned(),
            DOT_MATRIX.replace('\n', "\r\n"),
            // The bare `frag` body (not what the snapshot carries).
            DOT_MATRIX[DOT_MATRIX.find("void main(){\n").unwrap() + 13..].to_owned(),
        ];
        for edit in edits {
            assert_ne!(sha256_hex(&edit), sha256_hex(DOT_MATRIX));
            let def = dot_matrix_definition(&edit);
            assert_eq!(kind_of(&def), None);
            assert!(!preserves_zero(&step(&def), &def));
        }
    }

    #[test]
    fn glass_requires_its_exact_source_and_three_pass_backdrop_contract() {
        let mut def = builtin("blur");
        def.id = "liquid-glass-ios27.glass".into();
        def.origin = DefinitionOrigin::Local;
        def.glsl.as_mut().unwrap().source = GLASS.to_owned();
        def.flags.passes = 3;
        def.flags.keep_orig = true;
        def.flags.backdrop = true;
        assert_eq!(kind_of(&def), Some(Kind::LiquidGlassIos27));

        let expected = step(&def);
        assert!(expected.raster_space);
        assert!(preserves_zero(&expected, &def));
        assert_eq!(reach(&expected, &def), Some(3));
        let mut viewer = expected.clone();
        viewer.raster_space = false;
        viewer.density = [0.5, 2.0];
        assert_eq!(reach(&viewer, &def), Some(3));

        let check_refused = |step: &EffectStep| {
            assert!(!preserves_zero(step, &def));
            assert_eq!(reach(step, &def), None);
        };
        for change in [
            |s: &mut EffectStep| s.passes = 2,
            |s: &mut EffectStep| s.keep_orig = false,
            |s: &mut EffectStep| s.backdrop = false,
            |s: &mut EffectStep| s.temporal = true,
            |s: &mut EffectStep| s.params[0].value = Binding::Float(f64::MAX),
            |s: &mut EffectStep| {
                s.spatial = Some(super::super::plan::Spatial { lod: 1, grid: None });
            },
        ] {
            let mut changed = expected.clone();
            change(&mut changed);
            check_refused(&changed);
        }

        def.glsl.as_mut().unwrap().source.push(' ');
        assert!(!preserves_zero(&expected, &def));
        assert_eq!(reach(&expected, &def), None);
    }

    #[test]
    fn deep_glow_maps_intermediates_to_tiles_and_checks_parameter_ranges() {
        let mut def = builtin("blur");
        def.id = "deep-glow.effect".into();
        def.origin = DefinitionOrigin::Local;
        def.glsl.as_mut().unwrap().source = DEEP_GLOW.to_owned();
        def.flags.passes = 8;
        def.flags.keep_orig = true;
        let mut step = step(&def);
        step.params = [
            ("radius", 20.),
            ("exposure", 2.),
            ("threshold", 0.5),
            ("smoothness", 0.1),
            ("falloff", 0.5),
            ("gamma", 2.2),
            ("tintMix", 0.),
            ("saturation", 1.),
            ("aspect", 1.),
            ("angle", 0.),
            ("dispersion", 0.25),
            ("sourceOpacity", 1.),
            ("glowOnly", 0.),
            ("screen", 1.),
            ("highQuality", 0.),
        ]
        .map(|(key, v)| super::super::plan::Param {
            key: key.into(),
            color: false,
            value: Binding::Float(v),
        })
        .to_vec();
        step.params.push(super::super::plan::Param {
            key: "tint".into(),
            color: true,
            value: Binding::Color([1., 0.5, 0.]),
        });

        let size = (501, 301);
        let input = (171, 97, 23, 26);
        let clipped = scissors(&step, &def, input, size).unwrap();
        assert_eq!(clipped.output, (113, 39, 139, 142));
        let tiles = clipped.packed.unwrap();
        assert_eq!(tiles[0], [(84, 47, 15, 16), (334, 47, 15, 16)]);
        assert_eq!(tiles[6], [(59, 22, 65, 66), (309, 22, 65, 66)]);
        let edge = scissors(&step, &def, (18, 97, 8, 26), size).unwrap();
        assert_eq!(edge.output, (0, 39, 501, 142));
        assert_eq!(edge.packed.unwrap()[0][0], (0, 47, 501, 16));

        step.raster_space = false;
        step.density = [0.5, 0.5];
        assert_eq!(
            scissors(&step, &def, input, size).unwrap().output,
            (123, 49, 119, 122)
        );
        step.density = [3., 1.];
        assert!(scissors(&step, &def, input, size).is_none());
        step.density = [1., 1.];
        step.params
            .iter_mut()
            .find(|p| p.key == "gamma")
            .unwrap()
            .value = Binding::Float(0.);
        assert!(!preserves_zero(&step, &def));
        assert!(scissors(&step, &def, input, size).is_none());
    }

    /// Whole chains: empty, reviewed built-ins, the pinned extension; not
    /// generators, unreviewed extensions, backdrop reads, other inputs, or
    /// non-finite parameters.
    #[test]
    fn chains_preserve_zero_only_when_every_step_is_reviewed() {
        let blur = builtin("blur");
        let gradient = builtin("gradient");
        let dots = dot_matrix_definition(DOT_MATRIX);
        let all = [blur.clone(), gradient.clone(), dots.clone()];
        let check = |steps: &[EffectStep]| {
            chain_preserves_zero(steps, |id| all.iter().find(|d| d.id.as_str() == id))
        };
        assert!(check(&[]));
        assert!(check(&[step(&blur)]));
        assert!(check(&[step(&dots)]));
        assert!(check(&[step(&dots), step(&blur), step(&dots)]));
        // Scissored with its own reach (`dot_matrix_reach`): pitch 16 at
        // density 1, `ceil(13.28) + 2`.
        assert_eq!(reach(&step(&dots), &dots), Some(16));
        // A generator draws from nothing.
        assert!(!check(&[step(&gradient)]));
        assert!(!check(&[step(&dots), step(&gradient)]));
        // An unreviewed extension: another source under another id, and
        // the built-in blur's own source registered by an extension.
        let mut other = dots.clone();
        other.id = pm_protocol::common::DefinitionId::from("skyscape-ground");
        assert!(!preserves_zero(&step(&other), &other));
        let mut fork = blur.clone();
        fork.origin = DefinitionOrigin::Fork;
        assert!(!preserves_zero(&step(&fork), &fork));
        // A step whose definition is not found.
        assert!(!chain_preserves_zero(&[step(&dots)], |_| None));
        // Reads beyond its input, or a non-finite parameter.
        let mut backdrop = step(&dots);
        backdrop.backdrop = true;
        assert!(!check(&[backdrop]));
        let mut temporal = step(&blur);
        temporal.temporal = true;
        assert!(!check(&[temporal]));
        let mut keep_orig = step(&dots);
        keep_orig.keep_orig = true;
        assert!(!check(&[keep_orig]));
        let mut passes = step(&dots);
        passes.passes = 2;
        assert!(!check(&[passes]));
        for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            let mut s = step(&dots);
            let pitch = s.params.iter_mut().find(|p| p.key == "pitch").unwrap();
            pitch.value = Binding::Float(bad);
            assert!(!check(&[s]));
        }
        // Position and output addressing do not matter to zero
        // preservation (they do to the scissor).
        let mut raster = step(&blur);
        raster.raster_space = true;
        raster.viewport_safe = false;
        assert!(check(&[raster.clone()]));
        assert_eq!(reach(&raster, &blur), None);
    }

    /// `dot-matrix-resolve`'s reach: `ceil(0.83 · max(pitch, 2) · d) + 2`
    /// output pixels, `d` the larger density its program sees (the step's
    /// in a viewer frame; 1 in raster-space final output, with an owner
    /// binding, or for a density that is not finite and positive), whatever
    /// `viewport_safe` says. Refused for a missing, repeated, mistyped, or
    /// non-finite parameter, a reduced-resolution input, a changed source,
    /// or reads beyond its input.
    #[test]
    fn dot_matrix_resolve_reach_follows_pitch_and_density() {
        let dots = dot_matrix_definition(DOT_MATRIX);
        let with = |pitch: f64, density: [f64; 2], raster_space: bool| {
            let mut s = step(&dots);
            let p = s.params.iter_mut().find(|p| p.key == "pitch").unwrap();
            p.value = Binding::Float(pitch);
            s.density = density;
            s.raster_space = raster_space;
            s
        };
        let at = |pitch: f64, density: f64, raster_space: bool| {
            let s = with(pitch, [density, density], raster_space);
            let unsafe_viewport = reach(&s, &dots);
            let mut safe = s;
            safe.viewport_safe = true;
            assert_eq!(reach(&safe, &dots), unsafe_viewport, "viewport_safe");
            unsafe_viewport
        };
        // Viewer frames (`raster_space` cleared): the step's density.
        for (pitch, density, expected) in [
            // ceil(6.64) + 2, ceil(4.98) + 2, ceil(9.96) + 2
            (8., 1., 9),
            (8., 0.75, 7),
            (8., 1.5, 12),
            // ceil(8.3) + 2, ceil(6.225) + 2, ceil(12.45) + 2
            (10., 1., 11),
            (10., 0.75, 9),
            (10., 1.5, 15),
            // ceil(13.28) + 2, ceil(19.92) + 2
            (16., 1., 16),
            (16., 1.5, 22),
            // Below 2 the program spaces at 2: ceil(1.66) + 2, ceil(2.49) + 2
            (1., 1., 4),
            (0., 1., 4),
            (-5., 1., 4),
            (0.5, 1.5, 5),
            // The extremes of the parameter's range: ceil(53.12) + 2.
            (64., 1., 56),
        ] {
            assert_eq!(
                at(pitch, density, false),
                Some(expected),
                "pitch {pitch} density {density}"
            );
        }
        // Raster-space final output: density 1, whatever the step's.
        assert_eq!(at(10., 0.75, true), Some(11));
        assert_eq!(at(16., 1.5, true), Some(16));
        assert_eq!(at(8., 2., true), Some(9));
        // Anisotropic: the larger axis.
        assert_eq!(reach(&with(10., [0.5, 1.5], false), &dots), Some(15));
        assert_eq!(reach(&with(10., [1.5, 0.5], false), &dots), Some(15));
        // A density the program does not see (not finite or not positive)
        // falls back to 1, as `effect_space` does.
        for density in [f64::NAN, f64::INFINITY, 0., -1.] {
            assert_eq!(at(10., density, false), Some(11), "density {density}");
        }
        // An owner binding: the program sees output pixels.
        let mut owned = with(10., [1.5, 1.5], false);
        owned.owner = Some(std::sync::Arc::new(super::super::plan::Owner {
            valid: true,
            uv: [1., 0., 0., 0., 1., 0., 0., 0., 1.],
            size: [10., 10.],
            bounds: [0., 0., 10., 10.],
            pixel_scale: [1.5, 1.5],
            planar: true,
        }));
        assert_eq!(reach(&owned, &dots), Some(11));
        // A huge pitch saturates (the whole target, `grow`); one whose
        // spread overflows is refused.
        assert_eq!(at(1e300, 1., false), Some(u32::MAX));
        assert_eq!(grow((10, 10, 5, 5), u32::MAX, (100, 100)), (0, 0, 100, 100));
        assert_eq!(at(f64::MAX, 1.5, false), None);
        // Non-finite parameters.
        for bad in [f64::INFINITY, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(at(bad, 1., false), None, "pitch {bad}");
            let mut s = step(&dots);
            let detail = s.params.iter_mut().find(|p| p.key == "detail").unwrap();
            detail.value = Binding::Float(bad);
            assert_eq!(reach(&s, &dots), None, "detail {bad}");
        }
        // Missing, repeated, or not a number.
        let mut missing = step(&dots);
        missing.params.retain(|p| p.key != "pitch");
        assert_eq!(reach(&missing, &dots), None);
        let mut repeated = step(&dots);
        let pitch = repeated
            .params
            .iter()
            .find(|p| p.key == "pitch")
            .unwrap()
            .clone();
        repeated.params.push(pitch);
        assert_eq!(reach(&repeated, &dots), None);
        let mut colour = step(&dots);
        let p = colour.params.iter_mut().find(|p| p.key == "pitch").unwrap();
        p.value = Binding::Color([1., 0., 0.]);
        assert_eq!(reach(&colour, &dots), None);
        // A reduced-resolution input, reads beyond its input, more passes.
        let edits: [fn(&mut EffectStep); 6] = [
            |s| s.spatial = Some(super::super::plan::Spatial { lod: 0, grid: None }),
            |s| s.spatial = Some(super::super::plan::Spatial { lod: 2, grid: None }),
            |s| s.backdrop = true,
            |s| s.keep_orig = true,
            |s| s.passes = 2,
            |s| s.temporal = true,
        ];
        for edit in edits {
            let mut s = step(&dots);
            edit(&mut s);
            assert_eq!(reach(&s, &dots), None, "{s:?}");
        }
        // A changed source, or the source under another id.
        let changed = dot_matrix_definition(&DOT_MATRIX.replace("* 0.5", "* 0.6"));
        assert_eq!(reach(&step(&changed), &changed), None);
        let mut renamed = dots.clone();
        renamed.id = pm_protocol::common::DefinitionId::from("dot-matrix-resolve-2");
        assert_eq!(reach(&step(&renamed), &renamed), None);
        // Built-in blurs keep their own rules (`raster_space` refused).
        let blur = builtin("blur");
        let mut raster = step(&blur);
        raster.raster_space = true;
        assert_eq!(reach(&raster, &blur), None);
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
