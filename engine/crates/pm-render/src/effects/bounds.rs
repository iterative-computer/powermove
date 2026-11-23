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
//!
//! Zero preservation alone ([`preserves_zero`], [`chain_preserves_zero`])
//! also lets the plan leave out a boundary group whose members draw
//! nothing (`plan::draw_layer`): its target is all `+0`, so a chain that
//! maps an all-`+0` input to an all-`+0` output leaves it so, and its
//! normal composite changes nothing beneath it. That needs no locality, so
//! it also holds for reviewed extension sources ([`REVIEWED_EXTENSIONS`],
//! pinned by the SHA-256 of the wrapped source their definition snapshot
//! carries), which [`reach`] does not scissor (yet).

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
    /// The `dot-matrix-resolve` 1.2.0 extension (one pass, apiVersion 1;
    /// review: scene-30fps ANALYSIS §2.1). When every texel it reads is
    /// `+0`: the passthrough branch (`resolve ≥ 99.9 %`) writes `crisp =
    /// +0`; otherwise every generation's 3×3 average `s` is `+0`, so
    /// `cov = 0 < .04` skips it, `best` stays `vec4(0)`, `dots = +0`, and
    /// `o = mix(+0, +0, t) = +0` for `t ∈ [0, 1]`. No time, position, or
    /// colour term reaches the output without a non-zero tap (`u_color`
    /// only enters through `best`, written only when `cov ≥ .04`). With
    /// finite parameters there is no NaN path (`max(s.a, .001)` runs only
    /// when `cov ≥ .04`).
    ///
    /// Not scissored: [`reach`] refuses it, so the empty-group skip and the
    /// scissor are measured as separate optimizations. To scissor it
    /// (scene-30fps Opt 3, ANALYSIS §5.3), give [`reach`] the reach
    /// `ceil(0.83 · max(pitch, 2) · density) + 2` output pixels (taps at
    /// `gp ± 0.33·Pk` with `|gp − px| ≤ Pk / 2`) without its
    /// `viewport_safe` and `raster_space` refusals (the argument above does
    /// not depend on position).
    DotMatrixResolve,
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

/// Reviewed extension sources: definition id, SHA-256 of the wrapped GLSL
/// its definition snapshot carries (`DefinitionSnapshot::glsl.source`: the
/// compat realm's `legacyEffect(item).frag`, `apps/native/js/compat/src/
/// definitions.ts`, preamble and uniform declarations included), and its
/// kind. Any other byte, a param added, renamed or retyped (the uniform
/// declarations are part of the source), `keepOrig` or `backdrop` (they add
/// declarations), or another wrapper, and the source is not reviewed.
const REVIEWED_EXTENSIONS: &[(&str, &str, Kind)] = &[(
    "dot-matrix-resolve",
    // a user profile's v1.2.0 (manifest `apiVersion` 1). Only the digest is
    // committed: the extension's source is private and is not a fixture.
    // Unit tests pin a synthetic stand-in instead (`digest_kind`).
    "df74cebf7460c5a69de557336ee57fa1329a83c0325d38d2f0b95063ae6ca8f6",
    Kind::DotMatrixResolve,
)];

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
        _ => true,
    })
}

/// Whether `step` maps an input whose every texel is `+0` to an output
/// whose every texel is `+0` (or runs no pass), whatever the position, for
/// the parameter values given: a reviewed source ([`kind_of`]), reading
/// nothing but its input (no backdrop, other layers, assets, or frames), on
/// a same-size target, with finite parameters.
pub(crate) fn preserves_zero(step: &EffectStep, definition: &DefinitionSnapshot) -> bool {
    let Some(kind) = kind_of(definition) else {
        return false;
    };
    if step.backdrop
        || step.temporal
        || !step.references.is_empty()
        || !step.assets.is_empty()
        || step.spatial.is_some_and(|s| s.grid.is_some())
        || !finite(step)
    {
        return false;
    }
    match kind {
        Kind::LegacyBlur | Kind::Glow1 | Kind::KeepsClear => true,
        Kind::Blur2 => value(step, "repeatEdgePixels").is_some_and(|v| v <= 0.5),
        Kind::Levels2 => levels2_keeps_zero(step),
        // Reviewed as one pass of its own input (the pin already excludes
        // `u_orig` and `u_backdrop`).
        Kind::DotMatrixResolve => step.passes == 1 && !step.keep_orig,
    }
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

/// The scissor growth of a zero-preserving step (output pixels on every
/// side), or `None` when the step must shade its whole target (and every
/// step after it, the chain's bounds being lost).
pub(crate) fn reach(step: &EffectStep, definition: &DefinitionSnapshot) -> Option<u32> {
    // Reviewed extension sources only preserve zero for now: not
    // scissored (scene-30fps Opt 3 adds `DotMatrixResolve`'s reach here;
    // see [`Kind::DotMatrixResolve`]).
    if kind_of(definition)? == Kind::DotMatrixResolve {
        return None;
    }
    // Inputs other than the layer itself, a different-size grid target,
    // or output-raster addressing: not covered by the argument above.
    if step.raster_space || !step.viewport_safe || !preserves_zero(step, definition) {
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
pub(crate) mod tests {
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

    /// The digest of [`DOT_MATRIX`] counts as a pin for the reviewed id in
    /// tests (see `digest_kind`).
    pub(super) fn is_test_pin(id: &str, digest: &str) -> bool {
        id == "dot-matrix-resolve" && digest == sha256_hex(DOT_MATRIX)
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

    fn step(def: &DefinitionSnapshot) -> EffectStep {
        super::super::plan::effect_step(def, "fx", &[])
    }

    /// The committed digest pins the reviewed id and nothing else: a digest
    /// one bit off, another id, or the empty digest is not reviewed. (The
    /// source behind the real digest is private: only the digest is tested.)
    #[test]
    fn the_committed_digest_pins_only_its_id() {
        let (id, real, kind) = REVIEWED_EXTENSIONS[0];
        assert_eq!(real.len(), 64);
        assert!(real.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(digest_kind(id, real), Some(kind));
        assert_eq!(digest_kind("dot-matrix-resolve-2", real), None);
        assert_eq!(digest_kind("blur", real), None);
        assert_eq!(digest_kind(id, ""), None);
        // Every single-bit change of the digest (flipped in the hex nibbles).
        for at in 0..real.len() {
            let nibble = u8::from_str_radix(&real[at..at + 1], 16).unwrap();
            for bit in 0..4 {
                let mut altered = real.to_owned();
                altered.replace_range(at..at + 1, &format!("{:x}", nibble ^ (1 << bit)));
                assert_eq!(digest_kind(id, &altered), None, "{at}/{bit}");
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
        // Not scissored yet (scene-30fps Opt 3).
        assert_eq!(reach(&step(&dots), &dots), None);
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

    #[test]
    fn growth_and_intersection_stay_in_the_target() {
        assert_eq!(grow((10, 5, 20, 10), 8, (100, 100)), (2, 0, 36, 23));
        assert_eq!(grow((90, 90, 10, 10), 50, (100, 100)), (40, 40, 60, 60));
        assert_eq!(grow((0, 0, 0, 0), 3, (2, 2)), (0, 0, 2, 2));
        assert_eq!(intersect((0, 0, 10, 10), (5, 5, 10, 10)), (5, 5, 5, 5));
        assert_eq!(intersect((0, 0, 4, 4), (6, 6, 2, 2)), (6, 6, 0, 0));
    }
}
