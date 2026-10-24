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

/// Frames whose paused-viewer render differs from the export's for a
/// known, owned reason. Each must still differ: an entry that starts
/// passing fails until it is removed, and a full run fails for an entry it
/// never measured.
///
/// M-R1 (lane L4): the viewer keeps video 0.04 s before a clip's end
/// (`VideoEnd::Viewer`), exports 1 / fps (`VideoEnd::Final`).
const PREVIEW_EXPORT_PENDING: &[(&str, f64)] = &[("retime/speed", 2.9)];

#[test]
fn corpus_render_parity() {
    let captures = corpus_dirs();
    if captures.is_empty() {
        eprintln!("no parity corpus capture found; set PM_ENGINE_CORPUS to run this comparison");
        return;
    }
    let only = std::env::var("PM_RENDER_ONLY").ok();
    let narrowed = only.is_some() || std::env::var_os("PM_ENGINE_CORPUS").is_some();
    let mut pending_seen = Vec::new();
    let diffs = std::env::var("PM_RENDER_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
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
            // capture.ts runs prepareFrame before renderToPixels: final
            // output keeps the last source frame with 1 / composition fps
            // padding, unlike the viewer's default 0.04 seconds.
            request.video_end = pm_render::VideoEnd::Final;
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
            // Preview == export (gate-policy.md).
            let (preview, export) = reference::preview_and_export(
                &mut renderer,
                &p.document,
                &p.composition,
                ticks(s.time),
                request.output_size,
                request.motion_blur,
                &p.resolver,
            );
            let pending = PREVIEW_EXPORT_PENDING.contains(&(p.name.as_str(), s.time));
            if pending {
                pending_seen.push((p.name.clone(), s.time));
            }
            // The gated frame is the export's.
            if let Err(e) = identical(&export, &rgba) {
                failed.push(format!("{} t={}: gated frame vs export: {e}", p.name, s.time));
            }
            match (identical(&preview, &export), pending) {
                (Err(e), false) => failed.push(format!("{} t={}: {e}", p.name, s.time)),
                (Err(e), true) => eprintln!("{} t={}: pending M-R1: {e}", p.name, s.time),
                (Ok(()), true) => failed.push(format!(
                    "{} t={}: preview equals export; remove it from PREVIEW_EXPORT_PENDING",
                    p.name, s.time
                )),
                (Ok(()), false) => {}
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
                            let alpha = |f: &[f64]| f.chunks_exact(4).map(|p| p[3]).collect::<Vec<_>>();
                            let wanted = geometry(&r.alpha(), r.width);
                            let ours = geometry(&alpha(&engine), r.width);
                            let theirs = geometry(&alpha(&today), r.width);
                            shape = format!(
                                " geometry: area {:.3} engine {:.3} today {:.3}",
                                wanted.area, ours.area, theirs.area
                            );
                            if let Err(e) = same_geometry(&ours, &wanted, 1e-3) {
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
                        &p.composition,
                        ticks(s.time),
                        s.width,
                        s.height,
                        true,
                    );
                    let modelled = r.modelled();
                    complete += usize::from(modelled == 1.0);
                    partial += usize::from(modelled > 0.0 && modelled < 1.0);
                    let report = compare(&unit8(&ours), &unit8(&reference), &r.opaque(), &r.classes());
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
    if !narrowed {
        for (name, time) in PREVIEW_EXPORT_PENDING {
            if !pending_seen.iter().any(|(n, t)| n == name && t == time) {
                failed.push(format!(
                    "{name} t={time}: PREVIEW_EXPORT_PENDING entry not measured"
                ));
            }
        }
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
