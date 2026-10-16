//! Motion blur, 2.5D, and retiming (docs/engine/motion.md).
//!
//! - `motion_corpus_parity`: every project of the newest `motion-*`
//!   capture (`engine/corpus/src/projects/motion.ts`) against the legacy
//!   renderer at today's export counts (16 opaque, 20 transparent) and at
//!   every extra sample count captured (`blurSamples`), by section 25's
//!   classes: Standard on the opaque 8-bit frames, the alpha class (float
//!   frames within 1 CV), and the video class (patch interiors within 2
//!   CV) for projects with video.
//! - Synthetic tests: the accumulation against an exact CPU count of
//!   covered samples, one sample against the sharp frame, sample counts per
//!   output kind, regions, determinism, 2.5D coverage against the inverse
//!   homography, the camera plane, and the painter sort.
//!
//! `PM_MOTION_CORPUS` picks the capture (default: the newest `motion-*`);
//! `PM_RENDER_ONLY` limits to projects with a name prefix;
//! `PM_RENDER_DIFFS=<dir>` writes engine and reference PNGs of failures.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_protocol::frame::MotionBlurKind;
use pm_render::motion::MotionBlur;
use pm_render::{RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;
use support::*;

fn motion_capture() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("PM_MOTION_CORPUS") {
        return Some(p.into());
    }
    newest_capture("motion-")
}

fn has_video(p: &Project) -> bool {
    let root = p.document.root().unwrap();
    let Some(assets) = root.get("assets") else {
        return false;
    };
    let pm_model::NodeKind::Object(map) = assets.kind() else {
        return false;
    };
    map.iter().any(|(_, record)| {
        record.get("kind").is_some_and(
            |k| matches!(k.kind(), pm_model::NodeKind::String(s) if s.to_string_lossy() == "video"),
        )
    })
}

/// A transparent float frame against the capture: the alpha class, or the
/// video class on the reference's patch interiors.
fn compare_float(ours: &[f32], reference: &[f32], w: u32, h: u32, video: bool) -> (f64, String) {
    let _ = h;
    if video {
        let r8 = rgba8(reference);
        let mask = interior(&r8, w, h);
        let max = float_max_cv_masked(ours, reference, &mask);
        let n = mask.iter().filter(|m| **m).count();
        (max, format!("{n} interior px (video class)"))
    } else {
        let max = float_max_cv(ours, reference);
        // Where the largest difference is, for the report.
        let (i, _) = ours
            .iter()
            .zip(reference)
            .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
            .enumerate()
            .fold((0, 0.0), |m, (i, d)| if d > m.1 { (i, d) } else { m });
        let px = i / 4;
        let at = |v: &[f32]| [v[px * 4], v[px * 4 + 1], v[px * 4 + 2], v[px * 4 + 3]];
        (
            max,
            format!(
                "alpha class; at {},{} ours {:?} legacy {:?}",
                px as u32 % w,
                px as u32 / w,
                at(ours),
                at(reference)
            ),
        )
    }
}

#[test]
fn motion_corpus_parity() {
    let Some(capture) = motion_capture() else {
        eprintln!(
            "no motion capture; capture with `bun engine/corpus/src/capture.ts --only \
             motion/,space25d/,retime/ --label motion --skip-scenarios --skip-real`"
        );
        return;
    };
    let only = std::env::var("PM_RENDER_ONLY").ok();
    let diffs = std::env::var("PM_RENDER_DIFFS")
        .ok()
        .map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().expect("a Metal device");
    let mut compared = 0;
    let mut failed = Vec::new();
    let mut routed = Vec::new();
    for dir in projects(&capture) {
        let p = load_project(&capture, &dir);
        if only.as_deref().is_some_and(|o| !p.name.starts_with(o)) {
            continue;
        }
        assert!(
            p.missing_media.is_empty(),
            "{}: media not available {:?}",
            p.name,
            p.missing_media
        );
        // Every case is drawn (checked after the comparison, so a routing
        // regression still reports the pixels).
        let issues = pm_render::unsupported_with_media(&p.document, &p.composition, &p.resolver);
        if !issues.is_empty() {
            routed.push(format!("{}: {issues:?}", p.name));
        }
        let video = has_video(&p);
        for s in &p.samples {
            let size = Size::new(s.width, s.height);
            let precision = if std::env::var("PM_MOTION_F32").is_ok() {
                Precision::Float32
            } else {
                Precision::Float16
            };
            let mut request = RenderRequest::sized(size, precision);
            request.motion_blur = MotionBlur::Samples(16);
            let render = |renderer: &mut Renderer, request: &RenderRequest| {
                let frame = renderer
                    .render(
                        &p.document,
                        &p.composition,
                        ticks(s.time),
                        request,
                        &p.resolver,
                    )
                    .unwrap_or_else(|e| panic!("{} t={}: {e}", p.name, s.time));
                assert!(
                    frame.failures.is_empty(),
                    "{} t={}: {:?}",
                    p.name,
                    s.time,
                    frame.failures
                );
                renderer.read_rgba(&frame).unwrap()
            };
            // Opaque, 16 samples (`renderOpaqueFrame`).
            let ours = opaque8(&render(&mut renderer, &request));
            let (w, h, reference) = read_png(&s.png);
            compared += 1;
            let (pass, line) = if video {
                let mask = interior(&reference, w, h);
                let (max, n) = masked_max(&ours, &reference, &mask);
                (
                    max <= 2 && n > 0,
                    format!(
                        "{:<34} t={:<5} opaque 16  {n:>7} interior px (video class) max {max}",
                        p.name, s.time
                    ),
                )
            } else {
                let d = Diff::of(&ours, &reference);
                (
                    d.passes(),
                    format!(
                        "{:<34} t={:<5} opaque 16  exact {:>6.2}%  ≤1 {:>7.3}%  max {}",
                        p.name,
                        s.time,
                        100.0 * d.exact as f64 / d.pixels as f64,
                        100.0 * d.fraction_within_one(),
                        d.max
                    ),
                )
            };
            eprintln!("{line}");
            if !pass {
                failed.push(line);
                if let Some(dir) = &diffs {
                    std::fs::create_dir_all(dir).unwrap();
                    let stem = format!("{}-{}", p.name.replace('/', "_"), s.time);
                    write_png(&dir.join(format!("{stem}-engine.png")), w, h, &ours);
                    write_png(&dir.join(format!("{stem}-legacy.png")), w, h, &reference);
                }
            }
            // Transparent float frames: 20 samples, then every extra count.
            let mut floats: Vec<(u32, &std::path::PathBuf)> = Vec::new();
            if let Some(f) = &s.float {
                floats.push((20, f));
            }
            floats.extend(s.blur.iter().map(|(n, f)| (*n, f)));
            for (n, file) in floats {
                let mut request = request;
                request.background = false;
                request.motion_blur = MotionBlur::Samples(n);
                let ours = render(&mut renderer, &request);
                let reference = read_float(file);
                let (max, class) = compare_float(&ours, &reference, w, h, video);
                compared += 1;
                let limit = if video { 2.0 } else { 1.0 };
                let line = format!(
                    "{:<34} t={:<5} float {n:>3}  max {max:.4} CV ({class})",
                    p.name, s.time
                );
                eprintln!("{line}");
                if max > limit {
                    failed.push(line);
                    if let Some(dir) = &diffs {
                        std::fs::create_dir_all(dir).unwrap();
                        let stem = format!("{}-{}-mb{n}", p.name.replace('/', "_"), s.time);
                        write_png(&dir.join(format!("{stem}-engine.png")), w, h, &rgba8(&ours));
                        write_png(
                            &dir.join(format!("{stem}-legacy.png")),
                            w,
                            h,
                            &rgba8(&reference),
                        );
                    }
                }
            }
        }
    }
    eprintln!("\ncompared {compared} frames; {} failed", failed.len());
    for f in &failed {
        eprintln!("  FAILED {f}");
    }
    assert!(compared > 0, "no motion project was compared");
    assert!(
        failed.is_empty(),
        "{} frames outside their class",
        failed.len()
    );
    assert!(
        routed.is_empty(),
        "routed by the capability check: {routed:#?}"
    );
}

/// A white solid sliding right over a transparent background.
fn slider(blur: bool, samples_field: Option<serde_json::Value>) -> pm_model::Document {
    let mut l = layer(
        "s",
        "solid",
        json!({ "color": "#FFFFFF", "w": 20.0, "h": 20.0 }),
        0.0,
        10.0,
        4.0,
    );
    // 300 px per second: 10 px per frame at 30 fps.
    // Linear handles (the legacy `eo`/`ei` of a linear key).
    set(
        &mut l,
        "position.x",
        json!({ "v": 0.3, "expr": null, "kf": [
            { "t": 0.0, "v": 0.3, "i": "k0", "eo": [0.333, 0.333], "ei": [0.667, 0.667], "hold": false },
            { "t": 2.0, "v": 600.3, "i": "k1", "eo": [0.333, 0.333], "ei": [0.667, 0.667], "hold": false }
        ] }),
    );
    l["mblur"] = json!(blur);
    let mut doc = project(160, 40, 4.0, no_fill(), vec![l]);
    if let Some(f) = samples_field {
        doc["motionBlurSamples"] = f;
    }
    load(&doc)
}

fn render(
    renderer: &mut Renderer,
    doc: &pm_model::Document,
    seconds: f64,
    request: &RenderRequest,
) -> Vec<f32> {
    let f = renderer
        .render(
            doc,
            "Ptest-root",
            ticks(seconds),
            request,
            &MapResolverEmpty,
        )
        .unwrap();
    assert!(f.failures.is_empty(), "{:?}", f.failures);
    renderer.read_rgba(&f).unwrap()
}

struct MapResolverEmpty;

impl pm_render::AssetResolver for MapResolverEmpty {
    fn path(&self, _: &str, _: Option<&pm_model::Node>) -> Option<std::path::PathBuf> {
        None
    }
}

#[test]
fn accumulation_counts_covered_samples() {
    let mut renderer = Renderer::headless().unwrap();
    let doc = slider(true, None);
    let t = 0.2;
    // f16 accumulation rounds every partial sum (as today's RGBA16F
    // target does): at most half an ulp of a value ≤ 1 per sample.
    for (precision, per_sample) in [(Precision::Float16, 4.9e-4), (Precision::Float32, 6e-8)] {
        for n in [1u32, 4, 7, 16, 20] {
            let tolerance = per_sample * f64::from(n);
            let mut request = RenderRequest::sized(Size::new(160, 40), precision);
            request.motion_blur = MotionBlur::Samples(n);
            let rgba = render(&mut renderer, &doc, t, &request);
            for x in 0..160usize {
                // The rasterizer covers a pixel center when it lies in
                // [left, left + 20).
                let center = x as f64 + 0.5;
                let covered = (0..n)
                    .filter(|&s| {
                        let ts = t + pm_render::motion::sample_offset(s, n, 0.5, 30.0);
                        let left = 0.3 + 300.0 * ts;
                        left <= center && center < left + 20.0
                    })
                    .count();
                let want = covered as f64 / f64::from(n);
                let px = &rgba[(20 * 160 + x) * 4..(20 * 160 + x) * 4 + 4];
                for (c, v) in px.iter().enumerate() {
                    assert!(
                        (f64::from(*v) - want).abs() <= tolerance,
                        "{precision:?} n={n} x={x} channel {c}: {v} want {want}"
                    );
                }
            }
        }
    }
}

#[test]
fn one_sample_is_the_sharp_frame_and_off_draws_sharp() {
    let mut renderer = Renderer::headless().unwrap();
    let doc = slider(true, None);
    let mut request = RenderRequest::sized(Size::new(160, 40), Precision::Float16);
    let sharp = render(&mut renderer, &doc, 0.5, &request);
    request.motion_blur = MotionBlur::Samples(1);
    assert_eq!(render(&mut renderer, &doc, 0.5, &request), sharp);
    // The switch off: the same as no blur at any count.
    let off = slider(false, None);
    request.motion_blur = MotionBlur::Samples(12);
    assert_eq!(render(&mut renderer, &off, 0.5, &request), sharp);
    request.motion_blur = MotionBlur::Samples(12);
    assert_ne!(render(&mut renderer, &doc, 0.5, &request), sharp);
}

#[test]
fn sample_counts_follow_the_output_kind() {
    let mut renderer = Renderer::headless().unwrap();
    let doc = slider(true, Some(json!({ "export": 3, "previewPlaying": 2 })));
    let base = RenderRequest::sized(Size::new(160, 40), Precision::Float16);
    let with = |m: MotionBlur| {
        let mut r = base;
        r.motion_blur = m;
        r
    };
    let mut get = |m| render(&mut renderer, &doc, 0.4, &with(m));
    assert_eq!(
        get(MotionBlur::Kind(MotionBlurKind::Export)),
        get(MotionBlur::Samples(3))
    );
    assert_eq!(
        get(MotionBlur::Kind(MotionBlurKind::PreviewPlaying)),
        get(MotionBlur::Samples(2))
    );
    // Not set in the document: today's counts.
    assert_eq!(
        get(MotionBlur::Kind(MotionBlurKind::AlphaExport)),
        get(MotionBlur::Samples(20))
    );
    assert_eq!(
        get(MotionBlur::Kind(MotionBlurKind::PreviewPaused)),
        get(MotionBlur::Samples(12))
    );
    assert_eq!(
        get(MotionBlur::Kind(MotionBlurKind::Off)),
        get(MotionBlur::Off)
    );
    assert_ne!(get(MotionBlur::Samples(3)), get(MotionBlur::Samples(16)));
}

#[test]
fn blurred_frames_are_deterministic_and_regions_equal_the_frame() {
    let mut renderer = Renderer::headless().unwrap();
    // A group with blur, a rotating image-free mix, a 2.5D plane with blur.
    let mut a = layer(
        "a",
        "solid",
        json!({ "color": "#FF6B1A", "w": 50.0, "h": 30.0 }),
        30.0,
        40.0,
        4.0,
    );
    set(
        &mut a,
        "rotation",
        keys(json!(0), &[(0.0, json!(0)), (2.0, json!(720))]),
    );
    set(&mut a, "anchor.x", json!(25));
    set(&mut a, "anchor.y", json!(15));
    a["mblur"] = json!(true);
    let mut m = layer(
        "m",
        "solid",
        json!({ "color": "#4C8DFF", "w": 40.0, "h": 40.0 }),
        100.0,
        20.0,
        4.0,
    );
    m["group"] = json!("g");
    set(
        &mut m,
        "position.x",
        keys(json!(100), &[(0.0, json!(100)), (2.0, json!(200))]),
    );
    let mut g = layer("g", "group", json!({}), 0.0, 0.0, 4.0);
    g["mblur"] = json!(true);
    set(&mut g, "opacity", json!(70));
    let mut p = layer(
        "p",
        "solid",
        json!({ "color": "#3FCF8E", "w": 60.0, "h": 40.0 }),
        160.0,
        60.0,
        4.0,
    );
    p["threeD"] = json!(true);
    p["mblur"] = json!(true);
    set(
        &mut p,
        "rotation.y",
        keys(json!(0), &[(0.0, json!(0)), (2.0, json!(360))]),
    );
    let doc = load(&project(
        240,
        120,
        4.0,
        solid_fill("#101014"),
        vec![p, a, m, g],
    ));
    for precision in [Precision::Float16, Precision::Float32] {
        let mut request = RenderRequest::sized(Size::new(240, 120), precision);
        request.motion_blur = MotionBlur::Samples(9);
        let one = render(&mut renderer, &doc, 0.7, &request);
        let two = render(&mut renderer, &doc, 0.7, &request);
        assert_eq!(one, two, "{precision:?}: determinism");
        let region = pm_protocol::common::PixelRect {
            x: 37,
            y: 11,
            width: 150,
            height: 70,
        };
        let mut r = request;
        r.region = Some(region);
        let crop = render(&mut renderer, &doc, 0.7, &r);
        for y in 0..70usize {
            for x in 0..150usize {
                let a = &crop[(y * 150 + x) * 4..(y * 150 + x) * 4 + 4];
                let i = ((y + 11) * 240 + x + 37) * 4;
                assert_eq!(a, &one[i..i + 4], "{precision:?} region pixel {x},{y}");
            }
        }
    }
}

/// The inverse of a column-major plane: target pixel to unit quad.
fn unproject(p: &[f64; 9], x: f64, y: f64) -> Option<(f64, f64)> {
    let [a, d, g, b, e, h, c, f, i] = *p;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = [
        (e * i - f * h) / det,
        (f * g - d * i) / det,
        (d * h - e * g) / det,
        (c * h - b * i) / det,
        (a * i - c * g) / det,
        (b * g - a * h) / det,
        (b * f - c * e) / det,
        (c * d - a * f) / det,
        (a * e - b * d) / det,
    ];
    let w = inv[2] * x + inv[5] * y + inv[8];
    let u = (inv[0] * x + inv[3] * y + inv[6]) / w;
    let v = (inv[1] * x + inv[4] * y + inv[7]) / w;
    // In front of the camera: the forward w of (u, v) is positive.
    let fw = p[2] * u + p[5] * v + p[8];
    (fw > 0.0).then_some((u, v))
}

#[test]
fn planes_cover_what_their_homography_covers() {
    let mut renderer = Renderer::headless().unwrap();
    let mut l = layer(
        "p",
        "solid",
        json!({ "color": "#FFFFFF", "w": 120.0, "h": 80.0 }),
        100.0,
        60.0,
        4.0,
    );
    l["threeD"] = json!(true);
    set(&mut l, "anchor.x", json!(60));
    set(&mut l, "anchor.y", json!(40));
    set(&mut l, "rotation.y", json!(55));
    set(&mut l, "rotation.x", json!(-20));
    set(&mut l, "position.z", json!(-60));
    set(&mut l, "perspective", json!(30));
    let doc = load(&project(200, 120, 4.0, no_fill(), vec![l]));
    let request = RenderRequest::sized(Size::new(200, 120), Precision::Float32);
    let (plan, _) = renderer
        .plan(&doc, "Ptest-root", ticks(0.0), &request, &MapResolverEmpty)
        .unwrap();
    let pm_render::plan::Content::Placed(placed) = &plan.draws[0].content else {
        panic!("a 3D layer draws through its plane");
    };
    let plane = placed.plane.expect("a plane");
    let rgba = render(&mut renderer, &doc, 0.0, &request);
    let mut inside = 0;
    for y in 0..120usize {
        for x in 0..200usize {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let a = rgba[(y * 200 + x) * 4 + 3];
            match unproject(&plane, px, py) {
                Some((u, v)) => {
                    let margin = 1e-3;
                    if u > margin && u < 1.0 - margin && v > margin && v < 1.0 - margin {
                        assert_eq!(a, 1.0, "inside at {x},{y} (u {u} v {v})");
                        inside += 1;
                    } else if u < -margin || u > 1.0 + margin || v < -margin || v > 1.0 + margin {
                        assert_eq!(a, 0.0, "outside at {x},{y}");
                    }
                }
                None => assert_eq!(a, 0.0, "behind the camera at {x},{y}"),
            }
        }
    }
    assert!(inside > 2000, "{inside} covered pixels");
}

#[test]
fn planes_crossing_the_camera_plane_are_clipped() {
    let mut renderer = Renderer::headless().unwrap();
    // f = 200 × 50 / 36 ≈ 277.8: a plane turned 80° at z = −250 reaches
    // behind the camera plane on one side.
    let mut l = layer(
        "p",
        "solid",
        json!({ "color": "#FFFFFF", "w": 400.0, "h": 40.0 }),
        100.0,
        60.0,
        4.0,
    );
    l["threeD"] = json!(true);
    set(&mut l, "anchor.x", json!(200));
    set(&mut l, "anchor.y", json!(20));
    set(&mut l, "rotation.y", json!(80));
    set(&mut l, "position.z", json!(-250));
    let doc = load(&project(200, 120, 4.0, no_fill(), vec![l]));
    let request = RenderRequest::sized(Size::new(200, 120), Precision::Float32);
    let (plan, _) = renderer
        .plan(&doc, "Ptest-root", ticks(0.0), &request, &MapResolverEmpty)
        .unwrap();
    let pm_render::plan::Content::Placed(placed) = &plan.draws[0].content else {
        panic!("a plane");
    };
    let plane = placed.plane.unwrap();
    // One corner behind the camera, one in front.
    let w = |u: f64| plane[2] * u + plane[5] * 0.5 + plane[8];
    assert!(w(0.0).min(w(1.0)) < 0.0 && w(0.0).max(w(1.0)) > 0.0);
    let rgba = render(&mut renderer, &doc, 0.0, &request);
    let mut drawn = 0;
    for y in 0..120usize {
        for x in 0..200usize {
            let a = rgba[(y * 200 + x) * 4 + 3];
            assert!(a == 0.0 || a == 1.0);
            if a == 1.0 {
                drawn += 1;
                let (u, v) = unproject(&plane, x as f64 + 0.5, y as f64 + 0.5)
                    .expect("drawn pixels are in front of the camera");
                assert!((-1e-3..=1.0 + 1e-3).contains(&u) && (-1e-3..=1.0 + 1e-3).contains(&v));
            }
        }
    }
    assert!(drawn > 100, "{drawn}");
}

#[test]
fn painter_sort_and_coplanar_order() {
    let mut renderer = Renderer::headless().unwrap();
    let plane = |id: &str, color: &str, x: f64, z: f64| {
        let mut l = layer(
            id,
            "solid",
            json!({ "color": color, "w": 60.0, "h": 60.0 }),
            x,
            30.0,
            4.0,
        );
        l["threeD"] = json!(true);
        set(&mut l, "position.z", json!(z));
        l
    };
    // Timeline order: top first. `near` is lowest in the stack but nearest.
    let far = plane("far", "#FF0000", 60.0, 200.0);
    let near = plane("near", "#00FF00", 70.0, -100.0);
    // Two coplanar layers (z = 0): the upper one stays on top.
    let upper = plane("upper", "#0000FF", 20.0, 0.0);
    let lower = plane("lower", "#FFFF00", 30.0, 0.0);
    let doc = load(&project(
        200,
        120,
        4.0,
        no_fill(),
        vec![far, upper, lower, near],
    ));
    let request = RenderRequest::sized(Size::new(200, 120), Precision::Float32);
    let (plan, _) = renderer
        .plan(&doc, "Ptest-root", ticks(0.0), &request, &MapResolverEmpty)
        .unwrap();
    let order: Vec<&str> = plan.draws.iter().map(|d| d.layer.as_str()).collect();
    // Bottom to top: far, then the coplanar surface (lower, upper), then near.
    assert_eq!(order, ["far", "lower", "upper", "near"]);
}
