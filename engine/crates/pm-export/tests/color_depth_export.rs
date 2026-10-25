//! Color depth 8 through the shipped job-export formats. The project is a
//! deterministic three-patch composition, with alpha outside the patches, and
//! a separate translucent/overlap sensitivity scene.
//! Every case renders through pm-render, encodes, verifies, and is decoded or
//! probed independently. There is no legacy-export path in this test.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use pm_export::Overwrite;
use pm_export::render::{
    ExportJob, MediaPaths, Progress, Unrestricted, plan_export_for, run_export,
};
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput, Container, DeliveryOptions, ExportFormat, ExportSettings, ProResProfile,
    QualityTier, WebmCodec,
};
use pm_render::{RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::{layer, load, no_fill, project, set};

const W: u32 = 128;
const H: u32 = 96;
const FRAMES: u32 = 3;

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "pm-depth8-export-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn scene(depth: u8) -> Value {
    let patches: Vec<_> = [
        ("red", "#E03028", 0.0, 0.0),
        ("green", "#20C840", 72.0, 0.0),
        ("blue", "#3050E0", 0.0, 56.0),
    ]
    .into_iter()
    .map(|(id, color, x, y)| {
        layer(
            id,
            "solid",
            json!({"color": color, "w": 56, "h": 40}),
            x,
            y,
            0.1,
        )
    })
    .collect();
    let mut doc = project(W, H, 0.1, no_fill(), patches);
    doc["colorDepth"] = json!(depth);
    doc
}

fn sensitive_scene() -> Value {
    let mut doc = scene(8);
    // One translucent patch and one 50%-opacity overlap exercise the
    // precision and alpha path without disturbing the opaque RGB anchors.
    let mut translucent = layer(
        "half-alpha",
        "solid",
        json!({"color": "#E0603080", "w": 32, "h": 24}),
        80.0,
        60.0,
        0.1,
    );
    set(&mut translucent, "opacity", json!(100));
    doc["layers"].as_array_mut().unwrap().push(translucent);
    let mut overlap = layer(
        "half-overlap",
        "solid",
        json!({"color": "#E03028", "w": 16, "h": 16}),
        108.0,
        20.0,
        0.1,
    );
    set(&mut overlap, "opacity", json!(50));
    doc["layers"].as_array_mut().unwrap().insert(0, overlap);
    doc
}

fn settings(format: ExportFormat, alpha: AlphaOutput) -> ExportSettings {
    let image = format.is_image();
    let eight_bit_output = match format {
        ExportFormat::ProRes { .. } => false,
        ExportFormat::H264 | ExportFormat::Webm { .. } => true,
        ExportFormat::Hevc { ten_bit, .. } => !ten_bit,
        ExportFormat::PngSequence { sixteen_bit } | ExportFormat::PngStill { sixteen_bit } => {
            !sixteen_bit
        }
    };
    ExportSettings {
        format,
        alpha,
        quality: QualityTier::Max,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(i64::from(FRAMES) * TICKS_PER_SECOND / 30).unwrap(),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: if image {
            OutputColor::SrgbImage
        } else {
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            }
        },
        // The app's wireSettings sends a Float32 hint and enables dither for
        // delivery formats at 8 bits. The document's depth takes precedence.
        precision: Precision::Float32,
        dither: eight_bit_output,
        draft_from_proxies: false,
        delivery: DeliveryOptions {
            preview: true,
            ..DeliveryOptions::default()
        },
    }
}

fn tool(name: &str) -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("PM_FFMPEG_DIR").expect("source engine/env.sh before this test"),
    );
    let path = root.join("bin").join(name);
    assert!(path.is_file(), "missing {}", path.display());
    path
}

fn video_info(path: &Path) -> String {
    let out = Command::new(tool("ffprobe"))
        .args([
            "-v",
            "error",
            "-count_frames",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=codec_name,profile,pix_fmt,width,height,nb_read_frames,color_range,color_space,color_transfer,color_primaries:stream_tags=alpha_mode",
            "-of",
            "default=nw=1",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "ffprobe {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn video_pixels(path: &Path, pixel_format: &str, decoder: Option<&str>, bt709: bool) -> Vec<u8> {
    let mut command = Command::new(tool("ffmpeg"));
    command.args(["-v", "error"]);
    if let Some(decoder) = decoder {
        command.args(["-c:v", decoder]);
    }
    command.arg("-i").arg(path);
    if bt709 {
        command.args([
            "-vf",
            "scale=in_color_matrix=bt709:in_range=tv:out_color_matrix=bt709:out_range=pc",
        ]);
    }
    let out = command
        .args([
            "-frames:v",
            "3",
            "-f",
            "rawvideo",
            "-pix_fmt",
            pixel_format,
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "decode {}: {}",
        path.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        out.stdout.len(),
        (W * H * if pixel_format == "rgba" { 4 } else { 3 } * FRAMES) as usize,
        "decoded three-frame shape"
    );
    out.stdout
}

fn video_rgb(path: &Path) -> Vec<u8> {
    video_pixels(path, "rgb24", None, true)
}

fn png_rgba(path: &Path) -> Vec<u8> {
    let bytes = std::fs::read(path).unwrap();
    let image = image::load_from_memory(&bytes).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (W, H));
    image.into_raw()
}

const ANCHORS: [(u32, u32); 3] = [(20, 20), (92, 20), (20, 72)];

fn channels(bytes: &[u8], x: u32, y: u32, stride: usize) -> [u8; 3] {
    let offset = (y * W + x) as usize * stride;
    bytes[offset..offset + 3].try_into().unwrap()
}

fn exceeds_by(a: u8, b: u8, margin: i16) -> bool {
    i16::from(a) - i16::from(b) > margin
}

#[derive(Clone, Copy)]
struct Case {
    name: &'static str,
    format: ExportFormat,
    alpha: AlphaOutput,
    ext: &'static str,
    codec: &'static str,
    depth: u32,
}

#[test]
fn depth8_project_exports_every_shipped_format() {
    let scratch = Scratch::new();
    let document = Arc::new(load(&scene(8)));
    let depth16_document = Arc::new(load(&scene(16)));
    let mut renderer = Renderer::headless().unwrap();
    let request = RenderRequest::sized(Size::new(W, H), Precision::Float32);
    let frame = renderer
        .render(
            &document,
            "Ptest-root",
            Ticks::ZERO,
            &request,
            &MediaPaths::default(),
        )
        .unwrap();
    assert_eq!(
        frame.color.format(),
        wgpu::TextureFormat::Rgba8Unorm,
        "the actual project frame must use the 8-bit working target"
    );
    let prores = |profile| ExportFormat::ProRes { profile };
    let hevc = |ten_bit, container| ExportFormat::Hevc { ten_bit, container };
    let webm = |codec| ExportFormat::Webm { codec };
    let cases = [
        Case {
            name: "png-seq8",
            format: ExportFormat::PngSequence { sixteen_bit: false },
            alpha: AlphaOutput::Straight,
            ext: "",
            codec: "png",
            depth: 8,
        },
        Case {
            name: "png-still8",
            format: ExportFormat::PngStill { sixteen_bit: false },
            alpha: AlphaOutput::Straight,
            ext: "png",
            codec: "png",
            depth: 8,
        },
        Case {
            name: "png-seq8-opaque",
            format: ExportFormat::PngSequence { sixteen_bit: false },
            alpha: AlphaOutput::None,
            ext: "",
            codec: "png",
            depth: 8,
        },
        Case {
            name: "png-seq16",
            format: ExportFormat::PngSequence { sixteen_bit: true },
            alpha: AlphaOutput::Straight,
            ext: "",
            codec: "png",
            depth: 16,
        },
        Case {
            name: "png-still16",
            format: ExportFormat::PngStill { sixteen_bit: true },
            alpha: AlphaOutput::Straight,
            ext: "png",
            codec: "png",
            depth: 16,
        },
        Case {
            name: "png-still16-opaque",
            format: ExportFormat::PngStill { sixteen_bit: true },
            alpha: AlphaOutput::None,
            ext: "png",
            codec: "png",
            depth: 16,
        },
        Case {
            name: "prores-proxy",
            format: prores(ProResProfile::Proxy),
            alpha: AlphaOutput::None,
            ext: "mov",
            codec: "prores",
            depth: 10,
        },
        Case {
            name: "prores-lt",
            format: prores(ProResProfile::Lt),
            alpha: AlphaOutput::None,
            ext: "mov",
            codec: "prores",
            depth: 10,
        },
        Case {
            name: "prores-422",
            format: prores(ProResProfile::Standard),
            alpha: AlphaOutput::None,
            ext: "mov",
            codec: "prores",
            depth: 10,
        },
        Case {
            name: "prores-hq",
            format: prores(ProResProfile::Hq),
            alpha: AlphaOutput::None,
            ext: "mov",
            codec: "prores",
            depth: 10,
        },
        Case {
            name: "prores-4444",
            format: prores(ProResProfile::P4444),
            alpha: AlphaOutput::Straight,
            ext: "mov",
            codec: "prores",
            depth: 12,
        },
        Case {
            name: "prores-4444xq",
            format: prores(ProResProfile::P4444Xq),
            alpha: AlphaOutput::Premultiplied,
            ext: "mov",
            codec: "prores",
            depth: 12,
        },
        Case {
            name: "h264",
            format: ExportFormat::H264,
            alpha: AlphaOutput::None,
            ext: "mp4",
            codec: "h264",
            depth: 8,
        },
        Case {
            name: "hevc-mp4",
            format: hevc(false, Container::Mp4),
            alpha: AlphaOutput::None,
            ext: "mp4",
            codec: "hevc",
            depth: 8,
        },
        Case {
            name: "hevc10-mp4",
            format: hevc(true, Container::Mp4),
            alpha: AlphaOutput::None,
            ext: "mp4",
            codec: "hevc",
            depth: 10,
        },
        Case {
            name: "hevc-mov",
            format: hevc(false, Container::Mov),
            alpha: AlphaOutput::None,
            ext: "mov",
            codec: "hevc",
            depth: 8,
        },
        Case {
            name: "hevc10-mov",
            format: hevc(true, Container::Mov),
            alpha: AlphaOutput::None,
            ext: "mov",
            codec: "hevc",
            depth: 10,
        },
        Case {
            name: "hevc-alpha",
            format: hevc(false, Container::Mov),
            alpha: AlphaOutput::Straight,
            ext: "mov",
            codec: "hevc",
            depth: 8,
        },
        Case {
            name: "vp9",
            format: webm(WebmCodec::Vp9),
            alpha: AlphaOutput::None,
            ext: "webm",
            codec: "vp9",
            depth: 8,
        },
        Case {
            name: "vp9-alpha",
            format: webm(WebmCodec::Vp9),
            alpha: AlphaOutput::Straight,
            ext: "webm",
            codec: "vp9",
            depth: 8,
        },
        Case {
            name: "vp8",
            format: webm(WebmCodec::Vp8),
            alpha: AlphaOutput::None,
            ext: "webm",
            codec: "vp8",
            depth: 8,
        },
        Case {
            name: "vp8-alpha",
            format: webm(WebmCodec::Vp8),
            alpha: AlphaOutput::Straight,
            ext: "webm",
            codec: "vp8",
            depth: 8,
        },
    ];
    let mut png8_sequence = None;
    let mut png16_sequence = None;
    let mut png16_raw: Option<Vec<u16>> = None;
    let mut video_reference: Option<Vec<u8>> = None;
    let mut jpeg_seen = false;
    for case in cases {
        let destination = if case.ext.is_empty() {
            scratch.path().join(case.name)
        } else {
            scratch.path().join(format!("{}.{}", case.name, case.ext))
        };
        let job = ExportJob {
            document: Arc::clone(&document),
            composition: "Ptest-root".into(),
            settings: settings(case.format, case.alpha),
            media: Arc::new(MediaPaths::default()),
            destination: destination.clone(),
            overwrite: Overwrite::Fail,
            mode: Some(0o600),
            seed: 73,
        };
        assert_eq!(job.settings.precision, Precision::Float32, "{}", case.name);
        assert_eq!(job.settings.dither, case.depth <= 8, "{}", case.name);
        let plan =
            plan_export_for(&job, &renderer).unwrap_or_else(|e| panic!("{} plan: {e}", case.name));
        assert_eq!(
            plan.request.precision,
            Precision::Float32,
            "{} must keep the app's precision hint",
            case.name
        );
        assert_eq!(
            renderer.device_target_format(&document, &plan.request),
            wgpu::TextureFormat::Rgba8Unorm,
            "{}: project depth must override the precision hint",
            case.name
        );
        let started = Instant::now();
        let delivered = run_export(
            &mut renderer,
            &job,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |progress: Progress| {
                if let Some(preview) = progress.preview {
                    let decoded = image::load_from_memory(&preview.jpeg).unwrap().to_rgb8();
                    assert_eq!(decoded.dimensions(), (W, H), "JPEG preview size");
                    if let Some(reference) = video_reference.as_ref() {
                        let rgb = decoded.as_raw();
                        for (x, y) in ANCHORS {
                            let actual = channels(rgb, x, y, 3);
                            let expected = channels(reference, x, y, 4);
                            let max_error = actual
                                .into_iter()
                                .zip(expected)
                                .map(|(a, b)| a.abs_diff(b))
                                .max()
                                .unwrap();
                            assert!(
                                max_error <= 24,
                                "JPEG preview at ({x},{y}): {actual:?}, PNG {expected:?}"
                            );
                        }
                    }
                    jpeg_seen = true;
                }
            },
        )
        .unwrap_or_else(|e| panic!("{}: {e}", case.name));
        let report = &delivered.report;
        assert_eq!(delivered.path, destination, "{}", case.name);
        assert_eq!(report.depth, case.depth, "{}", case.name);
        assert_eq!(
            report.alpha,
            case.alpha != AlphaOutput::None,
            "{}",
            case.name
        );
        assert_eq!(
            report.frames,
            if matches!(case.format, ExportFormat::PngStill { .. }) {
                1
            } else {
                FRAMES as u64
            },
            "{}",
            case.name
        );
        assert!(report.bytes > 0, "{}", case.name);
        eprintln!(
            "MEASURE depth8 {}: codec={} depth={} frames={} bytes={} max_cv={:.3} elapsed_ms={:.0}",
            case.name,
            report.codec,
            report.depth,
            report.frames,
            report.bytes,
            report.max_pixel_error_cv(),
            started.elapsed().as_secs_f64() * 1000.0
        );
        if case.codec == "png" {
            let path = if case.ext.is_empty() {
                let mut files: Vec<_> = std::fs::read_dir(&destination)
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .collect();
                files.sort();
                assert_eq!(files.len(), FRAMES as usize, "{}", case.name);
                files[0].clone()
            } else {
                destination.clone()
            };
            let (info, decoded) = pm_export::png::read(&path, true).unwrap();
            assert_eq!(
                (info.width, info.height, info.bit_depth, info.alpha),
                (W, H, case.depth as u8, case.alpha != AlphaOutput::None),
                "{}",
                case.name
            );
            let rgba = png_rgba(&path);
            if case.name == "png-seq8" {
                png8_sequence = Some(rgba.clone());
            }
            if case.name == "png-seq16" {
                png16_sequence = Some(rgba.clone());
                png16_raw = Some(decoded.unwrap().planes[0].u16().unwrap().to_vec());
            }
            if case.name == "png-still8" {
                assert_eq!(
                    Some(&rgba),
                    png8_sequence.as_ref(),
                    "8-bit still/sequence pixels"
                );
            }
            if case.name == "png-still16" {
                assert_eq!(
                    Some(&rgba),
                    png16_sequence.as_ref(),
                    "16-bit still/sequence pixels"
                );
            }
            // The last row distinguishes transparent and opaque delivery and
            // catches a bottom-up readback or alpha-row stride error.
            assert_eq!(
                rgba[((H - 1) * W * 4 + (W - 1) * 4 + 3) as usize],
                if case.alpha == AlphaOutput::None {
                    255
                } else {
                    0
                },
                "{}",
                case.name
            );
            if case.name == "png-still8" {
                let [r, g, b] = channels(&rgba, 20, 20, 4);
                assert!(
                    exceeds_by(r, g, 90) && exceeds_by(r, b, 90),
                    "top-left red patch: {r} {g} {b}"
                );
                let [r, g, b] = channels(&rgba, 92, 20, 4);
                assert!(
                    exceeds_by(g, r, 90) && exceeds_by(g, b, 90),
                    "top-right green patch: {r} {g} {b}"
                );
                let [r, g, b] = channels(&rgba, 20, 72, 4);
                assert!(
                    exceeds_by(b, r, 90) && exceeds_by(b, g, 90),
                    "bottom-left blue patch: {r} {g} {b}"
                );
                for (x, y) in ANCHORS {
                    assert_eq!(
                        rgba[((y * W + x) * 4 + 3) as usize],
                        255,
                        "opaque patch alpha"
                    );
                }
                video_reference = Some(rgba);
            }
        } else {
            let info = video_info(&destination);
            assert!(
                info.contains(&format!("codec_name={}", case.codec)),
                "{}: {info}",
                case.name
            );
            assert!(
                info.contains(&format!("width={W}")) && info.contains(&format!("height={H}")),
                "{}: {info}",
                case.name
            );
            assert!(
                info.contains(&format!("nb_read_frames={FRAMES}")),
                "{}: {info}",
                case.name
            );
            // WebM carries the export's colour description, limited range.
            let (space, range) = ("bt709", "tv");
            assert!(
                info.contains(&format!("color_space={space}")),
                "{}: {info}",
                case.name
            );
            assert!(
                info.contains(&format!("color_range={range}")),
                "{}: {info}",
                case.name
            );
            if case.name.starts_with("vp") {
                assert_eq!(
                    info.contains("TAG:ALPHA_MODE=1"),
                    case.alpha != AlphaOutput::None,
                    "{}: {info}",
                    case.name
                );
            }
            let rgb = video_rgb(&destination);
            let reference = video_reference
                .as_ref()
                .expect("lossless PNG ground truth first");
            let mut max_tagged_error = 0;
            for decoded_frame in rgb.chunks_exact((W * H * 3) as usize) {
                for (x, y) in ANCHORS {
                    let actual = channels(decoded_frame, x, y, 3);
                    let expected = channels(reference, x, y, 4);
                    max_tagged_error = max_tagged_error.max(
                        actual
                            .into_iter()
                            .zip(expected)
                            .map(|(a, b)| a.abs_diff(b))
                            .max()
                            .unwrap(),
                    );
                }
            }
            assert!(
                max_tagged_error <= 4,
                "{}: tagged BT.709 decoded anchor error {max_tagged_error}",
                case.name
            );
            eprintln!(
                "MEASURE {} BT709/tv tagged max anchor error across {} frames={max_tagged_error}",
                case.name, FRAMES
            );
            if case.name.starts_with("vp8") {
                eprintln!(
                    "MEASURE {} color tags: {}; explicit BT709 max anchor error={max_tagged_error}",
                    case.name,
                    info.replace('\n', " ")
                );
                let mut baseline_settings = settings(case.format, case.alpha);
                baseline_settings.precision = Precision::Float16;
                baseline_settings.dither = false;
                let baseline_destination = scratch
                    .path()
                    .join(format!("{}-depth16-no-dither.webm", case.name));
                let baseline_job = ExportJob {
                    document: Arc::clone(&depth16_document),
                    composition: "Ptest-root".into(),
                    settings: baseline_settings,
                    media: Arc::new(MediaPaths::default()),
                    destination: baseline_destination.clone(),
                    overwrite: Overwrite::Fail,
                    mode: Some(0o600),
                    seed: 73,
                };
                let baseline_plan = plan_export_for(&baseline_job, &renderer).unwrap();
                assert_eq!(baseline_plan.request.precision, Precision::Float16);
                run_export(
                    &mut renderer,
                    &baseline_job,
                    &Unrestricted,
                    &AtomicBool::new(false),
                    &mut |_| {},
                )
                .unwrap_or_else(|e| panic!("{} depth16 baseline: {e}", case.name));
                let baseline_rgb = video_rgb(&baseline_destination);
                let mut baseline8_settings = settings(case.format, case.alpha);
                baseline8_settings.dither = false;
                let baseline8_destination = scratch
                    .path()
                    .join(format!("{}-depth8-no-dither.webm", case.name));
                let baseline8_job = ExportJob {
                    document: Arc::clone(&document),
                    composition: "Ptest-root".into(),
                    settings: baseline8_settings,
                    media: Arc::new(MediaPaths::default()),
                    destination: baseline8_destination.clone(),
                    overwrite: Overwrite::Fail,
                    mode: Some(0o600),
                    seed: 73,
                };
                run_export(
                    &mut renderer,
                    &baseline8_job,
                    &Unrestricted,
                    &AtomicBool::new(false),
                    &mut |_| {},
                )
                .unwrap_or_else(|e| panic!("{} depth8 no-dither baseline: {e}", case.name));
                let baseline8_rgb = video_rgb(&baseline8_destination);
                assert_eq!(
                    baseline8_rgb, baseline_rgb,
                    "{}: no-dither depth8 vs depth16 decoded RGB frames",
                    case.name
                );
                eprintln!(
                    "MEASURE no-dither depth8 {} vs depth16: {} decoded RGB bytes, 0 mismatches across {} frames",
                    case.name,
                    baseline8_rgb.len(),
                    FRAMES
                );
            }
            if case.name == "hevc-alpha" {
                let decoded = video_pixels(&destination, "rgba", None, false);
                let corner_alpha = decoded[((H - 1) * W * 4 + (W - 1) * 4 + 3) as usize];
                let opaque_alpha = decoded[((20 * W + 20) * 4 + 3) as usize];
                assert_eq!((corner_alpha, opaque_alpha), (0, 255));
                eprintln!(
                    "MEASURE HEVC alpha external FFprobe: {}; FFmpeg RGBA corner/opaque alpha={corner_alpha}/{opaque_alpha}; native almo/decoded verification alpha={}",
                    info.replace('\n', " "),
                    report.alpha
                );
            }
        }
    }
    assert!(jpeg_seen, "real export previews encoded as JPEG");
    let rgba8 = video_reference.expect("PNG ground truth produced");
    let rgba16 = png16_raw.expect("raw 16-bit PNG samples produced");
    for (x, y) in [ANCHORS[0], ANCHORS[1], ANCHORS[2], (92, 72), (W - 1, H - 1)] {
        let at = ((y * W + x) * 4) as usize;
        let narrow = &rgba8[at..at + 4];
        let wide = &rgba16[at..at + 4];
        eprintln!("MEASURE raw PNG at ({x},{y}): rgba8={narrow:?} rgba16={wide:?}");
        assert_eq!(
            wide[3],
            u16::from(narrow[3]) * 257,
            "16-bit PNG alpha at ({x},{y})"
        );
        for channel in 0..3 {
            assert_eq!(
                wide[channel],
                u16::from(narrow[channel]) * 257,
                "8-bit working RGB expanded to raw 16-bit PNG at ({x},{y}) channel {channel}"
            );
        }
    }
}

#[test]
fn depth8_png_keeps_translucent_and_overlap_witnesses() {
    let scratch = Scratch::new();
    let document = Arc::new(load(&sensitive_scene()));
    let mut renderer = Renderer::headless().unwrap();
    let request = RenderRequest::sized(Size::new(W, H), Precision::Float32);
    let frame = renderer
        .render(
            &document,
            "Ptest-root",
            Ticks::ZERO,
            &request,
            &MediaPaths::default(),
        )
        .unwrap();
    assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba8Unorm);
    let destination = scratch.path().join("sensitive.png");
    let job = ExportJob {
        document: Arc::clone(&document),
        composition: "Ptest-root".into(),
        settings: settings(
            ExportFormat::PngStill { sixteen_bit: false },
            AlphaOutput::Straight,
        ),
        media: Arc::new(MediaPaths::default()),
        destination: destination.clone(),
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 73,
    };
    let plan = plan_export_for(&job, &renderer).unwrap();
    assert_eq!(plan.request.precision, Precision::Float32);
    assert!(
        job.settings.dither,
        "PNG8 uses the app's 8-bit dither setting"
    );
    run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let rgba = png_rgba(&destination);
    let half = &rgba[((72 * W + 92) * 4) as usize..((72 * W + 92) * 4 + 4) as usize];
    assert!((125..=130).contains(&half[3]), "50%-alpha patch: {half:?}");
    let overlap = channels(&rgba, 112, 24, 4);
    assert!(
        overlap[0] > 60 && overlap[1] > 80 && overlap[2] < 70,
        "50%-opacity overlap on green: {overlap:?}"
    );
    eprintln!("MEASURE depth8 sensitivity: half-alpha={half:?} overlap={overlap:?}");
    let wide_destination = scratch.path().join("sensitive16.png");
    let wide_job = ExportJob {
        document: Arc::clone(&document),
        composition: "Ptest-root".into(),
        settings: settings(
            ExportFormat::PngStill { sixteen_bit: true },
            AlphaOutput::Straight,
        ),
        media: Arc::new(MediaPaths::default()),
        destination: wide_destination.clone(),
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 73,
    };
    run_export(
        &mut renderer,
        &wide_job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let (wide_info, wide_input) = pm_export::png::read(&wide_destination, true).unwrap();
    assert_eq!(wide_info.bit_depth, 16);
    let wide_input = wide_input.unwrap();
    let wide = wide_input.planes[0].u16().unwrap();
    let half_wide = &wide[((72 * W + 92) * 4) as usize..((72 * W + 92) * 4 + 4) as usize];
    assert_eq!(half_wide[3], u16::from(half[3]) * 257);
    for channel in 0..3 {
        assert!(
            half_wide[channel].abs_diff(u16::from(half[channel]) * 257) <= 128,
            "16-bit straight color agrees within half an 8-bit code at translucent pixel, channel {channel}"
        );
    }
    eprintln!("MEASURE depth8 sensitive raw16 half-alpha={half_wide:?}");
    for (name, codec, decoder) in [
        ("vp8", WebmCodec::Vp8, "libvpx"),
        ("vp9", WebmCodec::Vp9, "libvpx-vp9"),
    ] {
        let destination = scratch.path().join(format!("sensitive-{name}.webm"));
        let job = ExportJob {
            document: Arc::clone(&document),
            composition: "Ptest-root".into(),
            settings: settings(ExportFormat::Webm { codec }, AlphaOutput::Straight),
            media: Arc::new(MediaPaths::default()),
            destination: destination.clone(),
            overwrite: Overwrite::Fail,
            mode: Some(0o600),
            seed: 73,
        };
        run_export(
            &mut renderer,
            &job,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let info = video_info(&destination);
        assert!(info.contains("TAG:ALPHA_MODE=1"), "{name}: {info}");
        let decoded = video_pixels(&destination, "rgba", Some(decoder), false);
        let alpha_at = |x: u32, y: u32| decoded[((y * W + x) * 4 + 3) as usize];
        let samples = [alpha_at(127, 95), alpha_at(20, 20), alpha_at(92, 72)];
        eprintln!("MEASURE {name} libvpx decoded alpha transparent/opaque/half: {samples:?}");
        assert_eq!(samples[0], 0, "{name} transparent corner");
        assert_eq!(samples[1], 255, "{name} opaque patch");
        assert!(samples[2].abs_diff(128) <= 1, "{name} half-alpha patch");
    }
    let hevc_destination = scratch.path().join("sensitive-hevc-alpha.mov");
    let hevc_job = ExportJob {
        document: Arc::clone(&document),
        composition: "Ptest-root".into(),
        settings: settings(
            ExportFormat::Hevc {
                ten_bit: false,
                container: Container::Mov,
            },
            AlphaOutput::Straight,
        ),
        media: Arc::new(MediaPaths::default()),
        destination: hevc_destination.clone(),
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 73,
    };
    run_export(
        &mut renderer,
        &hevc_job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let hevc_rgba = video_pixels(&hevc_destination, "rgba", None, false);
    let hevc_alpha_at = |x: u32, y: u32| hevc_rgba[((y * W + x) * 4 + 3) as usize];
    let hevc_samples = [
        hevc_alpha_at(127, 95),
        hevc_alpha_at(20, 20),
        hevc_alpha_at(92, 72),
    ];
    eprintln!("MEASURE HEVC FFmpeg decoded alpha transparent/opaque/half: {hevc_samples:?}");
    assert_eq!(hevc_samples[0], 0, "HEVC transparent corner");
    assert_eq!(hevc_samples[1], 255, "HEVC opaque patch");
    assert!(hevc_samples[2].abs_diff(128) <= 1, "HEVC half-alpha patch");
    for (name, profile, alpha) in [
        ("prores-4444", ProResProfile::P4444, AlphaOutput::Straight),
        (
            "prores-4444xq",
            ProResProfile::P4444Xq,
            AlphaOutput::Premultiplied,
        ),
    ] {
        let destination = scratch.path().join(format!("sensitive-{name}.mov"));
        let job = ExportJob {
            document: Arc::clone(&document),
            composition: "Ptest-root".into(),
            settings: settings(ExportFormat::ProRes { profile }, alpha),
            media: Arc::new(MediaPaths::default()),
            destination: destination.clone(),
            overwrite: Overwrite::Fail,
            mode: Some(0o600),
            seed: 73,
        };
        run_export(
            &mut renderer,
            &job,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap();
        let decoded = video_pixels(&destination, "rgba", None, false);
        let alpha_at = |x: u32, y: u32| decoded[((y * W + x) * 4 + 3) as usize];
        let samples = [alpha_at(127, 95), alpha_at(20, 20), alpha_at(92, 72)];
        eprintln!("MEASURE {name} FFmpeg decoded alpha transparent/opaque/half: {samples:?}");
        assert_eq!(samples[0], 0, "{name} transparent corner");
        assert_eq!(samples[1], 255, "{name} opaque patch");
        assert!(samples[2].abs_diff(128) <= 1, "{name} half-alpha patch");
    }
}
