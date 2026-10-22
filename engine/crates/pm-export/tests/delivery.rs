//! Phase 3b delivery through the job engine's pipeline (plan section 17): every
//! format at cutover from a generated project (render, mix, encode, mux, verify,
//! move), the delivery options (audio coding, start timecode and reel, captions
//! left out for a caption file, live previews), offline media detection
//! (section 14.8), and export safety (section 14.6): an export after preview
//! renders at Half and Quarter in 16-bit float is byte-identical to one from an
//! empty cache.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use pm_export::pipeline::test_sinks::MemorySink;
use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, AudioCoding, Container as WireContainer, DeliveryOptions,
    ExportFormat, ExportSettings, ExportTimecode, ProResProfile as WireProRes, QualityTier,
    WebmCodec,
};
use pm_render::Renderer;
use serde_json::{Value, json};
use support::doc::*;

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../apps/desktop/e2e/fixtures")
        .join(name)
}

fn scratch(name: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("pm-export-delivery-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn seconds(s: f64) -> Ticks {
    Ticks::new((s * TICKS_PER_SECOND as f64).round() as i64).unwrap()
}

const W: u32 = 320;
const H: u32 = 180;

/// Moving solids over a fading background, a PNG with alpha, an H.264 clip,
/// and a WAV tone; with `captions`, a captions layer on top.
fn project_with(dur: f64, captions: bool) -> (Value, MediaPaths) {
    let mut layers = vec![];
    if captions {
        layers.push(layer(
            "subs",
            "captions",
            json!({ "cues": [{ "id": "c1", "start": 0, "end": dur, "text": "Hello" }] }),
            160.0,
            150.0,
            dur,
        ));
    }
    let mut mover = layer(
        "mover",
        "solid",
        json!({ "color": "#E83A5E", "w": 90, "h": 60 }),
        40.0,
        60.0,
        dur,
    );
    set(
        &mut mover,
        "position.x",
        keys(json!(40), &[(0.0, json!(40)), (dur, json!(260))]),
    );
    layers.push(mover);
    layers.push(audio("tone", "tone", 0.25, dur));
    layers.push(layer(
        "img",
        "image",
        json!({ "asset": "still", "fit": "contain", "w": 80, "h": 45 }),
        250.0,
        40.0,
        dur,
    ));
    layers.push(layer(
        "vid",
        "video",
        json!({ "asset": "clip", "fit": "cover", "w": 120, "h": 68, "trim": 0, "speed": 1, "embeddedAudio": false }),
        90.0,
        120.0,
        dur,
    ));
    let mut back = layer(
        "back",
        "solid",
        json!({ "color": "#2A3350", "w": 240, "h": 140 }),
        160.0,
        90.0,
        dur,
    );
    set(
        &mut back,
        "opacity",
        keys(json!(100), &[(0.0, json!(100)), (dur, json!(30))]),
    );
    layers.push(back);
    let mut doc = project(W, H, dur, no_fill(), layers);
    asset(&mut doc, "still", "image", "png", 160, 90, 0.0);
    asset(&mut doc, "clip", "video", "mp4", 320, 180, 2.0);
    asset(&mut doc, "tone", "audio", "wav", 0, 0, 2.0);
    let mut media = MediaPaths::default();
    media.insert("still", fixture("still-alpha.png"));
    media.insert("clip", fixture("h264-aac.mp4"));
    media.insert("tone", fixture("tone.wav"));
    (doc, media)
}

fn settings(format: ExportFormat, alpha: WireAlpha, dur: f64) -> ExportSettings {
    let image = format.is_image();
    ExportSettings {
        format,
        alpha,
        quality: QualityTier::High,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(30, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: seconds(dur),
        },
        audio: !image,
        motion_blur: false,
        color: if image {
            OutputColor::SrgbImage
        } else {
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            }
        },
        precision: Precision::Float32,
        dither: !matches!(format, ExportFormat::ProRes { .. })
            && !matches!(
                format,
                ExportFormat::Hevc { ten_bit: true, .. }
                    | ExportFormat::PngSequence { sixteen_bit: true }
                    | ExportFormat::PngStill { sixteen_bit: true }
            ),
        draft_from_proxies: false,
        delivery: DeliveryOptions::default(),
    }
}

fn job(
    doc: &Value,
    media: MediaPaths,
    settings: ExportSettings,
    destination: PathBuf,
) -> ExportJob {
    ExportJob {
        document: Arc::new(load(doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination,
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 11,
    }
}

fn export_to(renderer: &mut Renderer, job: &ExportJob) -> Delivered {
    run_export(
        renderer,
        job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{}: {e}", job.destination.display()))
}

/// Every format at cutover, from one project, verified by the exporter.
#[test]
fn every_format_at_cutover_exports_and_verifies() {
    let dur = 1.0;
    let (doc, media) = project_with(dur, false);
    let dir = scratch("formats");
    let mut renderer = Renderer::headless().unwrap();
    let prores = |profile| ExportFormat::ProRes { profile };
    let hevc = |ten_bit, container| ExportFormat::Hevc { ten_bit, container };
    let cases: Vec<(&str, ExportFormat, WireAlpha, &str, &str, u32, &str)> = vec![
        (
            "proxy",
            prores(WireProRes::Proxy),
            WireAlpha::None,
            "mov",
            "apco",
            10,
            "4:2:2",
        ),
        (
            "lt",
            prores(WireProRes::Lt),
            WireAlpha::None,
            "mov",
            "apcs",
            10,
            "4:2:2",
        ),
        (
            "422",
            prores(WireProRes::Standard),
            WireAlpha::None,
            "mov",
            "apcn",
            10,
            "4:2:2",
        ),
        (
            "hq",
            prores(WireProRes::Hq),
            WireAlpha::None,
            "mov",
            "apch",
            10,
            "4:2:2",
        ),
        (
            "4444",
            prores(WireProRes::P4444),
            WireAlpha::Straight,
            "mov",
            "ap4h",
            12,
            "4:4:4:4",
        ),
        (
            "4444pm",
            prores(WireProRes::P4444),
            WireAlpha::Premultiplied,
            "mov",
            "ap4h",
            12,
            "4:4:4:4",
        ),
        (
            "xq",
            prores(WireProRes::P4444Xq),
            WireAlpha::None,
            "mov",
            "ap4x",
            12,
            "4:4:4",
        ),
        (
            "xqa",
            prores(WireProRes::P4444Xq),
            WireAlpha::Straight,
            "mov",
            "ap4x",
            12,
            "4:4:4:4",
        ),
        (
            "h264",
            ExportFormat::H264,
            WireAlpha::None,
            "mp4",
            "avc1",
            8,
            "4:2:0",
        ),
        (
            "hevc8",
            hevc(false, WireContainer::Mp4),
            WireAlpha::None,
            "mp4",
            "hvc1",
            8,
            "4:2:0",
        ),
        (
            "hevc10",
            hevc(true, WireContainer::Mp4),
            WireAlpha::None,
            "mp4",
            "hvc1",
            10,
            "4:2:0",
        ),
        (
            "hevc8mov",
            hevc(false, WireContainer::Mov),
            WireAlpha::None,
            "mov",
            "hvc1",
            8,
            "4:2:0",
        ),
        (
            "hevc10mov",
            hevc(true, WireContainer::Mov),
            WireAlpha::None,
            "mov",
            "hvc1",
            10,
            "4:2:0",
        ),
        (
            "hevca",
            hevc(false, WireContainer::Mov),
            WireAlpha::Straight,
            "mov",
            "hvc1",
            8,
            "4:2:0:4",
        ),
        (
            "vp9",
            ExportFormat::Webm {
                codec: WebmCodec::Vp9,
            },
            WireAlpha::None,
            "webm",
            "vp09",
            8,
            "4:2:0",
        ),
        (
            "vp9a",
            ExportFormat::Webm {
                codec: WebmCodec::Vp9,
            },
            WireAlpha::Straight,
            "webm",
            "vp09",
            8,
            "4:2:0:4",
        ),
        (
            "vp8",
            ExportFormat::Webm {
                codec: WebmCodec::Vp8,
            },
            WireAlpha::None,
            "webm",
            "vp08",
            8,
            "4:2:0",
        ),
        (
            "vp8a",
            ExportFormat::Webm {
                codec: WebmCodec::Vp8,
            },
            WireAlpha::Straight,
            "webm",
            "vp08",
            8,
            "4:2:0:4",
        ),
        (
            "seq8",
            ExportFormat::PngSequence { sixteen_bit: false },
            WireAlpha::None,
            "",
            "png",
            8,
            "RGB",
        ),
        (
            "seq16a",
            ExportFormat::PngSequence { sixteen_bit: true },
            WireAlpha::Straight,
            "",
            "png",
            16,
            "RGBA",
        ),
        (
            "still8a",
            ExportFormat::PngStill { sixteen_bit: false },
            WireAlpha::Straight,
            "png",
            "png",
            8,
            "RGBA",
        ),
        (
            "still16",
            ExportFormat::PngStill { sixteen_bit: true },
            WireAlpha::None,
            "png",
            "png",
            16,
            "RGB",
        ),
    ];
    for (name, format, alpha, ext, codec, depth, pixel_format) in cases {
        let destination = if ext.is_empty() {
            dir.join(name)
        } else {
            dir.join(format!("{name}.{ext}"))
        };
        let j = job(
            &doc,
            media.clone(),
            settings(format, alpha, dur),
            destination.clone(),
        );
        let started = std::time::Instant::now();
        let delivered = export_to(&mut renderer, &j);
        let r = &delivered.report;
        eprintln!(
            "MEASURE {name}: {} {} {}-bit {} frames, {} bytes, max block error {:.3} CV, {:.0} ms",
            r.codec,
            r.pixel_format,
            r.depth,
            r.frames,
            r.bytes,
            r.max_pixel_error_cv(),
            started.elapsed().as_secs_f64() * 1e3
        );
        assert_eq!(delivered.path, destination, "{name}");
        assert_eq!(
            (r.codec.as_str(), r.depth, r.pixel_format.as_str()),
            (codec, depth, pixel_format),
            "{name}"
        );
        assert_eq!(r.alpha, alpha != WireAlpha::None, "{name}");
        let still = matches!(format, ExportFormat::PngStill { .. });
        assert_eq!(r.frames, if still { 1 } else { 30 }, "{name}");
        if format.is_image() {
            assert_eq!(r.cicp, (1, 13, 0), "{name}");
            assert!(r.audio.is_none());
        } else {
            assert_eq!(
                r.cicp,
                // Today's WebM tracks leave primaries/transfer unspecified,
                // and VP8 also has no bitstream matrix.
                match codec {
                    "vp08" if alpha == WireAlpha::None => (2, 2, 2),
                    _ if ext == "webm" => (2, 2, 1),
                    _ => (1, 13, 1),
                },
                "{name}"
            );
            assert!(!r.full_range, "{name}");
            let audio = r.audio.as_ref().expect("audio");
            // round(duration × sr): the tone starts at 0.25 s; the track spans the export.
            // One second at 48 kHz (the project rate, and Opus's).
            assert_eq!(
                audio.samples,
                if ext == "webm" { 48_648 } else { 48_000 },
                "{name}: {audio:?}"
            );
            // ProRes MOV's 24-bit PCM goes through the desktop exporter's
            // FFmpeg mux, which writes QuickTime's `in24` sample entry; HEVC
            // keeps AVAssetWriter's `lpcm`.
            let want = match ext {
                "mov" if codec == "hvc1" => "lpcm",
                "mov" => "in24",
                "mp4" => "mp4a",
                _ => "opus",
            };
            assert_eq!(audio.codec, want, "{name}");
        }
        if ext == "mov" {
            let tc = r.timecode.as_ref().expect("MOV exports carry timecode");
            assert_eq!(tc.start_frame, 0, "{name}");
        }
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Audio codings, a chosen start timecode and reel, and previews.
#[test]
fn delivery_options_reach_the_file() {
    let dur = 1.0;
    let (doc, media) = project_with(dur, false);
    let dir = scratch("options");
    let mut renderer = Renderer::headless().unwrap();
    for (coding, bits, float, codec) in [
        (AudioCoding::Pcm16, 16, false, "lpcm"),
        (AudioCoding::Pcm24, 24, false, "in24"),
        (AudioCoding::PcmFloat, 32, true, "lpcm"),
        (AudioCoding::Aac, 0, false, "mp4a"),
    ] {
        let mut s = settings(
            ExportFormat::ProRes {
                profile: WireProRes::Lt,
            },
            WireAlpha::None,
            dur,
        );
        s.frame_rate = FrameRate::new(30000, 1001);
        s.delivery = DeliveryOptions {
            audio_coding: Some(coding),
            timecode: Some(ExportTimecode {
                start: "01:00:00;00".into(),
                drop_frame: true,
                reel: Some("A001".into()),
            }),
            preview: true,
            ..DeliveryOptions::default()
        };
        let j = job(&doc, media.clone(), s, dir.join(format!("{coding:?}.mov")));
        let previews = Mutex::new(vec![]);
        let delivered = run_export(
            &mut renderer,
            &j,
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |p: Progress| {
                if let Some(v) = p.preview {
                    previews.lock().unwrap().push(v);
                }
            },
        )
        .unwrap();
        let r = &delivered.report;
        let a = r.audio.as_ref().unwrap();
        assert_eq!(a.codec, codec, "{coding:?}");
        if codec == "lpcm" || codec == "in24" {
            assert_eq!((a.bits_per_channel, a.float), (bits, float), "{coding:?}");
        }
        // round(30 × 1001/30000 × 48000) = 48048.
        assert_eq!(a.samples, 48_048, "{coding:?}");
        let tc = r.timecode.as_ref().unwrap();
        // 01:00:00;00 at 29.97 drop-frame is frame 107892.
        assert_eq!(
            (tc.start_frame, tc.drop_frame, tc.reel.as_deref()),
            (107_892, true, Some("A001"))
        );
        let previews = previews.lock().unwrap();
        assert!(!previews.is_empty(), "previews");
        let last = previews.last().unwrap();
        assert_eq!((last.frame, last.width, last.height), (29, 320, 180));
        assert_eq!(&last.jpeg[..2], &[0xFF, 0xD8]);
    }
    // Without the option, no previews are made.
    let j = job(
        &doc,
        media.clone(),
        settings(ExportFormat::H264, WireAlpha::None, dur),
        dir.join("none.mp4"),
    );
    let any = Mutex::new(false);
    run_export(
        &mut renderer,
        &j,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |p: Progress| {
            *any.lock().unwrap() |= p.preview.is_some();
        },
    )
    .unwrap();
    assert!(!*any.lock().unwrap());
    std::fs::remove_dir_all(dir).unwrap();
}

/// Captions burn in by default (and route while the renderer cannot draw
/// them); delivered as a caption file instead, they are left out and the
/// export runs in the engine.
#[test]
fn captions_left_out_for_a_caption_file() {
    let dur = 0.5;
    let (doc, media) = project_with(dur, true);
    let dir = scratch("captions");
    let s = settings(ExportFormat::H264, WireAlpha::None, dur);
    let burn = job(&doc, media.clone(), s.clone(), dir.join("burn.mp4"));
    match plan_export(&burn) {
        Err(JobError::Unsupported(list)) => {
            assert!(list.iter().any(|u| u.contains("captions")), "{list:?}")
        }
        Ok(_) => {
            // The renderer draws captions now: burning in is the default.
        }
        Err(e) => panic!("{e}"),
    }
    let mut hidden = s;
    hidden.delivery.hide_captions = true;
    let j = job(&doc, media.clone(), hidden, dir.join("clean.mp4"));
    let mut renderer = Renderer::headless().unwrap();
    let delivered = export_to(&mut renderer, &j);
    assert_eq!(delivered.report.frames, 15);
    // The captions layer draws nothing: the clean export equals the project
    // without it, frame for frame.
    let (plain, _) = project_with(dur, false);
    let reference = job(&plain, media, j.settings.clone(), dir.join("unused.mp4"));
    let a = run_export_into(
        &mut renderer,
        &j,
        plan_export(&j).unwrap(),
        MemorySink::default(),
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    let b = run_export_into(
        &mut renderer,
        &reference,
        plan_export(&reference).unwrap(),
        MemorySink::default(),
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap();
    assert!(a.frames == b.frames, "captions left out draw nothing");
    std::fs::remove_dir_all(dir).unwrap();
}

/// Section 14.8: the clips an export needs that are offline, through nested
/// compositions; assets nothing uses and assets without records do not count.
#[test]
fn offline_media_is_listed_by_clip() {
    let (mut doc, media) = project_with(0.5, false);
    let mut inner = layer(
        "nested-vid",
        "video",
        json!({ "asset": "far", "fit": "cover", "w": 64, "h": 36 }),
        32.0,
        18.0,
        0.5,
    );
    inner["name"] = json!("nested");
    composition(&mut doc, "inner", 64, 36, 0.5, vec![inner]);
    doc["layers"].as_array_mut().unwrap().push(layer(
        "pre",
        "precomp",
        json!({ "comp": "inner", "w": 64, "h": 36 }),
        10.0,
        10.0,
        0.5,
    ));
    asset(&mut doc, "far", "video", "mov", 64, 36, 1.0);
    doc["assets"]["far"]["name"] = json!("A001_C003.mov");
    asset(&mut doc, "unused", "video", "mov", 64, 36, 1.0);
    let document = load(&doc);
    assert_eq!(
        offline_media(&document, "Ptest-root", &media),
        vec![OfflineClip {
            asset: "far".into(),
            name: "A001_C003.mov".into()
        }]
    );
    let mut fewer = media.clone();
    fewer.paths.remove("tone");
    let names: Vec<String> = offline_media(&document, "Ptest-root", &fewer)
        .into_iter()
        .map(|c| c.asset)
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.contains(&"tone".to_string()) && names.contains(&"far".to_string()));
}

/// Section 14.6: preview renders through the same renderer (Half and Quarter
/// at 16-bit float, sliced and whole) leave nothing an export reads: the export
/// after them equals one from a fresh renderer byte for byte, as encoder input
/// and as PNG files.
#[test]
fn exports_after_preview_renders_equal_empty_cache_exports() {
    let dur = 0.5;
    let (doc, media) = project_with(dur, false);
    let dir = scratch("safety");
    let document = Arc::new(load(&doc));
    let s = settings(
        ExportFormat::ProRes {
            profile: WireProRes::Hq,
        },
        WireAlpha::None,
        dur,
    );
    let mut j = job(&doc, media.clone(), s, dir.join("unused.mov"));
    j.document = document.clone();
    let encoder_input = |renderer: &mut Renderer| {
        run_export_into(
            renderer,
            &j,
            plan_export(&j).unwrap(),
            MemorySink::default(),
            &Unrestricted,
            &AtomicBool::new(false),
            &mut |_| {},
        )
        .unwrap()
        .frames
    };
    let mut fresh = Renderer::headless().unwrap();
    let empty_cache = encoder_input(&mut fresh);
    let mut warmed = Renderer::headless().unwrap();
    let mut routing = StillRouting::new();
    for (edge, n) in [(160u32, 0u64), (80, 3), (160, 7), (80, 14), (320, 9)] {
        let t = Ticks::new(n as i64 * TICKS_PER_SECOND / 30).unwrap();
        render_still(
            &mut warmed,
            &document,
            "Ptest-root",
            t,
            edge,
            &j.media,
            &mut routing,
            &Unrestricted,
            1,
            &AtomicBool::new(false),
        )
        .unwrap();
    }
    let after_previews = encoder_input(&mut warmed);
    assert_eq!(empty_cache.len(), 15);
    assert!(
        empty_cache == after_previews,
        "the export after preview renders differs from an empty-cache export"
    );
    // PNG files, byte for byte.
    let mut png = settings(
        ExportFormat::PngSequence { sixteen_bit: true },
        WireAlpha::Straight,
        dur,
    );
    png.delivery.sequence_name = Some("frame".into());
    let a = job(&doc, media.clone(), png.clone(), dir.join("fresh"));
    let b = job(&doc, media, png, dir.join("warm"));
    let mut fresh = Renderer::headless().unwrap();
    let pa = export_to(&mut fresh, &a).path;
    let pb = export_to(&mut warmed, &b).path;
    for n in 0..15 {
        let name = format!("frame_{n:05}.png");
        assert_eq!(
            std::fs::read(pa.join(&name)).unwrap(),
            std::fs::read(pb.join(&name)).unwrap(),
            "{name}"
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}
