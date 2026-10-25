//! The render gate on the parity corpus (docs/engine/gate-policy.md): every
//! corpus project this renderer supports (by `unsupported_with_media`) is
//! rendered at every captured sample. Projects it does not support are
//! listed with the reason and skipped.
//!
//! Each frame is compared with the capture ("today") and with the
//! reference render of the same document (`support::reference`), per pixel
//! class (`pm_reference::compare`): interior pixels within 1 CV of today;
//! edge pixels at least as close to the reference as today, within the
//! frames' rounding; pixels the reference does not model yet by the
//! previous bound against today (section 25's Standard class for opaque
//! frames, the alpha class for transparent ones). Transparent frames the
//! reference models also keep their geometry (alpha area and centroid
//! within 1e-3), and the paused viewer's frame equals the export's.
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
//! `PM_RENDER_DIFFS=<dir>` writes engine and today's PNGs of failures there.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_reference::compare::{Bounds, compare, geometry, identical, same_geometry};
use pm_render::parity::{bar_3d, dilate, msaa_edges};
use pm_render::{RenderRequest, Renderer, all_builtin_definitions, unsupported_with_definitions};
use support::*;

/// 8-bit RGBA as 0…1.
fn unit8(v: &[u8]) -> Vec<f64> {
    v.iter().map(|c| f64::from(*c) / 255.0).collect()
}

fn unit32(v: &[f32]) -> Vec<f64> {
    v.iter().map(|c| f64::from(*c)).collect()
}

fn unmodelled(r: &reference::Reference) -> String {
    if r.unmodelled.is_empty() {
        String::new()
    } else {
        format!(" (no reference: {})", r.unmodelled.join(", "))
    }
}

/// `render/still-*` projects and whether the renderer draws them.
const STILL_CLASSES: &[(&str, bool)] = &[
    ("render/still-gray-alpha-png", true),
    ("render/still-indexed-png", true),
    ("render/still-photo-gray-jpg", true),
    ("render/still-photo-heic", true),
    ("render/still-photo-lossy-webp", true),
    ("render/still-photo-lossless-opaque-webp", true),
    ("render/still-photo-tiff", true),
    ("render/still-16bit-png", true),
    ("render/still-p3-heic", true),
    ("render/still-p3-jpg", true),
    ("render/still-p3-png", true),
    ("render/still-photo-420-jpg", true),
    ("render/still-photo-422-jpg", true),
    ("render/still-photo-422-tall-mcu-jpg", true),
    ("render/still-photo-444-jpg", true),
    ("render/still-photo-444-tall-mcu-jpg", true),
    ("render/still-photo-alpha-webp", true),
    ("render/still-photo-avif", true),
    ("render/still-photo-lossless-webp", true),
    ("render/still-photo-orientation-6-jpg", true),
    ("render/still-photo-progressive-jpg", true),
];

/// Video-class projects drawing video minified under a projection, where
/// patch interiors of today's frame average source pixels across chroma
/// edges. The class restates today's reading (`support::restate_today`)
/// except its chroma filter; these projects draw a rendition carrying
/// today's filter (`support::todays_chroma_rendition`), so the rest is held
/// to the class. The engine's own filter is proven against 4:4:4 truth by
/// tests/chroma_truth.rs.
const MINIFIED_VIDEO: &[&str] = &["space25d/precomps"];

/// A scratch directory removed when dropped.
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir_guard(name: &str) -> TempDir {
    let dir = std::env::temp_dir().join(format!(
        "pm-render-corpus-{}-{}",
        std::process::id(),
        name.replace('/', "_")
    ));
    std::fs::create_dir_all(&dir).unwrap();
    TempDir(dir)
}

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
    // Level 1 SVG images (decision S1) draw Chromium's rasters: those the
    // svg-rasters tool captured (PM_SVG_RASTERS, else the newest
    // `svg-rasters-*` directory); uncaptured ones fail their frame and are
    // listed in its needs.json. Without a directory resvg approximates them.
    renderer.set_svg_wait(true);
    let svg_rasters = std::env::var("PM_SVG_RASTERS")
        .ok()
        .map(std::path::PathBuf::from)
        .or_else(|| support::newest_capture("svg-rasters-"))
        .map(|d| std::sync::Arc::new(pm_render::vector::svg_rasters::RasterDir::new(d)));
    match &svg_rasters {
        Some(dir) => renderer.set_svg_source(Some(dir.clone())),
        None => {
            eprintln!(
                "no Chromium SVG rasters (PM_SVG_RASTERS): resvg approximates level 1 SVG images"
            );
            renderer.set_svg_rasterizer(pm_render::vector::svg_rasters::SvgRasterizer::Resvg);
        }
    }
    // The built-in effects, transitions, and extension layers as the kernel
    // snapshots them (the captures load only built-in and missing
    // definitions).
    renderer
        .set_definitions(&all_builtin_definitions())
        .expect("built-in definitions");
    let mut compared = 0;
    // Opaque frames the reference models in full, and in part.
    let (mut complete, mut partial) = (0, 0);
    let mut failed = Vec::new();
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
        // The capture app had only the built-in definitions: any other
        // extension layer definition was missing there (the checkerboard).
        let mut set = all_builtin_definitions();
        set.missing = support::extension_definitions(&p.document)
            .into_iter()
            .filter(|id| !set.definitions.iter().any(|d| d.id.as_str() == id))
            .map(|id| id.as_str().into())
            .collect();
        renderer
            .set_definitions(&set)
            .expect("built-in definitions");
        let issues = unsupported_with_definitions(
            &p.document,
            &p.composition,
            Some(&p.resolver),
            renderer.effects(),
        );
        seen.insert(p.name.clone(), issues.is_empty());
        // Amendment 21: these classes are drawn natively; routing one of
        // them is a failure, not a skip.
        let forbidden: Vec<String> = issues
            .iter()
            .filter(|u| support::routing_forbidden(&u.feature))
            .map(|u| u.to_string())
            .collect();
        if !forbidden.is_empty() {
            failed.push(format!("{}: routed for {}", p.name, forbidden.join(", ")));
        }
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
        // Geometry is measured at Float32 for Float16 documents: f16 alpha
        // stores change area-weighted centroids without moving authored
        // shapes. The ordinary Float16 pixel and preview/export gates below
        // still run against the original document and request.
        let geometry_document = (pm_render::composite::color::project_color(root)
            .expect("project color")
            .precision
            == Precision::Float16)
            .then(|| {
                let mut value = p.document.to_value().expect("project root");
                value
                    .as_object_mut()
                    .expect("project object")
                    .insert("colorDepth", pm_model::Value::Number(32.0));
                pm_model::Document::new(Some(value), p.document.revision() + 1)
            });
        if let Some(assets) = root.get("assets")
            && let pm_model::NodeKind::Object(map) = assets.kind()
        {
            {
                for (id, record) in map.iter() {
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
                    let read = renderer.video_color(path).expect("video color");
                    let mut c = read;
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
                        _ => {}
                    }
                    // Today's readings the engine corrects (SD primaries,
                    // partly tagged SD H.264), restated so this gate keeps
                    // judging everything else.
                    support::restate_today(path, &mut c);
                    if c != read {
                        renderer.interpret(path, Some(c));
                    }
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
        let renditions = tempdir_guard(&p.name);
        let mut resolver = pm_render::MapResolver {
            paths: p.resolver.paths.clone(),
        };
        if video {
            // Media reached without an asset record (generated corpus clips).
            support::restate_today_for_videos(&mut renderer, &p.resolver);
            if MINIFIED_VIDEO.contains(&p.name.as_str()) {
                for path in resolver.paths.values_mut() {
                    let Ok(read) = renderer.video_color(path) else {
                        continue;
                    };
                    let rendition = support::todays_chroma_rendition(path, &renditions.0);
                    let mut c = renderer.video_color(&rendition).unwrap();
                    (c.primaries, c.matrix, c.range) = (read.primaries, read.matrix, read.range);
                    renderer.interpret(&rendition, Some(c));
                    *path = rendition;
                }
            }
        }
        for s in &p.samples {
            // The capture's motion blur: 16 samples opaque, 20 transparent.
            let mut request =
                RenderRequest::sized(Size::new(s.width, s.height), Precision::Float16);
            // Use the export's canvas policy; source timing is shared with
            // the viewer and holds the last source frame without padding.
            request.output_kind = pm_render::OutputKind::Final;
            request.motion_blur = pm_render::motion::MotionBlur::Samples(16);
            let frame = match renderer.render(
                &p.document,
                &p.composition,
                ticks(s.time),
                &request,
                &resolver,
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
            // Preview == export (gate-policy.md).
            let (preview, export) = reference::preview_and_export(
                &mut renderer,
                &p.document,
                &p.composition,
                ticks(s.time),
                request.output_size,
                request.motion_blur,
                // The gated frame's media (a minified video's rendition).
                &resolver,
            );
            // The gated frame is the export's.
            if let Err(e) = identical(&export, &rgba) {
                failed.push(format!(
                    "{} t={}: gated frame vs export: {e}",
                    p.name, s.time
                ));
            }
            if let Err(e) = identical(&preview, &export) {
                failed.push(format!("{} t={}: {e}", p.name, s.time));
            }
            let ours = opaque8(&rgba);
            let (w, h, reference) =
                support::reference_substitutions::opaque_reference(&capture, &p.name, s)
                    .unwrap_or_else(|error| panic!("{} t={}: {error}", p.name, s.time));
            assert_eq!((w, h), (s.width, s.height));
            // Keep reviewable native/reference evidence even when an explicit
            // owner-approved substitution passes its ordinary comparison.
            if s.reference_substitution.is_some()
                && let Some(dir) = &diffs
            {
                std::fs::create_dir_all(dir).unwrap();
                let stem = format!("{}-{}-COMPCLOSE-1", p.name.replace('/', "_"), s.time);
                write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                write_png(&dir.join(format!("{stem}-reference.png")), w, h, &reference);
            }
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
                let today = read_float(float);
                match &three_d {
                    // The 3D class (section 25) until the 3D lane converts it.
                    Some((object, edges, class)) => {
                        let rest: Vec<bool> = class.iter().map(|c| !c).collect();
                        let (w, h) = (s.width as usize, s.height as usize);
                        let b = bar_3d(&rgba8(&today), &rgba8(&ours), edges, object, w, h);
                        let max = float_max_cv_masked(&ours, &today, &rest);
                        let line = format!(
                            "{:<40} t={:<6} transparent float  max {max:.4} CV outside 3D; 3D bar ≤2 {:.4}% max outside edges {} SSIM {:.5}",
                            p.name,
                            s.time,
                            100.0 * b.fraction_within_2,
                            b.max_cv_outside_edges,
                            b.object_ssim.unwrap_or(f64::NAN)
                        );
                        eprintln!("{line}");
                        if max > 1.0 || !b.pass() {
                            failed.push(line);
                        }
                    }
                    None => {
                        let r = reference::render(
                            &p.document,
                            &p.resolver,
                            &p.composition,
                            ticks(s.time),
                            s.width,
                            s.height,
                            false,
                        );
                        let (engine, today) = (unit32(&ours), unit32(&today));
                        let report = compare(&engine, &today, &r.rgba, &r.classes());
                        let mut problems = report.failures(&Bounds::HALF_FLOAT);
                        let mut shape = String::new();
                        if r.modelled() == 1.0 {
                            let alpha =
                                |f: &[f64]| f.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
                            let wanted = geometry(&r.alpha(), r.width);
                            let ours = geometry(&alpha(&engine), r.width);
                            let theirs = geometry(&alpha(&today), r.width);
                            shape = format!(
                                " geometry: f64 area {:.3} centroid ({:.6}, {:.6}); engine area {:.3} centroid ({:.6}, {:.6}), delta area {:.3} centroid {:.6} px; today area {:.3} centroid ({:.6}, {:.6})",
                                wanted.area,
                                wanted.centroid[0],
                                wanted.centroid[1],
                                ours.area,
                                ours.centroid[0],
                                ours.centroid[1],
                                ours.area - wanted.area,
                                (ours.centroid[0] - wanted.centroid[0])
                                    .abs()
                                    .max((ours.centroid[1] - wanted.centroid[1]).abs()),
                                theirs.area,
                                theirs.centroid[0],
                                theirs.centroid[1]
                            );
                            if let Some(geometry_document) = &geometry_document {
                                let mut probe = request.clone();
                                probe.precision = Precision::Float32;
                                let frame = renderer
                                    .render(
                                        geometry_document,
                                        &p.composition,
                                        ticks(s.time),
                                        &probe,
                                        &resolver,
                                    )
                                    .expect("Float32 geometry probe render");
                                if !frame.failures.is_empty() {
                                    problems.push(format!(
                                        "Float32 geometry probe layer failures {:?}",
                                        frame.failures
                                    ));
                                }
                                let pixels = renderer.read_rgba(&frame).unwrap();
                                let measured = geometry(&alpha(&unit32(&pixels)), r.width);
                                shape.push_str(&format!(
                                    "; Float32 probe area {:.3} centroid ({:.6}, {:.6})",
                                    measured.area, measured.centroid[0], measured.centroid[1]
                                ));
                                if let Err(e) = same_geometry(&measured, &wanted, 1e-3) {
                                    problems.push(format!("Float32 geometry probe {e}"));
                                }
                            } else if let Err(e) = same_geometry(&ours, &wanted, 1e-3) {
                                problems.push(format!("engine {e}"));
                            }
                        }
                        let line = format!(
                            "{:<40} t={:<6} transparent float{shape}{}{report}",
                            p.name,
                            s.time,
                            unmodelled(&r)
                        );
                        eprintln!("{line}");
                        if !problems.is_empty() {
                            failed.push(format!("{line}\n      {}", problems.join("\n      ")));
                        }
                    }
                }
            }
            compared += 1;
            let (line, pass) = match &three_d {
                // The 3D class (section 25) until the 3D lane converts it.
                Some((object, edges, class)) => {
                    let rest: Vec<bool> = class.iter().map(|c| !c).collect();
                    let (w, h) = (s.width as usize, s.height as usize);
                    let d = Diff::of_masked(&ours, &reference, &rest);
                    let b = bar_3d(&reference, &ours, edges, object, w, h);
                    let line = format!(
                        "{:<40} t={:<6} {:>7} px  ≤1 {:>7.3}%  max {} (2D); 3D bar ≤2 {:.4}% max {} max outside edges {} SSIM {:.5}",
                        p.name,
                        s.time,
                        d.pixels,
                        100.0 * d.fraction_within_one(),
                        d.max,
                        100.0 * b.fraction_within_2,
                        b.max_cv,
                        b.max_cv_outside_edges,
                        b.object_ssim.unwrap_or(f64::NAN)
                    );
                    (line, d.passes() && b.pass())
                }
                None => {
                    let r = reference::render(
                        &p.document,
                        &p.resolver,
                        &p.composition,
                        ticks(s.time),
                        s.width,
                        s.height,
                        true,
                    );
                    let modelled = r.modelled();
                    complete += usize::from(modelled == 1.0);
                    partial += usize::from(modelled > 0.0 && modelled < 1.0);
                    let report =
                        compare(&unit8(&ours), &unit8(&reference), &r.opaque(), &r.classes());
                    let problems = report.failures(&Bounds::EIGHT_BIT);
                    let line = format!("{:<40} t={:<6}{}{report}", p.name, s.time, unmodelled(&r));
                    let pass = problems.is_empty();
                    let line = if pass {
                        line
                    } else {
                        format!("{line}\n      {}", problems.join("\n      "))
                    };
                    (line, pass)
                }
            };
            eprintln!("{line}");
            if !pass {
                failed.push(line);
                // Side by side: the engine's frame, today's, and the pixels
                // more than 1 CV apart in red over a dimmed copy of today's.
                if let Some(dir) = &diffs {
                    std::fs::create_dir_all(dir).unwrap();
                    let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                    write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                    write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                    let marked: Vec<u8> = ours
                        .chunks_exact(4)
                        .zip(reference.chunks_exact(4))
                        .flat_map(|(a, b)| {
                            if a.iter().zip(b).any(|(x, y)| x.abs_diff(*y) > 1) {
                                [255, 0, 0, 255]
                            } else {
                                [b[0] / 3, b[1] / 3, b[2] / 3, 255]
                            }
                        })
                        .collect();
                    write_png(&dir.join(format!("{stem}-diff.png")), w, h, &marked);
                }
            }
        }
    }
    if let Some(dir) = &svg_rasters
        && !dir.missing().is_empty()
    {
        let listed = dir.write_needs().expect("needs.json");
        eprintln!(
            "\n{} Chromium SVG rasters were not captured ({listed} listed in needs.json; run engine/crates/pm-render/tools/svg-rasters/run.ts --needs <dir>)",
            dir.missing().len()
        );
    }
    eprintln!("\nskipped (unsupported by this renderer):");
    for (name, why) in &skipped {
        eprintln!("  {name:<40} {why}");
    }
    eprintln!(
        "\ncompared {compared} frames (reference: {complete} opaque frames in full, {partial} in part); {} failed",
        failed.len()
    );
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
