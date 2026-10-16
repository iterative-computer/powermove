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
//! Projects the engine routes (`unsupported_with_media`) are listed with
//! their reasons, as the host would route them to the legacy engine.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use pm_export::render::{Unrestricted, render_frame};
use pm_protocol::color::Precision;
use pm_protocol::common::{Size, Ticks};
use pm_render::parity::{Bar3d, bar_3d, dilate, msaa_edges};
use pm_render::{RenderRequest, Renderer, unsupported_with_media};
use serde::Serialize;

use crate::compare::{Diff, float_max_cv, interior, masked_max, opaque8, rgba8};
use crate::corpus::{self, Project};

pub struct Options {
    pub captures: Vec<PathBuf>,
    pub only: Option<String>,
    pub diffs: Option<PathBuf>,
    pub json: Option<PathBuf>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub captures: Vec<String>,
    pub frames: Vec<FrameResult>,
    pub routed: Vec<Routed>,
    pub skipped: Vec<Routed>,
    pub compared: usize,
    pub failed: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Routed {
    pub project: String,
    pub reasons: Vec<String>,
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
    let mut with_ids = *request;
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
        if r.pass { "" } else { "  FAILED" }
    )
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
    for capture in &options.captures {
        for dir in corpus::projects(capture) {
            let p = match corpus::load_project(capture, &dir) {
                Ok(p) => p,
                Err(e) => {
                    report.skipped.push(Routed {
                        project: dir.display().to_string(),
                        reasons: vec![e],
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
            if !p.missing_media.is_empty() {
                report.skipped.push(Routed {
                    project: p.name.clone(),
                    reasons: vec![format!("media not available: {:?}", p.missing_media)],
                });
                continue;
            }
            let issues = unsupported_with_media(&p.document, &p.composition, &p.media);
            if !issues.is_empty() {
                let mut reasons: Vec<String> =
                    issues.iter().map(|u| u.feature.to_string()).collect();
                reasons.sort();
                reasons.dedup();
                report.routed.push(Routed {
                    project: p.name.clone(),
                    reasons,
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
                });
                continue;
            }
            interpret_videos(&mut renderer, &p);
            for s in &p.samples {
                for r in compare_sample(
                    &mut renderer,
                    &p,
                    s,
                    !videos.is_empty(),
                    options.diffs.as_deref(),
                ) {
                    eprintln!("{}", describe(&r));
                    report.compared += 1;
                    if !r.pass {
                        report.failed += 1;
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
    }
    if !report.skipped.is_empty() {
        eprintln!("\nskipped:");
        for r in &report.skipped {
            eprintln!("  {:<44} {}", r.project, r.reasons.join(", "));
        }
    }
    eprintln!(
        "\ncompared {} frames; {} failed; {} projects routed",
        report.compared,
        report.failed,
        report.routed.len()
    );
    if let Some(path) = &options.json {
        let text = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(report)
}
