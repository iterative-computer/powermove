//! Persisted WebM sequence compatibility, timing and named media errors.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{Feature, MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;

#[test]
fn persisted_sequence_renditions_use_the_video_path() {
    let path = support::repo_root().join("apps/desktop/e2e/fixtures/vp9-opus.webm");
    let mut doc = project(
        160,
        90,
        1.0,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({"asset":"clip","w":160,"h":90,"fit":"stretch"}),
            80.0,
            45.0,
            1.0,
        )],
    );
    asset(&mut doc, "clip", "video", "webm", 160, 90, 1.0);
    doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":10});
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path);
    for format in ["gif", "png", "apng", "webp", "avif", "webm"] {
        doc["assets"]["clip"]["format"] = json!(format);
        assert!(
            pm_render::unsupported_with_media(&load(&doc), "Ptest-root", &resolver)
                .iter()
                .all(|u| u.feature != Feature::ImageSequence)
        );
    }
}

#[test]
fn sequence_timing_keeps_the_last_slot_and_the_shipped_boundary_epsilon() {
    for fps in [12.0, 120.0, 23.976, 240.0] {
        let record = pm_model::Node::from_value(
            pm_model::json::parse(&json!({"imageSequence":{"fps":fps,"frames":3}}).to_string())
                .unwrap(),
            0,
        );
        for (time, index) in [
            (-1.0, 0),
            (0.0, 0),
            (1.0 / fps - 5e-8, 1),
            (2.0 / fps, 2),
            (3.0 / fps, 2),
        ] {
            assert_eq!(
                pm_render::media::sequence_playback_time(&record, time),
                Some(index as f64 / fps + 0.001)
            );
        }
    }
}

#[test]
fn offline_and_corrupt_media_are_named_failures_without_routing() {
    let mut renderer = Renderer::headless().unwrap();
    let mut doc = project(16, 16, 1.0, no_fill(), vec![]);
    let request = RenderRequest::at_scale(Size::new(16, 16), 1.0, Precision::Float16);
    let dir = std::env::temp_dir().join(format!("pm-render-media-errors-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bytes");
    for (kind, format, sequence) in [
        ("image", "png", false),
        ("image", "svg", false),
        ("video", "webm", false),
        ("video", "gif", true),
    ] {
        doc["layers"] = json!([layer(
            "source",
            kind,
            json!({"asset":"clip", "w":16,"h":16}),
            8.0,
            8.0,
            1.0
        )]);
        asset(&mut doc, "clip", kind, format, 16, 16, 1.0);
        if sequence {
            doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":3});
        }
        let mut resolver = MapResolver::default();
        resolver.paths.insert("clip".into(), path.clone());
        for corrupt in [0, 1, 2] {
            if corrupt != 0 {
                let bytes: &[u8] = if corrupt == 1 {
                    b"not media"
                } else {
                    b"\x1a\x45\xdf\xa3broken WebM"
                };
                std::fs::write(&path, bytes).unwrap();
            }
            let document = load(&doc);
            let unsupported = renderer.unsupported_with_media(&document, "Ptest-root", &resolver);
            assert!(
                unsupported.is_empty(),
                "{kind} corrupt={corrupt}: {unsupported:?}"
            );
            let frame = renderer
                .render(
                    &document,
                    "Ptest-root",
                    support::ticks(0.0),
                    &request,
                    &resolver,
                )
                .unwrap();
            assert_eq!(frame.failures.len(), 1, "{kind}: {:?}", frame.failures);
            assert_eq!(frame.failures[0].layer, "source");
            assert!(!frame.failures[0].message.is_empty());
        }
        std::fs::remove_file(&path).unwrap();
    }
    std::fs::remove_dir(&dir).unwrap();
}

/// Video class's independent reference: FFmpeg decode + zimg, using the
/// native 4:2:0 filter (linear vertical, Catmull-Rom across). Level 1's
/// libyuv bitmap is compared to Chromium by the corpus/app tests below;
/// its separate zimg quantization gap is recorded in media.md.
/// This checks opaque native pixels and all alpha against that reference;
/// Chromium's different chroma reconstruction is judged separately in e2e.
#[test]
fn level_two_animation_frames_match_ffmpeg_zimg() {
    use std::process::Command;
    let bin = std::path::PathBuf::from(std::env::var("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    let dir = std::env::temp_dir().join(format!("pm-animation-zimg-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut rows = Vec::new();
    let mut renderer = Renderer::headless().unwrap();
    for name in ["animated.gif", "animated-apng.png", "animated.webp"] {
        let path = persisted_rendition(&dir, name);
        let mut value = project(
            160,
            90,
            1.0,
            no_fill(),
            vec![layer(
                "v",
                "video",
                json!({"asset":"clip","w":160,"h":90}),
                80.0,
                45.0,
                1.0,
            )],
        );
        asset(
            &mut value,
            "clip",
            "video",
            name.rsplit('.').next().unwrap(),
            160,
            90,
            0.3,
        );
        value["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":3});
        value["layers"][0]["compat"] = json!(2);
        let mut resolver = MapResolver::default();
        resolver.paths.insert("clip".into(), path.clone());
        let document = load(&value);
        assert!(
            renderer
                .unsupported_with_media(&document, "Ptest-root", &resolver)
                .is_empty()
        );
        let mut request = RenderRequest::at_scale(Size::new(160, 90), 1.0, Precision::Float32);
        request.background = false;
        for n in [2, 0, 1] {
            let frame = renderer
                .render(
                    &document,
                    "Ptest-root",
                    support::ticks(f64::from(n) / 10.0),
                    &request,
                    &resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let native = renderer.read_rgba(&frame).unwrap();
            // Keep alpha separate: zimg has no yuva422p16le input. Using
            // it silently inserts a swscale chroma resampling pass.
            let vf = format!(
                "select=eq(n\\,{n}),format=yuv420p,zscale=matrixin=170m:matrix=170m:rangein=limited:range=limited:chromalin=left:chromal=left:filter=bilinear:dither=none,format=yuv422p16le,zscale=matrixin=170m:matrix=gbr:rangein=limited:range=full:chromalin=left:chromal=left:filter=bicubic:param_a=0:param_b=0.5:dither=none,format=gbrpf32le"
            );
            let out = Command::new(&bin)
                .args(["-v", "error", "-c:v", "libvpx-vp9", "-i"])
                .arg(&path)
                .args(["-vf", &vf, "-frames:v", "1", "-f", "rawvideo", "-"])
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
            let planar: Vec<f32> = out
                .stdout
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            let pixels = 160 * 90;
            assert_eq!(planar.len(), pixels * 3);
            let alpha_out = Command::new(&bin)
                .args(["-v", "error", "-c:v", "libvpx-vp9", "-i"])
                .arg(&path)
                .args([
                    "-vf",
                    &format!("select=eq(n\\,{n}),alphaextract"),
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "gray",
                    "-f",
                    "rawvideo",
                    "-",
                ])
                .output()
                .unwrap();
            assert!(
                alpha_out.status.success(),
                "{}",
                String::from_utf8_lossy(&alpha_out.stderr)
            );
            assert_eq!(alpha_out.stdout.len(), pixels);
            let mut max = 0.0f64;
            let mut alpha = 0.0f64;
            let mut peak = String::new();
            for i in 0..pixels {
                let a = f32::from(alpha_out.stdout[i]) / 255.0;
                alpha = alpha.max(f64::from((native[i * 4 + 3] - a).abs()) * 255.0);
                // Transparent RGB is undefined. Opaque areas compare color
                // without masking any chroma edge or frame border.
                if a == 1.0 {
                    for (c, plane) in [2, 0, 1].into_iter().enumerate() {
                        let reference = planar[plane * pixels + i];
                        let d = f64::from((native[i * 4 + c] - reference).abs()) * 255.0;
                        if d > max {
                            max = d;
                            peak = format!(
                                "at {},{} c{} native{} ref{}",
                                i % 160,
                                i / 160,
                                c,
                                native[i * 4 + c],
                                planar[plane * pixels + i]
                            );
                        }
                    }
                }
            }
            eprintln!("{name} frame{n}: FFmpeg+zimg max {max:.6} CV alpha {alpha:.6} {peak}");
            rows.push(json!({"source":name,"frame":n,"max_cv":max,"alpha_cv":alpha}));
            if let Some(out) = std::env::var_os("PM_MEDIA_EVIDENCE") {
                let out = std::path::PathBuf::from(out);
                let mut pair = vec![0u8; pixels * 8];
                for i in 0..pixels {
                    let a = f32::from(alpha_out.stdout[i]) / 255.0;
                    for c in 0..3 {
                        pair[((i / 160) * 320 + i % 160) * 4 + c] =
                            (planar[[2, 0, 1][c] * pixels + i].clamp(0.0, 1.0) * a * 255.0).round()
                                as u8;
                        pair[((i / 160) * 320 + 160 + i % 160) * 4 + c] =
                            (native[i * 4 + c].clamp(0.0, 1.0) * 255.0).round() as u8;
                    }
                    pair[((i / 160) * 320 + i % 160) * 4 + 3] = 255;
                    pair[((i / 160) * 320 + 160 + i % 160) * 4 + 3] = 255;
                }
                support::write_png(
                    &out.join(format!("zimg-{name}-{n}-reference-left-native-right.png")),
                    320,
                    90,
                    &pair,
                );
            }
        }
    }
    if let Some(out) = std::env::var_os("PM_MEDIA_EVIDENCE") {
        std::fs::write(
            std::path::PathBuf::from(out).join("zimg-metrics.json"),
            serde_json::to_string_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    std::fs::remove_dir_all(dir).unwrap();
    for row in rows {
        assert!(
            row["max_cv"].as_f64().unwrap() <= 1.0 && row["alpha_cv"].as_f64().unwrap() <= 1.0,
            "{row}"
        );
    }
}

/// Prefer actual stored bytes from the hidden-app capture; otherwise use
/// the current converter's exact settings with the same fixture.
fn persisted_rendition(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    use std::process::Command;
    let bin = std::path::PathBuf::from(std::env::var("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    let evidence = std::env::var_os("PM_MEDIA_RENDITIONS").map(std::path::PathBuf::from);
    if let Some(e) = &evidence {
        e.join(name).join("native-source.webm")
    } else {
        let path = dir.join(format!("{name}.webm"));
        let input = support::repo_root()
            .join("apps/desktop/e2e/fixtures")
            .join(name);
        let out = Command::new(&bin)
            .args(["-v", "error", "-y", "-i"])
            .arg(input)
            .args([
                "-an",
                "-c:v",
                "libvpx-vp9",
                "-pix_fmt",
                "yuva420p",
                "-b:v",
                "0",
                "-crf",
                "18",
                "-g",
                "1",
                "-deadline",
                "realtime",
                "-cpu-used",
                "6",
                "-row-mt",
                "1",
                "-threads",
                "4",
                "-f",
                "webm",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        path
    }
}

/// The capture's import hash identifies the original, but its loaded document
/// and Chromium frames use the persisted WebM. Resolve that asset to the
/// stored rendition without changing the composition, transforms or times.
#[test]
fn animated_corpus_projects_use_stored_renditions_within_the_video_class() {
    let Some(capture) = support::newest_capture("v1-") else {
        return;
    };
    let dir = std::env::temp_dir().join(format!("pm-animation-corpus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let evidence = std::env::var_os("PM_MEDIA_EVIDENCE").map(std::path::PathBuf::from);
    let mut renderer = Renderer::headless().unwrap();
    let mut rows = Vec::new();
    for (project, source) in [
        ("animated-gif", "animated.gif"),
        ("animated-apng-png", "animated-apng.png"),
        ("animated-webp", "animated.webp"),
    ] {
        let mut p = support::load_project(&capture, &capture.join("projects/media").join(project));
        assert!(p.missing_media.is_empty());
        assert_eq!(p.resolver.paths.len(), 1);
        *p.resolver.paths.values_mut().next().unwrap() = persisted_rendition(&dir, source);
        assert!(
            renderer
                .unsupported_with_media(&p.document, &p.composition, &p.resolver)
                .is_empty()
        );
        let value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(p.dir.join("loaded.json")).unwrap())
                .unwrap();
        let document = load(&value);
        for sample in &p.samples {
            let mut request = RenderRequest::at_scale(
                Size::new(sample.width, sample.height),
                1.0,
                Precision::Float32,
            );
            let frame = renderer
                .render(
                    &document,
                    &p.composition,
                    support::ticks(sample.time),
                    &request,
                    &p.resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let rgba = support::opaque8(&renderer.read_rgba(&frame).unwrap());
            let (w, h, reference) = support::read_png(&sample.png);
            let (max, pixels) =
                support::masked_max(&rgba, &reference, &support::interior(&reference, w, h));
            request.background = false;
            let frame = renderer
                .render(
                    &document,
                    &p.composition,
                    support::ticks(sample.time),
                    &request,
                    &p.resolver,
                )
                .unwrap();
            let transparent = renderer.read_rgba(&frame).unwrap();
            let reference_alpha =
                support::read_float(sample.float.as_ref().expect("captured alpha frame"));
            let alpha = transparent
                .chunks_exact(4)
                .zip(reference_alpha.chunks_exact(4))
                .map(|(a, b)| f64::from((a[3] - b[3]).abs()) * 255.0)
                .fold(0.0f64, f64::max);
            rows.push(json!({"project":p.name,"time":sample.time,"max_cv":max,"interior_pixels":pixels,"alpha_cv":alpha}));
            if let Some(out) = &evidence {
                let stem = format!("{project}-{}", sample.time);
                support::write_png(&out.join(format!("{stem}-native.png")), w, h, &rgba);
                let mut pair = vec![0u8; w as usize * h as usize * 8];
                for y in 0..h as usize {
                    for (which, bytes) in [&reference, &rgba].iter().enumerate() {
                        let start = (y * w as usize * 2 + which * w as usize) * 4;
                        pair[start..start + w as usize * 4]
                            .copy_from_slice(&bytes[y * w as usize * 4..(y + 1) * w as usize * 4]);
                    }
                }
                support::write_png(
                    &out.join(format!("{stem}-legacy-left-native-right.png")),
                    w * 2,
                    h,
                    &pair,
                );
            }
            eprintln!(
                "{} t{}: max {max} CV, {pixels} interior pixels, alpha {alpha}",
                p.name, sample.time
            );
        }
    }
    assert_eq!(rows.len(), 9);
    if let Some(out) = evidence {
        std::fs::write(
            out.join("corpus-animation-metrics.json"),
            serde_json::to_string_pretty(&rows).unwrap(),
        )
        .unwrap();
    }
    for row in rows {
        assert!(
            row["max_cv"].as_u64().unwrap() <= 2
                && row["interior_pixels"].as_u64().unwrap() > 0
                && row["alpha_cv"].as_f64().unwrap() <= 1.0,
            "{row}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
