//! `pm-cli parity render` (plan section 25, "Infrastructure": `pm-cli
//! parity`, renders from Phase 2): renders every corpus project the engine
//! supports, at every captured sample, through the job engine's frame path
//! (`pm_export::render::render_frame`, the path exports and agent stills
//! take), and compares it with the legacy capture by section 25's classes:
//!
//! - **Standard color**: per-channel 8-bit difference ≤ 1 for 99.9% of
//!   pixels and ≤ 4 for any pixel, against the opaque capture.
//! - **Video**: projects with video compare natively decoded sources (no
//!   playback proxy) on patch interiors within 2 CV.
//! - **Alpha**: transparent captures' float frames within 1 CV.
//! - **3D**: the 3D pixels (the depth and id surface, grown by a pixel) and
//!   MSAA edges are judged by section 2's 3D bar (`pm_render::parity`);
//!   every other pixel keeps the Standard class.
//!
//! Projects the engine routes (`unsupported_with_definitions`, with the
//! built-in definitions the kernel pushes) are listed with their reasons, as
//! the host would route them to the legacy engine.
//!
//! Frames of projects that use a definition whose WebGL output is itself
//! unstable (amendment 9; `posterize`, docs/engine/effects.md) and that fail
//! the Standard class are listed apart for review, not counted as failures.
//!
//! `--real` adds each capture's real-world project (`real/<slug>`) with its
//! media unpacked from the project file the capture opened.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use pm_export::render::{Unrestricted, render_frame};
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_protocol::definitions::DefinitionSetSnapshot;
use pm_render::parity::{Bar3d, bar_3d, dilate, msaa_edges};
use pm_render::{
    Feature, RenderRequest, Renderer, all_builtin_definitions, unsupported_with_definitions,
    unsupported_with_fonts,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::compare::{Diff, float_max_cv, interior, masked_max, opaque8, rgba8};
use crate::corpus::{self, Project};

pub struct Options {
    pub captures: Vec<PathBuf>,
    pub only: Option<String>,
    pub diffs: Option<PathBuf>,
    pub json: Option<PathBuf>,
    /// The project file of the captures' real projects (`--real`).
    pub real: Option<PathBuf>,
    /// The built-in definitions as the kernel pushes them (default); off
    /// with `--no-definitions`, so effects and transitions route.
    pub definitions: bool,
    /// Render every sample a second time after dropping the renderer's
    /// caches and require bit-identical float buffers (section 2,
    /// "Determinism").
    pub determinism: bool,
}

/// Effect types whose WebGL output is itself unstable (amendment 9; the
/// effects lane's corpus finding, docs/engine/effects.md): a corpus frame
/// of theirs that fails is judged by review.
const WEBGL_UNSTABLE: &[&str] = &["posterize"];

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub captures: Vec<String>,
    pub frames: Vec<FrameResult>,
    pub routed: Vec<Routed>,
    pub skipped: Vec<Routed>,
    pub compared: usize,
    pub failed: usize,
    /// Failing frames of WebGL-unstable definitions, judged by review.
    pub review: usize,
    /// Samples rendered twice (`--determinism`) and how many differed.
    pub determinism_checked: usize,
    pub determinism_failed: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Routed {
    pub project: String,
    pub reasons: Vec<String>,
    /// What would still route once the hosts give their renderers fonts
    /// (docs/engine/text-render.md, "Engine hosts do not give their
    /// renderers fonts yet"): the static check with fonts, effect features
    /// taken from the check with definitions.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub with_fonts: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameResult {
    pub project: String,
    pub time: f64,
    pub class: &'static str,
    pub pass: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub standard: Option<Diff>,
    /// Video class: interior pixels compared and the largest difference.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub video: Option<(usize, u8)>,
    /// Alpha class: the largest float difference in CV.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha_max_cv: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub three_d: Option<Bar3dReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `--determinism`: whether a second render, caches dropped, was
    /// bit-identical (set on the sample's first result).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deterministic: Option<bool>,
    /// Set when a failing frame is judged by review instead (`webgl-unstable`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub review: Option<&'static str>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bar3dReport {
    pub fraction_within_2: f64,
    pub max_cv: u8,
    pub max_cv_outside_edges: u8,
    pub object_ssim: Option<f64>,
}

impl From<&Bar3d> for Bar3dReport {
    fn from(b: &Bar3d) -> Self {
        Bar3dReport {
            fraction_within_2: b.fraction_within_2,
            max_cv: b.max_cv,
            max_cv_outside_edges: b.max_cv_outside_edges,
            object_ssim: b.object_ssim,
        }
    }
}

fn ticks(seconds: f64) -> Ticks {
    Ticks::new((seconds * pm_time::TICKS_PER_SECOND as f64).round() as i64).unwrap_or(Ticks::ZERO)
}

/// One render through the job engine's frame path, read back.
fn frame(
    renderer: &mut Renderer,
    p: &Project,
    time: f64,
    request: &RenderRequest,
) -> Result<Vec<f32>, String> {
    let f = render_frame(
        renderer,
        &p.document,
        &p.composition,
        ticks(time),
        request,
        &p.media,
        &Unrestricted,
        0,
        &AtomicBool::new(false),
    )
    .map_err(|e| e.to_string())?;
    if !f.failures.is_empty() {
        return Err(format!("layer failures {:?}", f.failures));
    }
    Ok(f.rgba)
}

/// Object pixels, MSAA edges, and the 3D class (their union, grown by a pixel).
type Masks3d = (Vec<bool>, Vec<bool>, Vec<bool>);

/// The 3D class masks of a frame: object pixels, MSAA edges, and their union
/// grown by a pixel.
fn three_d_masks(
    renderer: &mut Renderer,
    p: &Project,
    time: f64,
    request: &RenderRequest,
    ours: &[u8],
) -> Result<Masks3d, String> {
    let mut with_ids = request.clone();
    with_ids.depth_id = true;
    let ids = renderer
        .render(
            &p.document,
            &p.composition,
            ticks(time),
            &with_ids,
            &p.media,
        )
        .map_err(|e| e.to_string())?;
    let surface = renderer
        .read_depth_id(&ids)
        .map_err(|e| e.to_string())?
        .ok_or("no depth and id surface")?;
    renderer.set_3d_msaa(false);
    let plain = frame(renderer, p, time, request);
    renderer.set_3d_msaa(true);
    let plain = opaque8(&plain?);
    let (w, h) = (
        request.output_size.width as usize,
        request.output_size.height as usize,
    );
    let object: Vec<bool> = surface
        .chunks_exact(4)
        .map(|px| px[0].is_finite())
        .collect();
    let edges = msaa_edges(ours, &plain, w, h);
    let class: Vec<bool> = dilate(&object, w, h)
        .iter()
        .zip(&edges)
        .map(|(a, b)| *a || *b)
        .collect();
    Ok((object, edges, class))
}

fn interpret_videos(renderer: &mut Renderer, p: &Project) {
    for (id, _) in corpus::video_assets(&p.document) {
        let Some(path) = p.media.paths.get(&id) else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let Ok(mut c) = renderer.video_color(path) else {
            continue;
        };
        // The recorded deviations of pm-render's corpus comparison
        // (docs/engine/pm-render.md): Interpret Footage settings, not renderer
        // changes.
        match name {
            "hevc-alpha.mov" => c.alpha = pm_protocol::color::AlphaMode::Straight,
            "av1.mp4" | "vp9-opus.webm" => c.matrix = pm_protocol::color::MatrixCoefficients::Bt709,
            _ => continue,
        }
        renderer.interpret(path, Some(c));
    }
}

fn save_diff(dir: &Path, p: &Project, time: f64, w: u32, h: u32, ours: &[u8], legacy: &[u8]) {
    let _ = std::fs::create_dir_all(dir);
    let stem = format!("{}-{time}", p.name.replace('/', "_"));
    let _ = corpus::write_png(&dir.join(format!("{stem}-engine.png")), w, h, ours);
    let _ = corpus::write_png(&dir.join(format!("{stem}-legacy.png")), w, h, legacy);
}

fn compare_sample(
    renderer: &mut Renderer,
    p: &Project,
    s: &corpus::Sample,
    video: bool,
    diffs: Option<&Path>,
    determinism: bool,
) -> Vec<FrameResult> {
    let mut checked = None;
    let mut out = compare_sample_inner(renderer, p, s, video, diffs, determinism, &mut checked);
    if let Some(first) = out.first_mut() {
        first.deterministic = checked;
    }
    out
}

fn compare_sample_inner(
    renderer: &mut Renderer,
    p: &Project,
    s: &corpus::Sample,
    video: bool,
    diffs: Option<&Path>,
    determinism: bool,
    checked: &mut Option<bool>,
) -> Vec<FrameResult> {
    let mut out = vec![];
    let result = |class, pass, error: Option<String>| FrameResult {
        project: p.name.clone(),
        time: s.time,
        class,
        pass,
        standard: None,
        video: None,
        alpha_max_cv: None,
        three_d: None,
        error,
        deterministic: None,
        review: None,
    };
    // The capture's motion blur: 16 samples opaque, 20 transparent.
    let mut request = RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
    request.motion_blur = pm_render::motion::MotionBlur::Samples(16);
    let rgba = match frame(renderer, p, s.time, &request) {
        Ok(f) => f,
        Err(e) => {
            out.push(result("error", false, Some(e)));
            return out;
        }
    };
    let deterministic = if determinism {
        renderer.trim(0);
        match frame(renderer, p, s.time, &request) {
            Ok(again) => Some(
                again.len() == rgba.len()
                    && again
                        .iter()
                        .zip(&rgba)
                        .all(|(a, b)| a.to_bits() == b.to_bits()),
            ),
            Err(_) => Some(false),
        }
    } else {
        None
    };
    *checked = deterministic;
    let ours = opaque8(&rgba);
    let (w, h, reference) = match corpus::read_png(&s.png) {
        Ok(r) => r,
        Err(e) => {
            out.push(result("error", false, Some(e)));
            return out;
        }
    };
    if (w, h) != (s.width, s.height) {
        out.push(result("error", false, Some("capture size mismatch".into())));
        return out;
    }
    if video {
        let mask = interior(&reference, w, h);
        let (max, n) = masked_max(&ours, &reference, &mask);
        let pass = max <= 2 && n > 0;
        if !pass && let Some(d) = diffs {
            save_diff(d, p, s.time, w, h, &ours, &reference);
        }
        let mut r = result("video", pass, None);
        r.video = Some((n, max));
        out.push(r);
        return out;
    }
    let three_d = if corpus::has_3d_objects(&p.document) {
        match three_d_masks(renderer, p, s.time, &request, &ours) {
            Ok(m) => Some(m),
            Err(e) => {
                out.push(result("error", false, Some(e)));
                return out;
            }
        }
    } else {
        None
    };
    let (wu, hu) = (w as usize, h as usize);
    if let Some(float) = &s.float {
        let mut transparent = request;
        transparent.background = false;
        transparent.motion_blur = pm_render::motion::MotionBlur::Samples(20);
        let reference = corpus::read_float(float);
        match (frame(renderer, p, s.time, &transparent), reference) {
            (Ok(ours_f), Ok(reference_f)) if ours_f.len() == reference_f.len() => {
                let (max, bar) = match &three_d {
                    None => (float_max_cv(&ours_f, &reference_f, None), None),
                    Some((object, edges, class)) => {
                        let rest: Vec<bool> = class.iter().map(|c| !c).collect();
                        let bar =
                            bar_3d(&rgba8(&reference_f), &rgba8(&ours_f), edges, object, wu, hu);
                        (float_max_cv(&ours_f, &reference_f, Some(&rest)), Some(bar))
                    }
                };
                let pass = max <= 1.0 && bar.as_ref().is_none_or(|b| b.pass());
                let mut r = result("alpha", pass, None);
                r.alpha_max_cv = Some(max);
                r.three_d = bar.as_ref().map(Bar3dReport::from);
                out.push(r);
            }
            (Err(e), _) | (_, Err(e)) => out.push(result("error", false, Some(e))),
            _ => out.push(result(
                "error",
                false,
                Some("float frame size mismatch".into()),
            )),
        }
    }
    let (d, bar) = match &three_d {
        None => (Diff::of(&ours, &reference, None), None),
        Some((object, edges, class)) => {
            let rest: Vec<bool> = class.iter().map(|c| !c).collect();
            (
                Diff::of(&ours, &reference, Some(&rest)),
                Some(bar_3d(&reference, &ours, edges, object, wu, hu)),
            )
        }
    };
    let pass = d.passes() && bar.as_ref().is_none_or(|b| b.pass());
    if !pass && let Some(dir) = diffs {
        save_diff(dir, p, s.time, w, h, &ours, &reference);
    }
    let mut r = result(
        if bar.is_some() {
            "standard+3d"
        } else {
            "standard"
        },
        pass,
        None,
    );
    r.standard = Some(d);
    r.three_d = bar.as_ref().map(Bar3dReport::from);
    out.push(r);
    out
}

fn describe(r: &FrameResult) -> String {
    let head = format!("{:<44} t={:<7} {:<12}", r.project, r.time, r.class);
    if let Some(e) = &r.error {
        return format!("{head} error: {e}");
    }
    let mut parts = vec![];
    if let Some(d) = &r.standard {
        parts.push(format!(
            "{} px exact {:.2}% ≤1 {:.3}% max {}",
            d.pixels,
            100.0 * d.exact as f64 / d.pixels.max(1) as f64,
            100.0 * d.fraction_within_one(),
            d.max
        ));
    }
    if let Some((n, max)) = r.video {
        parts.push(format!("{n} interior px max {max}"));
    }
    if let Some(m) = r.alpha_max_cv {
        parts.push(format!("float max {m:.4} CV"));
    }
    if let Some(b) = &r.three_d {
        parts.push(format!(
            "3D ≤2 {:.4}% max {} outside edges {} SSIM {:.5}",
            100.0 * b.fraction_within_2,
            b.max_cv,
            b.max_cv_outside_edges,
            b.object_ssim.unwrap_or(f64::NAN)
        ));
    }
    format!(
        "{head} {}{}",
        parts.join("; "),
        match (r.pass, r.review) {
            (true, _) => "",
            (false, Some(_)) => "  REVIEW (WebGL-unstable)",
            (false, None) => "  FAILED",
        }
    )
}

/// Whether any effect of the document is WebGL-unstable.
fn webgl_unstable(document: &pm_model::Document) -> bool {
    fn walk(n: &pm_model::Node) -> bool {
        match n.kind() {
            pm_model::NodeKind::Object(map) => {
                let unstable = map.iter().any(|(k, v)| {
                    k.to_string_lossy() == "fx"
                        && matches!(v.kind(), pm_model::NodeKind::Array(items) if items.iter().any(|e| {
                            e.get("type").is_some_and(|t| matches!(t.kind(), pm_model::NodeKind::String(s)
                                if WEBGL_UNSTABLE.contains(&s.to_string_lossy().as_str())))
                        }))
                });
                unstable || map.iter().any(|(_, v)| walk(v))
            }
            pm_model::NodeKind::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    document.root().is_some_and(walk)
}

/// Extension layer definition ids a document uses.
fn extension_definitions(document: &pm_model::Document) -> Vec<String> {
    fn walk(n: &pm_model::Node, out: &mut Vec<String>) {
        match n.kind() {
            pm_model::NodeKind::Object(map) => {
                let is_extension = map.get(b"type".as_slice()).is_some_and(|t| {
                    matches!(t.kind(), pm_model::NodeKind::String(s) if s.to_string_lossy() == "extension")
                });
                if is_extension
                    && let Some(d) = map.get(b"d".as_slice())
                    && let Some(def) = d.get("definition")
                    && let pm_model::NodeKind::String(s) = def.kind()
                {
                    out.push(s.to_string_lossy());
                }
                for v in map.values() {
                    walk(v, out);
                }
            }
            pm_model::NodeKind::Array(items) => items.iter().for_each(|i| walk(i, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Some(root) = document.root() {
        walk(root, &mut out);
    }
    out.sort();
    out.dedup();
    out
}

/// The definitions the capture app held: the built-ins, with every other
/// extension layer definition the project names marked missing (the corpus
/// app had no third-party extension installed).
fn capture_definitions(document: &pm_model::Document) -> DefinitionSetSnapshot {
    let mut set = all_builtin_definitions();
    set.missing = extension_definitions(document)
        .into_iter()
        .filter(|id| !set.definitions.iter().any(|d| d.id.as_str() == id))
        .map(|id| id.as_str().into())
        .collect();
    set
}

/// The real project's media, unpacked from the file the capture opened.
fn real_media(pmv: &Path, record_sha: Option<String>) -> Result<crate::render::Loaded, String> {
    let bytes = std::fs::read(pmv).map_err(|e| format!("{}: {e}", pmv.display()))?;
    let sha: String = Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if let Some(expected) = record_sha
        && expected != sha
    {
        return Err(format!(
            "{}: SHA-256 {sha} is not the captured file's {expected}",
            pmv.display()
        ));
    }
    crate::render::load_project(pmv, None, &[]).map_err(|e| match e {
        crate::render::Failure::Usage(m) | crate::render::Failure::Failed(m) => m,
        crate::render::Failure::Unsupported(r) => r.join(", "),
    })
}

/// Features of the check with definitions that only effects, transitions,
/// and extension definitions raise (the check without definitions lists
/// them differently).
fn effect_feature(f: &Feature) -> bool {
    matches!(
        f,
        Feature::Effects
            | Feature::Transition
            | Feature::Adjustment
            | Feature::ExtensionDefinition(_)
            | Feature::Definition(_)
            | Feature::ShaderFallback(_)
    )
}

/// What still routes with fonts: the static check with fonts for every
/// feature but effects, and the check with definitions for effects.
fn routing_with_fonts(p: &Project, with_definitions: &[pm_render::Unsupported]) -> Vec<String> {
    let mut out: Vec<String> = unsupported_with_fonts(&p.document, &p.composition, Some(&p.media))
        .iter()
        .filter(|u| !effect_feature(&u.feature))
        .chain(
            with_definitions
                .iter()
                .filter(|u| effect_feature(&u.feature)),
        )
        .map(|u| u.feature.to_string())
        .collect();
    out.sort();
    out.dedup();
    if out.is_empty() {
        out.push("nothing (drawn once the hosts give fonts)".into());
    }
    out
}

/// Runs the comparison. Returns the report; the caller decides the exit
/// status (failures, or nothing compared).
pub fn run(options: &Options) -> Result<Report, String> {
    if options.captures.is_empty() {
        return Err(
            "no parity corpus capture found; capture one (docs/engine/corpus.md) or pass --capture"
                .into(),
        );
    }
    let mut renderer = Renderer::headless().map_err(|e| e.to_string())?;
    let mut report = Report {
        captures: options
            .captures
            .iter()
            .map(|c| c.display().to_string())
            .collect(),
        ..Report::default()
    };
    if !options.definitions {
        eprintln!("no definitions: effects, transitions, and extension layers route");
    }
    for capture in &options.captures {
        let mut dirs = corpus::projects(capture);
        if options.real.is_some() {
            dirs.extend(corpus::real_projects(capture));
        }
        for dir in dirs {
            let mut p = match corpus::load_project(capture, &dir) {
                Ok(p) => p,
                Err(e) => {
                    report.skipped.push(Routed {
                        project: dir.display().to_string(),
                        reasons: vec![e],
                        with_fonts: vec![],
                    });
                    continue;
                }
            };
            if options
                .only
                .as_deref()
                .is_some_and(|o| !p.name.starts_with(o))
            {
                continue;
            }
            // Kept alive while the real project renders (its unpacked media).
            let mut _unpacked = None;
            if p.name.starts_with("real/")
                && let Some(pmv) = &options.real
            {
                let record = corpus::read_json(&dir.join("project.json")).ok();
                let sha = record
                    .as_ref()
                    .and_then(|r| corpus::string(corpus::member(r, "fileSha256")));
                match real_media(pmv, sha) {
                    Ok(loaded) => {
                        for (asset, path) in &loaded.media.paths {
                            p.media.insert(asset.clone(), path.clone());
                        }
                        _unpacked = Some(loaded);
                    }
                    Err(e) => {
                        report.skipped.push(Routed {
                            project: p.name.clone(),
                            reasons: vec![e],
                            with_fonts: vec![],
                        });
                        continue;
                    }
                }
            }
            if !p.missing_media.is_empty() {
                report.skipped.push(Routed {
                    project: p.name.clone(),
                    reasons: vec![format!("media not available: {:?}", p.missing_media)],
                    with_fonts: vec![],
                });
                continue;
            }
            let set = if options.definitions {
                capture_definitions(&p.document)
            } else {
                DefinitionSetSnapshot::default()
            };
            renderer
                .set_definitions(&set)
                .map_err(|e| format!("definitions: {e}"))?;
            let issues = unsupported_with_definitions(
                &p.document,
                &p.composition,
                Some(&p.media),
                renderer.effects(),
            );
            if !issues.is_empty() {
                let mut reasons: Vec<String> =
                    issues.iter().map(|u| u.feature.to_string()).collect();
                reasons.sort();
                reasons.dedup();
                let with_fonts = if issues.iter().any(|u| matches!(u.feature, Feature::Text(_))) {
                    routing_with_fonts(&p, &issues)
                } else {
                    vec![]
                };
                report.routed.push(Routed {
                    project: p.name.clone(),
                    reasons,
                    with_fonts,
                });
                continue;
            }
            let videos = corpus::video_assets(&p.document);
            if videos.iter().any(|(_, proxied)| *proxied) {
                report.skipped.push(Routed {
                    project: p.name.clone(),
                    reasons: vec![
                        "the legacy frame shows a playback proxy (not a native decode)".into(),
                    ],
                    with_fonts: vec![],
                });
                continue;
            }
            interpret_videos(&mut renderer, &p);
            let unstable = webgl_unstable(&p.document);
            for s in &p.samples {
                for r in compare_sample(
                    &mut renderer,
                    &p,
                    s,
                    !videos.is_empty(),
                    options.diffs.as_deref(),
                    options.determinism,
                ) {
                    let mut r = r;
                    if let Some(same) = r.deterministic {
                        report.determinism_checked += 1;
                        if !same {
                            report.determinism_failed += 1;
                            eprintln!("{} t={}: NOT DETERMINISTIC", r.project, r.time);
                        }
                    }
                    if !r.pass && unstable && r.error.is_none() {
                        r.review = Some("webgl-unstable");
                    }
                    eprintln!("{}", describe(&r));
                    report.compared += 1;
                    if !r.pass {
                        if r.review.is_some() {
                            report.review += 1;
                        } else {
                            report.failed += 1;
                        }
                    }
                    report.frames.push(r);
                }
            }
            renderer.trim(256 << 20);
        }
    }
    eprintln!("\nrouted to the legacy engine:");
    for r in &report.routed {
        eprintln!("  {:<44} {}", r.project, r.reasons.join(", "));
        if !r.with_fonts.is_empty() {
            eprintln!("  {:<44}   with fonts: {}", "", r.with_fonts.join(", "));
        }
    }
    if !report.skipped.is_empty() {
        eprintln!("\nskipped:");
        for r in &report.skipped {
            eprintln!("  {:<44} {}", r.project, r.reasons.join(", "));
        }
    }
    eprintln!(
        "\ncompared {} frames; {} failed; {} for review (WebGL-unstable); {} projects routed",
        report.compared,
        report.failed,
        report.review,
        report.routed.len()
    );
    if options.determinism {
        eprintln!(
            "determinism: {} samples rendered twice (caches dropped), {} differed",
            report.determinism_checked, report.determinism_failed
        );
    }
    if let Some(path) = &options.json {
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(report)
}
