//! Render parity against the legacy renderer on the parity corpus (plan
//! section 25, "Render, Standard color"): every corpus project this
//! renderer supports (by `unsupported_with_media`) is rendered at every
//! captured sample and compared with the capture's opaque frame. Projects it
//! does not support are listed with the reason and skipped.
//!
//! Projects with video use section 25's video class: the legacy frames come
//! from Chromium, so only natively decoded sources (no playback proxy) are
//! compared, on patch interiors, within 2 CV. `hevc-alpha.mov` is
//! interpreted as it was encoded (BT.709 coefficients, straight alpha; see
//! below).
//!
//! Projects with 3D object layers use section 25's 3D class on the 3D
//! pixels: the pixels where a 3D surface is on top (the depth and id
//! surface) grown by one pixel, plus the MSAA edges (pixels that change when
//! the 3D pass renders without MSAA), are judged by section 2's 3D bar
//! (`pm_render::parity`); every other pixel keeps the Standard class.
//!
//! `PM_ENGINE_CORPUS` picks captures (comma-separated; default: the newest
//! `v1-*` and the newest `render-*`);
//! `PM_RENDER_ONLY` limits to project names with a prefix;
//! `PM_RENDER_DIFFS=<dir>` writes engine and reference PNGs of failures there.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::parity::{bar_3d, dilate, msaa_edges};
use pm_render::{RenderRequest, Renderer, builtin_definitions, unsupported_with_definitions};
use support::*;

/// `render/still-*` projects and whether the renderer draws them.
const STILL_CLASSES: &[(&str, bool)] = &[
    ("render/still-gray-alpha-png", true),
    ("render/still-indexed-png", true),
    ("render/still-photo-gray-jpg", true),
    ("render/still-photo-heic", true),
    ("render/still-photo-lossy-webp", true),
    ("render/still-photo-lossless-opaque-webp", true),
    ("render/still-photo-tiff", true),
    ("render/still-16bit-png", false),
    ("render/still-p3-heic", false),
    ("render/still-p3-jpg", false),
    ("render/still-p3-png", false),
    ("render/still-photo-420-jpg", true),
    ("render/still-photo-422-jpg", true),
    ("render/still-photo-422-tall-mcu-jpg", true),
    ("render/still-photo-444-jpg", true),
    ("render/still-photo-444-tall-mcu-jpg", true),
    ("render/still-photo-alpha-webp", false),
    ("render/still-photo-avif", false),
    ("render/still-photo-lossless-webp", false),
    ("render/still-photo-orientation-6-jpg", false),
    ("render/still-photo-progressive-jpg", true),
];

/// The effects whose corpus frames are listed apart rather than failed:
/// the part of `pm_render::effects::WEBGL_UNSTABLE` the app corpus has
/// shown departing (Posterize's one-level boundary flips). Levels and Color
/// are unstable at the shader level only; a corpus frame of theirs that
/// departs fails.
const CORPUS_UNSTABLE: &[&str] = &["posterize"];

/// Whether any effect in the document is in [`CORPUS_UNSTABLE`].
fn webgl_unstable(document: &pm_model::Document) -> bool {
    fn walk(n: &pm_model::Node) -> bool {
        match n.kind() {
            pm_model::NodeKind::Object(map) => {
                let unstable_fx = map.iter().any(|(k, v)| {
                    k.to_string_lossy() == "fx"
                        && matches!(v.kind(), pm_model::NodeKind::Array(items) if items.iter().any(|e| {
                            e.get("type").is_some_and(|t| matches!(t.kind(), pm_model::NodeKind::String(s)
                                if CORPUS_UNSTABLE.contains(&s.to_string_lossy().as_str())))
                        }))
                });
                unstable_fx || map.iter().any(|(_, v)| walk(v))
            }
            pm_model::NodeKind::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    document.root().is_some_and(walk)
}

/// Pixels whose every channel is within 1 CV (float values, 8-bit scale).
fn float_fraction_within_one(a: &[f32], b: &[f32]) -> f64 {
    let pixels = a.len() / 4;
    let within = a
        .chunks_exact(4)
        .zip(b.chunks_exact(4))
        .filter(|(x, y)| {
            x.iter()
                .zip(y.iter())
                .all(|(p, q)| ((p - q).abs() * 255.0) <= 1.0 + 1e-3)
        })
        .count();
    within as f64 / pixels.max(1) as f64
}

/// Frames where today's renderer departs from the exact composite and the
/// engine does not follow (docs/engine/composite.md, "Today's composite
/// drift"; proposed appendix A item): `FRAG_COMPOSITE` runs over the whole
/// frame for every blend layer and sends every pixel through unpremultiply
/// and re-premultiply, so on a translucent stack the half-float target loses
/// up to a few code values over many blend layers. The engine passes pixels
/// outside the layer through exactly. Bounds as measured on the capture
/// (largest transparent-float and opaque differences, in CV); the rest of the
/// class still applies.
const COMPOSITE_DRIFT: &[(&str, f64, u8)] = &[("composite/blend-modes-alpha", 5.0, 5)];

/// Frames outside the render class by a measured, explained residual:
/// (project, most pixels off by more than 1, largest difference). A frame
/// within its pin is listed as known instead of failed; one beyond it fails.
///
/// - `composition/metadata`: the radius-24 rectangle's Canvas2D raster has
///   two pixels where one of Chromium's four MSAA samples falls on the other
///   side of a tessellated arc edge (0.15/256 px from it); the compat level 1
///   emulation reproduces the raster otherwise exactly
///   (docs/engine/vector.md, "Residuals"; `shape-corpus-rect-r24-s1` in
///   pm-vector's Canvas2D parity test).
const KNOWN_RESIDUALS: &[(&str, usize, u8)] = &[("composition/metadata", 6, 43)];

#[test]
fn corpus_render_parity() {
    let captures = corpus_dirs();
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_RENDER_ONLY").ok();
    let diffs = std::env::var("PM_RENDER_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
    // The built-in effects and transitions as the kernel snapshots them
    // (the captures load only built-in and missing definitions).
    renderer
        .set_definitions(&builtin_definitions())
        .expect("built-in definitions");
    let mut compared = 0;
    let mut failed = Vec::new();
    // Frames of projects using a WebGL-unstable definition that fail only
    // at isolated pixels (amendment 9; judged by review, listed apart).
    let mut unstable: Vec<String> = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = std::collections::BTreeMap::new();
    for (capture, dir) in captures
        .iter()
        .flat_map(|c| projects(c).into_iter().map(move |d| (c.clone(), d)))
    {
        let p = load_project(&capture, &dir);
        if only.as_deref().is_some_and(|o| !p.name.starts_with(o)) {
            continue;
        }
        if !p.missing_media.is_empty() {
            skipped.push((
                p.name.clone(),
                format!("media not available: {:?}", p.missing_media),
            ));
            continue;
        }
        // Text and captions are judged by the text class (tests/text_corpus.rs).
        if support::text::has_text(&p.document) {
            skipped.push((p.name.clone(), "text class: tests/text_corpus.rs".into()));
            continue;
        }
        let issues = unsupported_with_definitions(
            &p.document,
            &p.composition,
            Some(&p.resolver),
            renderer.effects(),
        );
        seen.insert(p.name.clone(), issues.is_empty());
        if !issues.is_empty() {
            let mut reasons: Vec<String> = issues.iter().map(|u| u.feature.to_string()).collect();
            reasons.dedup();
            skipped.push((p.name.clone(), reasons.join(", ")));
            continue;
        }
        // Video sources: natively decoded only, interpreted as encoded.
        let mut video = false;
        let mut proxied = false;
        let root = p.document.root().unwrap();
        if let Some(assets) = root.get("assets")
            && let pm_model::NodeKind::Object(map) = assets.kind()
        {
            {
                for (id, record) in map {
                    let id = id.to_string_lossy();
                    let kind = record.get("kind").and_then(|k| match k.kind() {
                        pm_model::NodeKind::String(s) => Some(s.to_string_lossy()),
                        _ => None,
                    });
                    if kind.as_deref() != Some("video") {
                        continue;
                    }
                    video = true;
                    if matches!(
                        record.get("playbackProxy").map(|n| n.kind()),
                        Some(pm_model::NodeKind::Bool(true))
                    ) {
                        proxied = true;
                    }
                    let Some(path) = p.resolver.paths.get(&id) else {
                        continue;
                    };
                    let name = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or_default();
                    let mut c = renderer.video_color(path).expect("video color");
                    match name {
                        // Encoded with straight alpha (its color is red where
                        // alpha is 0); the probe applies Apple's premultiplied
                        // default for HEVC with alpha. Chromium reads it as
                        // straight (and with BT.601 coefficients, as the
                        // untagged rule does).
                        "hevc-alpha.mov" => {
                            c.alpha = pm_protocol::color::AlphaMode::Straight;
                        }
                        // Untagged SD: section 9 reads BT.601; Chromium reads
                        // untagged VP9 and AV1 with BT.709 coefficients
                        // (measured here; H.264, HEVC, and VP8 read BT.601 in
                        // both). A recorded deviation (docs/engine/pm-render.md).
                        "av1.mp4" | "vp9-opus.webm" => {
                            c.matrix = pm_protocol::color::MatrixCoefficients::Bt709;
                        }
                        _ => continue,
                    }
                    renderer.interpret(path, Some(c));
                }
            }
        }
        if proxied {
            skipped.push((
                p.name.clone(),
                "the legacy frame shows a playback proxy (not a native decode)".into(),
            ));
            continue;
        }
        for s in &p.samples {
            // The capture's motion blur: 16 samples opaque, 20 transparent.
            let mut request =
                RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
            request.motion_blur = pm_render::motion::MotionBlur::Samples(16);
            let frame = match renderer.render(
                &p.document,
                &p.composition,
                ticks(s.time),
                &request,
                &p.resolver,
            ) {
                Ok(f) => f,
                Err(e) => {
                    failed.push(format!("{} t={}: render error {e}", p.name, s.time));
                    continue;
                }
            };
            if !frame.failures.is_empty() {
                failed.push(format!(
                    "{} t={}: layer failures {:?}",
                    p.name, s.time, frame.failures
                ));
            }
            let rgba = renderer.read_rgba(&frame).unwrap();
            let ours = opaque8(&rgba);
            let (w, h, reference) = read_png(&s.png);
            assert_eq!((w, h), (s.width, s.height));
            if video {
                let mask = interior(&reference, w, h);
                let (max, n) = masked_max(&ours, &reference, &mask);
                compared += 1;
                let line = format!(
                    "{:<40} t={:<6} {:>7} interior px (video class)  max {}",
                    p.name, s.time, n, max
                );
                eprintln!("{line}");
                if max > 2 || n == 0 {
                    failed.push(line);
                    if let Some(dir) = &diffs {
                        std::fs::create_dir_all(dir).unwrap();
                        let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                        write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                        write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                    }
                }
                continue;
            }
            // 3D class: the 3D pixels and MSAA edges, measured on our renders.
            let three_d = if support::has_3d_objects(&p.document) {
                let mut with_ids = request.clone();
                with_ids.depth_id = true;
                let ids = renderer
                    .render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        &with_ids,
                        &p.resolver,
                    )
                    .expect("depth and id render");
                let surface = renderer.read_depth_id(&ids).unwrap().unwrap();
                renderer.set_3d_msaa(false);
                let plain = renderer
                    .render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        &request,
                        &p.resolver,
                    )
                    .expect("render without MSAA");
                renderer.set_3d_msaa(true);
                let plain = opaque8(&renderer.read_rgba(&plain).unwrap());
                let (w, h) = (s.width as usize, s.height as usize);
                let object: Vec<bool> = surface
                    .chunks_exact(4)
                    .map(|px| px[0].is_finite())
                    .collect();
                let edges = msaa_edges(&ours, &plain, w, h);
                let class: Vec<bool> = dilate(&object, w, h)
                    .iter()
                    .zip(&edges)
                    .map(|(a, b)| *a || *b)
                    .collect();
                Some((object, edges, class))
            } else {
                None
            };
            // Transparent captures (`renderToPixels(…, {transparent: true})`):
            // premultiplied float values, section 25's alpha class (within
            // 1 CV of the reference).
            if let Some(float) = &s.float {
                let mut request = request;
                request.background = false;
                request.motion_blur = pm_render::motion::MotionBlur::Samples(20);
                let frame = renderer
                    .render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        &request,
                        &p.resolver,
                    )
                    .expect("transparent render");
                let ours = renderer.read_rgba(&frame).unwrap();
                let reference = read_float(float);
                let (max, bar) = match &three_d {
                    None => (float_max_cv(&ours, &reference), None),
                    Some((object, edges, class)) => {
                        let rest: Vec<bool> = class.iter().map(|c| !c).collect();
                        let (w, h) = (s.width as usize, s.height as usize);
                        let bar = bar_3d(&rgba8(&reference), &rgba8(&ours), edges, object, w, h);
                        (float_max_cv_masked(&ours, &reference, &rest), Some(bar))
                    }
                };
                let line = format!(
                    "{:<40} t={:<6} transparent float  max {max:.4} CV{}",
                    p.name,
                    s.time,
                    bar.as_ref().map_or(String::new(), |b| format!(
                        " outside 3D; 3D bar ≤2 {:.4}% max outside edges {} SSIM {:.5}",
                        100.0 * b.fraction_within_2,
                        b.max_cv_outside_edges,
                        b.object_ssim.unwrap_or(f64::NAN)
                    ))
                );
                eprintln!("{line}");
                let drift = COMPOSITE_DRIFT.iter().find(|d| d.0 == p.name);
                let bound = drift.map_or(1.0, |d| d.1);
                if max > bound || bar.as_ref().is_some_and(|b| !b.pass()) {
                    if bar.is_none()
                        && drift.is_none()
                        && webgl_unstable(&p.document)
                        && float_fraction_within_one(&ours, &reference) >= 0.999
                    {
                        unstable.push(line);
                    } else {
                        failed.push(line);
                    }
                }
            }
            let (d, bar) = match &three_d {
                None => (Diff::of(&ours, &reference), None),
                Some((object, edges, class)) => {
                    let rest: Vec<bool> = class.iter().map(|c| !c).collect();
                    let (w, h) = (s.width as usize, s.height as usize);
                    (
                        Diff::of_masked(&ours, &reference, &rest),
                        Some(bar_3d(&reference, &ours, edges, object, w, h)),
                    )
                }
            };
            compared += 1;
            let line = format!(
                "{:<40} t={:<6} {:>7} px  exact {:>6.2}%  ≤1 {:>7.3}%  max {}{}",
                p.name,
                s.time,
                d.pixels,
                100.0 * d.exact as f64 / d.pixels as f64,
                100.0 * d.fraction_within_one(),
                d.max,
                bar.as_ref().map_or(String::new(), |b| format!(
                    " (2D); 3D bar ≤2 {:.4}% max {} max outside edges {} SSIM {:.5}",
                    100.0 * b.fraction_within_2,
                    b.max_cv,
                    b.max_cv_outside_edges,
                    b.object_ssim.unwrap_or(f64::NAN)
                ))
            );
            eprintln!("{line}");
            let drift_max = COMPOSITE_DRIFT.iter().find(|x| x.0 == p.name).map(|x| x.2);
            let class = match drift_max {
                Some(m) => d.fraction_within_one() >= 0.999 && d.max <= m,
                None => d.passes(),
            };
            let known = KNOWN_RESIDUALS.iter().any(|&(name, pixels, max)| {
                p.name == name && bar.is_none() && d.pixels - d.within_one <= pixels && d.max <= max
            });
            if known && !class {
                eprintln!("  known residual: {}", p.name);
            } else if !class || bar.as_ref().is_some_and(|b| !b.pass()) {
                if bar.is_none()
                    && drift_max.is_none()
                    && webgl_unstable(&p.document)
                    && d.fraction_within_one() >= 0.999
                {
                    unstable.push(line);
                    continue;
                }
                failed.push(line);
                if let Some(dir) = &diffs {
                    std::fs::create_dir_all(dir).unwrap();
                    let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                    write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                    write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                }
            }
        }
    }
    eprintln!("\nskipped (unsupported by this renderer):");
    for (name, why) in &skipped {
        eprintln!("  {name:<40} {why}");
    }
    eprintln!(
        "\nWebGL-unstable class (amendment 9, judged by review): {} frames within 1 CV for 99.9% of pixels, over the maximum at isolated pixels",
        unstable.len()
    );
    for u in &unstable {
        eprintln!("  UNSTABLE {u}");
    }
    eprintln!("\ncompared {compared} frames; {} failed", failed.len());
    for f in &failed {
        eprintln!("  FAILED {f}");
    }
    assert!(compared > 0, "no supported corpus project was compared");
    // The still classes (docs/engine/pm-render.md, "Image classes"): the
    // ones that matched are drawn, the others routed. A render capture must
    // hold every `render/still-*` project, so the pin cannot pass vacuously.
    let render_capture = captures.iter().any(|c| {
        c.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with("render-"))
    });
    for (name, drawn) in STILL_CLASSES {
        if only.as_deref().is_some_and(|o| !name.starts_with(o)) {
            continue;
        }
        match seen.get(*name) {
            Some(supported) => assert_eq!(supported, drawn, "{name}: routing changed"),
            None => assert!(
                !render_capture,
                "{name}: not in the render capture (or its media is missing); recapture with \
                 `bun engine/corpus/src/capture.ts --only render/ --skip-real --skip-scenarios --label render`"
            ),
        }
    }
    assert!(
        failed.is_empty(),
        "{} frames outside the render class",
        failed.len()
    );
}
