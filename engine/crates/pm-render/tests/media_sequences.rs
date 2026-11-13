//! Persisted WebM sequence compatibility, timing and named media errors.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{Feature, MapResolver, RenderRequest, Renderer};
use serde_json::json;
use support::doc::*;

#[test]
fn persisted_sequence_renditions_use_the_video_path() {
    let path = support::repo_root().join("engine/fixtures/vp9-opus.webm");
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
                pm_render::media::sequence_playback_time(&record, time, None),
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
    // (kind, format, sequence, compat level)
    for (kind, format, sequence, level) in [
        ("image", "png", false, 1),
        ("image", "svg", false, 1),
        ("image", "svg", false, 2),
        ("video", "webm", false, 1),
        ("video", "gif", true, 1),
    ] {
        let mut source = layer(
            "source",
            kind,
            json!({"asset":"clip", "w":16,"h":16}),
            8.0,
            8.0,
            1.0,
        );
        source["compat"] = json!(level);
        doc["layers"] = json!([source]);
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
                "{kind} level {level} corrupt={corrupt}: {unsupported:?}"
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
            if format == "svg" && corrupt != 0 {
                // Chromium's broken <img> paints nothing. Valid SVG paint
                // records remain gated independently of this XML check.
                assert!(frame.failures.is_empty());
                assert!(
                    renderer
                        .read_rgba(&frame)
                        .unwrap()
                        .iter()
                        .all(|&v| v == 0.0)
                );
                continue;
            }
            assert_eq!(
                frame.failures.len(),
                1,
                "{kind}/{format} level {level} corrupt={corrupt}: {:?}",
                frame.failures
            );
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
        let path = rendition_or_reencode(&dir, name);
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

/// A WebM of fixture `name` for comparisons computed from the file itself:
/// the persisted bytes when they are available, otherwise the current
/// converter's exact settings with the same fixture.
fn rendition_or_reencode(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    use std::process::Command;
    let bin = std::path::PathBuf::from(std::env::var("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    let evidence = std::env::var_os("PM_MEDIA_RENDITIONS").map(std::path::PathBuf::from);
    if let Some(e) = &evidence {
        e.join(name).join("native-source.webm")
    } else {
        let path = dir.join(format!("{name}.webm"));
        let input = support::repo_root().join("engine/fixtures").join(name);
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

/// The WebM today's import persisted for fixture `name`: the bytes the
/// hidden app stored (`PM_MEDIA_RENDITIONS`, or the private corpus's
/// `animation-renditions/<name>/native-source.webm`). A re-encode with the
/// current FFmpeg is not those bytes (its frames differ by 1 to 2 CV), so
/// there is no fallback.
fn persisted_rendition(name: &str) -> std::path::PathBuf {
    let base = std::env::var_os("PM_MEDIA_RENDITIONS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::PathBuf::from(std::env::var("HOME").unwrap())
                .join("Library/Application Support/Powermove Engine Corpus/animation-renditions")
        });
    let path = base.join(name).join("native-source.webm");
    assert!(
        path.is_file(),
        "{} is missing: capture the app-persisted rendition first",
        path.display()
    );
    path
}

/// The capture's import hash identifies the original, but its loaded document
/// and Chromium frames use the persisted WebM. Resolve that asset to the
/// stored rendition without changing the composition, transforms or times.
#[test]
#[ignore = "needs the private v1 corpus capture and persisted renditions (docs/engine/media.md)"]
fn animated_corpus_projects_use_stored_renditions_within_the_video_class() {
    let capture = support::newest_capture("v1-").expect("a v1-* corpus capture");
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
        *p.resolver.paths.values_mut().next().unwrap() = persisted_rendition(source);
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

/// Ten 16-bit frames `shot.0001.png`… at 10 fps, read in place (the small
/// file a sequence asset retains), each coded by its number in red.
struct CodedSequence {
    _dir: tempfile::TempDir,
    frames: std::path::PathBuf,
    retained: std::path::PathBuf,
}
fn coded_sequence() -> CodedSequence {
    coded_sequence_at([10, 1])
}
/// The ten coded files numbered 1–10, imported at `rate`.
fn coded_sequence_at(rate: [u32; 2]) -> CodedSequence {
    let dir = tempfile::tempdir().unwrap();
    let frames = dir.path().join("frames");
    std::fs::create_dir(&frames).unwrap();
    for n in 1..=10u16 {
        let mut bytes = Vec::new();
        for _ in 0..16 * 16 {
            for code in [n * 6_000, 65_535 - n * 6_000, 30_000] {
                bytes.extend_from_slice(&code.to_be_bytes());
            }
        }
        let file = std::fs::File::create(frames.join(format!("shot.{n:04}.png"))).unwrap();
        let mut encoder = png::Encoder::new(file, 16, 16);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&bytes).unwrap();
    }
    let manifest = pm_media::image::sequence::Manifest {
        powermove_image_sequence: 1,
        dir: frames.clone(),
        prefix: "shot.".into(),
        suffix: ".png".into(),
        padding: 4,
        first: 1,
        last: 10,
        rate,
    };
    let retained = dir.path().join("retained");
    std::fs::write(&retained, manifest.to_bytes()).unwrap();
    CodedSequence {
        _dir: dir,
        frames,
        retained,
    }
}
/// A movie of the coded frames at 10 fps, as a proxy (ProRes 4444 in
/// QuickTime) or an earlier build's transcoded rendition (VP9 WebM).
fn sequence_movie(sequence: &CodedSequence, name: &str, codec: &[&str]) -> std::path::PathBuf {
    sequence_movie_at(sequence, name, "10", codec)
}
fn sequence_movie_at(
    sequence: &CodedSequence,
    name: &str,
    rate: &str,
    codec: &[&str],
) -> std::path::PathBuf {
    let ffmpeg =
        std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    let out = sequence.frames.parent().unwrap().join(name);
    let status = std::process::Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-framerate",
            rate,
            "-start_number",
            "1",
            "-i",
        ])
        .arg(sequence.frames.join("shot.%04d.png"))
        .args(codec)
        .arg(&out)
        .status()
        .unwrap();
    assert!(status.success());
    out
}
/// The number of the coded file a sequence layer `d` shows at each time.
fn shown(d: serde_json::Value, path: &std::path::Path, from: f64, times: &[f64]) -> Vec<u16> {
    shown_in(d, path, from, times, 10, None)
}
/// [`shown`] for a sequence imported at `fps`, with an Interpret Footage
/// reading of the asset.
fn shown_in(
    d: serde_json::Value,
    path: &std::path::Path,
    from: f64,
    times: &[f64],
    fps: u32,
    interpret: Option<serde_json::Value>,
) -> Vec<u16> {
    let mut source = layer("seq", "video", d, 8.0, 8.0, 10.0);
    source["from"] = json!(from);
    let mut doc = project(16, 16, 10.0, no_fill(), vec![source]);
    asset(&mut doc, "clip", "video", "png", 16, 16, 10.0 / f64::from(fps));
    doc["assets"]["clip"]["fps"] = json!(fps);
    doc["assets"]["clip"]["imageSequence"] =
        json!({"fps":fps,"frames":10,"first":1,"last":10});
    if let Some(interpret) = interpret {
        doc["assets"]["clip"]["interpret"] = interpret;
    }
    let mut resolver = MapResolver::default();
    resolver.paths.insert("clip".into(), path.into());
    let document = load(&doc);
    let mut renderer = Renderer::headless().unwrap();
    let request = RenderRequest::at_scale(Size::new(16, 16), 1.0, Precision::Float16);
    times
        .iter()
        .map(|&t| {
            let frame = renderer
                .render(
                    &document,
                    "Ptest-root",
                    support::ticks(t),
                    &request,
                    &resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{t}: {:?}", frame.failures);
            let rgba = renderer.read_rgba(&frame).unwrap();
            let red = rgba[(8 * 16 + 8) * 4];
            (f64::from(red) * 65_535.0 / 6_000.0).round() as u16
        })
        .collect()
}

/// Trim, speed other than 1, reverse and time remapping choose the exact
/// file of a sequence read in place, as they choose a clip's frame; its
/// proxy (a QuickTime movie of the same frames) shows the same files.
#[test]
fn an_in_place_sequence_is_trimmed_retimed_reversed_and_remapped_to_the_exact_file() {
    let sequence = coded_sequence();
    let proxy = sequence_movie(
        &sequence,
        "proxy.mov",
        &[
            "-c:v",
            "prores_videotoolbox",
            "-profile:v",
            "4444",
            "-pix_fmt",
            "p416le",
        ],
    );
    let base = json!({"asset":"clip","w":16,"h":16,"fit":"stretch"});
    let with = |pairs: &[(&str, serde_json::Value)]| {
        let mut d = base.clone();
        for (k, v) in pairs {
            d[*k] = v.clone();
        }
        d
    };
    // (case, d, layer start, composition times, files shown)
    let cases = [
        (
            "plain",
            base.clone(),
            0.0,
            vec![0.0, 0.1, 0.35, 0.99, 3.0],
            vec![1, 2, 4, 10, 10],
        ),
        (
            "start",
            base.clone(),
            0.5,
            vec![0.5, 0.7, 1.45],
            vec![1, 3, 10],
        ),
        (
            "trim",
            with(&[("trim", json!(0.2))]),
            0.0,
            vec![0.0, 0.1, 0.75],
            vec![3, 4, 10],
        ),
        (
            "speed 2",
            with(&[("speed", json!(2))]),
            0.0,
            vec![0.0, 0.1, 0.3, 0.45],
            vec![1, 3, 7, 10],
        ),
        (
            "speed ½",
            with(&[("speed", json!(0.5))]),
            0.0,
            vec![0.1, 0.2, 0.3, 1.8],
            vec![1, 2, 2, 10],
        ),
        (
            "reverse",
            with(&[("speed", json!(-1)), ("trim", json!(1.0))]),
            0.0,
            vec![0.05, 0.25, 0.95],
            vec![10, 8, 1],
        ),
        (
            "remap",
            with(&[
                ("timeRemap", json!(true)),
                (
                    "sourceTime",
                    keys(
                        json!(0),
                        &[(0.0, json!(0.65)), (0.5, json!(0.35)), (1.0, json!(0.05))],
                    ),
                ),
            ]),
            0.0,
            vec![0.0, 0.5, 1.0],
            vec![7, 4, 1],
        ),
    ];
    for (case, d, from, times, files) in cases {
        assert_eq!(
            shown(d.clone(), &sequence.retained, from, &times),
            files,
            "{case}"
        );
        assert_eq!(
            shown(d, &proxy, from, &times),
            files,
            "{case} from its proxy"
        );
    }
}

/// Its files and its proxy choose a frame by one rule (section 5 rule 7,
/// as a clip's frames are chosen): with Interpret Footage's frame rate, a
/// speed other than 1, and source times just before a frame's start, both
/// show the same file.
#[test]
fn a_conformed_sequence_and_its_proxy_show_the_same_file() {
    let sequence = coded_sequence_at([24, 1]);
    let proxy = sequence_movie_at(
        &sequence,
        "proxy.mov",
        "24",
        &[
            "-c:v",
            "prores_videotoolbox",
            "-profile:v",
            "4444",
            "-pix_fmt",
            "p416le",
        ],
    );
    // A 24 fps sequence conformed to 24/1 at speed 0.9: composition frame 1
    // (1/24 s) is source 0.0375 s, 0.9 of a frame: frame 2 has not started,
    // and file 1 shows, from the files as from the proxy (whose movie
    // reading would otherwise take frame 2, within an eighth of its start).
    let conformed = json!({"frameRate": [24, 1]});
    let speed = |s: f64| json!({"asset":"clip","w":16,"h":16,"fit":"stretch","speed":s});
    let at = |frames: &[f64]| frames.iter().map(|k| k / 24.0).collect::<Vec<_>>();
    assert_eq!(
        shown_in(speed(0.9), &sequence.retained, 0.0, &[1.0 / 24.0], 24, Some(conformed.clone())),
        vec![1]
    );
    assert_eq!(
        shown_in(speed(0.9), &proxy, 0.0, &[1.0 / 24.0], 24, Some(conformed.clone())),
        vec![1]
    );
    let times = at(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
    for interpret in [Some(conformed), Some(json!({"frameRate": [25, 1]})), None] {
        for s in [0.9, 0.95, 0.88, 1.1, 1.13] {
            let files = shown_in(speed(s), &sequence.retained, 0.0, &times, 24, interpret.clone());
            let from_proxy = shown_in(speed(s), &proxy, 0.0, &times, 24, interpret.clone());
            assert_eq!(files, from_proxy, "speed {s}, {interpret:?}");
        }
    }
}

/// A frame whose file exists but can't be read (truncated, or not
/// permitted), or was rendered again at another size, shows the Media
/// Offline card as a missing one does (never a blank or stretched picture),
/// in frames that draw cards; any other render fails naming the file.
#[test]
fn an_unreadable_or_resized_sequence_frame_shows_its_card() {
    use std::os::unix::fs::PermissionsExt;
    let sequence = coded_sequence();
    let file = |n: u32| sequence.frames.join(format!("shot.{n:04}.png"));
    let bytes = std::fs::read(file(2)).unwrap();
    std::fs::write(file(2), &bytes[..bytes.len() / 2]).unwrap();
    std::fs::set_permissions(file(3), std::fs::Permissions::from_mode(0o000)).unwrap();
    std::fs::remove_file(file(4)).unwrap();
    // Frame 5 rendered again at another size.
    {
        let out = std::fs::File::create(file(5)).unwrap();
        let mut encoder = png::Encoder::new(out, 32, 16);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(&[200u8; 32 * 16 * 3]).unwrap();
    }
    let mut source = layer(
        "seq",
        "video",
        json!({"asset":"clip","w":16,"h":16,"fit":"stretch"}),
        8.0,
        8.0,
        10.0,
    );
    source["from"] = json!(0);
    let mut doc = project(16, 16, 10.0, no_fill(), vec![source]);
    asset(&mut doc, "clip", "video", "png", 16, 16, 1.0);
    doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":10,"first":1,"last":10});
    let mut resolver = MapResolver::default();
    resolver
        .paths
        .insert("clip".into(), sequence.retained.clone());
    let document = load(&doc);
    let request = RenderRequest::at_scale(Size::new(16, 16), 1.0, Precision::Float16);
    let mut viewer = Renderer::headless().unwrap();
    viewer.set_isolate_layers(true);
    // The picture at the box's centre, drawn again when the first drawing
    // found a file it could not show (as the viewer does when what it
    // has landed changes).
    let mut drawn = |t: f64| {
        let landed = viewer.media_landed();
        let mut frame = viewer
            .render(&document, "Ptest-root", support::ticks(t), &request, &resolver)
            .unwrap();
        if viewer.media_landed() != landed {
            frame = viewer
                .render(&document, "Ptest-root", support::ticks(t), &request, &resolver)
                .unwrap();
            assert_eq!(viewer.media_landed(), landed + 1, "{t}: drawn again once");
        }
        assert!(frame.failures.is_empty(), "{t}: {:?}", frame.failures);
        let rgba = viewer.read_rgba(&frame).unwrap();
        rgba[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 4].to_vec()
    };
    let card = drawn(0.3);
    let file1 = drawn(0.0);
    assert_ne!(card, file1, "frame 4 (missing) shows its card");
    assert_eq!(drawn(0.1), card, "frame 2 (truncated) shows the card");
    assert_eq!(drawn(0.2), card, "frame 3 (not permitted) shows the card");
    assert_eq!(drawn(0.4), card, "frame 5 (another size) shows the card");

    let mut export = Renderer::headless().unwrap();
    for (t, name) in [
        (0.1, "shot.0002.png"),
        (0.2, "shot.0003.png"),
        (0.4, "is 32×16, not 16×16 like the sequence: shot.0005.png"),
    ] {
        let failures = match export.render(&document, "Ptest-root", support::ticks(t), &request, &resolver) {
            Ok(frame) => frame.failures,
            Err(e) => panic!("{t}: {e}"),
        };
        assert!(
            failures.iter().any(|f| f.message.contains(name)),
            "{t}: {failures:?}"
        );
    }
    std::fs::set_permissions(file(3), std::fs::Permissions::from_mode(0o644)).unwrap();
}

/// An uncompressed little-endian 16-bit RGBA TIFF whose alpha is
/// associated (ExtraSamples = 1): `rgba` premultiplied, 0–65535.
fn associated_tiff(path: &std::path::Path, w: u32, h: u32, rgba: [u16; 4]) {
    let mut b: Vec<u8> = b"II*\0".to_vec();
    b.extend_from_slice(&8u32.to_le_bytes());
    let tags: [(u16, u16, u32, u32); 11] = [
        (256, 4, 1, w),
        (257, 4, 1, h),
        (258, 3, 4, 0),
        (259, 3, 1, 1),
        (262, 3, 1, 2),
        (273, 4, 1, 0),
        (277, 3, 1, 4),
        (278, 4, 1, h),
        (279, 4, 1, w * h * 8),
        (284, 3, 1, 1),
        (338, 3, 1, 1),
    ];
    let ifd_len = 2 + tags.len() as u32 * 12 + 4;
    let bits_at = 8 + ifd_len;
    let data_at = bits_at + 8;
    b.extend_from_slice(&(tags.len() as u16).to_le_bytes());
    for (tag, kind, count, value) in tags {
        let value = match tag {
            258 => bits_at,
            273 => data_at,
            _ => value,
        };
        b.extend_from_slice(&tag.to_le_bytes());
        b.extend_from_slice(&kind.to_le_bytes());
        b.extend_from_slice(&count.to_le_bytes());
        if kind == 3 && count == 1 {
            b.extend_from_slice(&(value as u16).to_le_bytes());
            b.extend_from_slice(&[0, 0]);
        } else {
            b.extend_from_slice(&value.to_le_bytes());
        }
    }
    b.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..4 {
        b.extend_from_slice(&16u16.to_le_bytes());
    }
    for _ in 0..w * h {
        for v in rgba {
            b.extend_from_slice(&v.to_le_bytes());
        }
    }
    std::fs::write(path, b).unwrap();
}

/// A sequence whose files hold associated (premultiplied) alpha, as
/// OpenEXR always does and a TIFF may: its proxy, made by the generator,
/// composites over the composition exactly as its files do (half-covered
/// red stays half-covered red, never darkened by a second multiply).
#[test]
fn an_associated_alpha_sequence_and_its_proxy_composite_alike() {
    use pm_protocol::priority::{ProxyFormat, ProxySize};
    let dir = tempfile::tempdir().unwrap();
    let ffmpeg =
        std::path::PathBuf::from(std::env::var_os("PM_FFMPEG_DIR").unwrap()).join("bin/ffmpeg");
    let exr = dir.path().join("exr");
    let tif = dir.path().join("tif");
    std::fs::create_dir(&exr).unwrap();
    std::fs::create_dir(&tif).unwrap();
    // Stored linear R 0.25, G = B = 0, A 0.5: a straight 0.5 red at half
    // coverage. gbrapf32le planes are G, B, R, A.
    let mut planes = Vec::new();
    for value in [0.0f32, 0.0, 0.25, 0.5] {
        for _ in 0..16 * 16 {
            planes.extend_from_slice(&value.to_le_bytes());
        }
    }
    let raw = dir.path().join("frame.raw");
    std::fs::write(&raw, planes).unwrap();
    for n in 1..=2 {
        let status = std::process::Command::new(&ffmpeg)
            .args(["-v", "error", "-y", "-f", "rawvideo", "-pix_fmt", "gbrapf32le"])
            .args(["-s", "16x16", "-i"])
            .arg(&raw)
            .args(["-c:v", "exr", "-format", "half", "-frames:v", "1"])
            .arg(exr.join(format!("shot.{n:04}.exr")))
            .status()
            .unwrap();
        assert!(status.success());
        associated_tiff(&tif.join(format!("shot.{n:04}.tif")), 16, 16, [16384, 0, 0, 32768]);
    }
    for (folder, suffix) in [(&exr, ".exr"), (&tif, ".tif")] {
        let manifest = pm_media::image::sequence::Manifest {
            powermove_image_sequence: 1,
            dir: folder.clone(),
            prefix: "shot.".into(),
            suffix: suffix.into(),
            padding: 4,
            first: 1,
            last: 2,
            rate: [10, 1],
        };
        let retained = folder.join("retained");
        std::fs::write(&retained, manifest.to_bytes()).unwrap();
        let cancel = std::sync::atomic::AtomicBool::new(false);
        let hash = pm_proxy::sequence_identity(&retained, &cancel)
            .unwrap()
            .unwrap();
        let store = pm_proxy::ProxyStore::open(folder.join("store")).unwrap();
        let proxy = pm_proxy::generate_with_disk_probe(
            pm_proxy::GenerationRequest {
                source: &retained,
                source_hash: &hash,
                format: ProxyFormat::ProResProxy,
                size: ProxySize::Full,
                ffmpeg: &ffmpeg,
                ffprobe: &ffmpeg.with_file_name("ffprobe"),
                store: &store,
                control: pm_proxy::GenerationControl {
                    cancel: &cancel,
                    pause: &|| false,
                    on_pause: &|_| {},
                    on_progress: &|_| {},
                },
            },
            &|_| Ok((40 << 30, 100 << 30)),
        )
        .unwrap()
        .stored
        .path;
        // Over a white composition, where a second multiply shows most.
        let composited = |path: &std::path::Path, depth: u32| {
            let mut source = layer(
                "seq",
                "video",
                json!({"asset":"clip","w":16,"h":16,"fit":"stretch"}),
                8.0,
                8.0,
                1.0,
            );
            source["from"] = json!(0);
            let mut doc = project(16, 16, 1.0, solid_fill("#ffffff"), vec![source]);
            doc["colorDepth"] = json!(depth);
            asset(&mut doc, "clip", "video", "png", 16, 16, 0.2);
            doc["assets"]["clip"]["imageSequence"] =
                json!({"fps":10,"frames":2,"first":1,"last":2});
            let mut resolver = MapResolver::default();
            resolver.paths.insert("clip".into(), path.into());
            let document = load(&doc);
            let mut renderer = Renderer::headless().unwrap();
            let request = RenderRequest::at_scale(Size::new(16, 16), 1.0, Precision::Float32);
            let frame = renderer
                .render(&document, "Ptest-root", support::ticks(0.0), &request, &resolver)
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let rgba = renderer.read_rgba(&frame).unwrap();
            rgba[(8 * 16 + 8) * 4..(8 * 16 + 8) * 4 + 3].to_vec()
        };
        for depth in [32, 8] {
            let files = composited(&retained, depth);
            let from_proxy = composited(&proxy, depth);
            for (c, (a, b)) in files.iter().zip(&from_proxy).enumerate() {
                assert!(
                    (a - b).abs() < 0.02,
                    "{suffix} at depth {depth}: channel {c}: files {files:?}, proxy {from_proxy:?}"
                );
            }
        }
    }
}

/// A project saved with an earlier build's sequence asset (its frames
/// transcoded to a WebM rendition) plays exactly as before: the video path,
/// frame k at k / fps.
#[test]
fn an_older_transcoded_sequence_asset_shows_the_same_frames() {
    let sequence = coded_sequence();
    let webm = sequence_movie(
        &sequence,
        "rendition.webm",
        &[
            "-c:v",
            "libvpx-vp9",
            "-lossless",
            "1",
            "-pix_fmt",
            "yuv444p",
        ],
    );
    let d = json!({"asset":"clip","w":16,"h":16,"fit":"stretch"});
    assert_eq!(
        shown(d, &webm, 0.0, &[0.0, 0.1, 0.35, 0.99, 3.0]),
        vec![1, 2, 4, 10, 10]
    );
}

/// While playing, a sequence's next files are decoded ahead on workers: the
/// frame that meets one draws it without decoding (each file is decoded
/// once). Paused and dragged frames read nothing ahead.
#[test]
fn a_playing_sequence_reads_its_next_files_ahead() {
    let sequence = coded_sequence();
    let mut source = layer(
        "seq",
        "video",
        json!({"asset":"clip","w":16,"h":16,"fit":"stretch"}),
        8.0,
        8.0,
        10.0,
    );
    source["from"] = json!(0);
    let mut doc = project(16, 16, 10.0, no_fill(), vec![source]);
    asset(&mut doc, "clip", "video", "png", 16, 16, 1.0);
    doc["assets"]["clip"]["imageSequence"] = json!({"fps":10,"frames":10,"first":1,"last":10});
    let mut resolver = MapResolver::default();
    resolver
        .paths
        .insert("clip".into(), sequence.retained.clone());
    let document = load(&doc);
    for ahead in [false, true] {
        let mut renderer = Renderer::headless().unwrap();
        let mut request = RenderRequest::at_scale(Size::new(16, 16), 1.0, Precision::Float16);
        request.read_ahead = ahead;
        // Files 1, 2 and 3 show at 0, 0.1 and 0.2 s (10 fps in 30).
        let mut decodes = vec![];
        for t in [0.0, 0.1, 0.2] {
            let before = pm_perf::snapshot();
            let frame = renderer
                .render(
                    &document,
                    "Ptest-root",
                    support::ticks(t),
                    &request,
                    &resolver,
                )
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            decodes.push(
                pm_perf::snapshot()
                    .since(&before)
                    .get(pm_perf::Counter::StillDecodes),
            );
            // Workers finish between frames, as they do while a frame is shown.
            assert!(renderer.finish_media_preparation().is_empty());
        }
        // A file the frame reads itself is decoded once, never again for
        // its facts.
        if ahead {
            assert_eq!(decodes, [1, 0, 0], "decoded ahead");
        } else {
            assert_eq!(decodes, [1, 1, 1], "paused");
        }
    }
}
