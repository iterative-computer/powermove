//! The transfer a delivered video is labelled with, and what the label does to
//! its codes, through the job engine's export pipeline: a Rec.709 (1-1-1)
//! ProRes clip shown untouched, exported to ProRes 422 HQ, H.264 and HEVC
//! Main10 as "Rec.709 (QuickTime and Safari)" (1-13-1, the default) and as
//! "Rec.709" (1-1-1). Either way the picture is the same codes as the source
//! (Standard reads transfers 1 and 13 as the same display encoding, so the
//! choice is a label, never a re-encoding); the label is the one chosen, in
//! the container's `colr` box and in the bitstream alike.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::convert::{EncoderInput, PlaneData, WorkingFrame};
use pm_export::mac::reader::decode_frame;
use pm_export::mp4box::Movie;
use pm_export::render::*;
use pm_export::timescale::video_timing;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, Container as WireContainer, ExportFormat, ExportSettings,
    ProResProfile as WireProRes, QualityTier,
};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::json;
use support::doc::*;

const W: u32 = 128;
const H: u32 = 64;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-tags-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A grey ramp across, colour bands below: 1-1-1, ProRes 422 HQ.
fn source(path: &Path) {
    let mut job = pm_export::export(ExportSpec {
        width: W,
        height: H,
        rate: Rate::FPS_24,
        frames: 2,
        codec: Codec::ProRes(ProResProfile::Hq),
        container: Container::Mov,
        tags: ColorTags::Rec709Broadcast,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: path.to_path_buf(),
        overwrite: Overwrite::Replace,
        mode: None,
    })
    .unwrap();
    let rgba: Vec<f32> = (0..H)
        .flat_map(|y| {
            (0..W).flat_map(move |x| {
                let t = x as f32 / (W - 1) as f32;
                let c = match y / 16 {
                    0 | 1 => [t, t, t],
                    2 => [0.8 * t + 0.1, 0.4, 0.2],
                    _ => [0.2, 0.5, 0.9 * t],
                };
                [c[0], c[1], c[2], 1.0]
            })
        })
        .collect();
    for _ in 0..2 {
        job.push_frame(&WorkingFrame {
            width: W,
            height: H,
            rgba: &rgba,
        })
        .unwrap();
    }
    job.finish().unwrap();
}

fn settings(format: ExportFormat, color: OutputColor) -> ExportSettings {
    ExportSettings {
        format,
        alpha: WireAlpha::None,
        quality: QualityTier::Max,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(24, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(TICKS_PER_SECOND / 24).unwrap(),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color,
        precision: Precision::Float32,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    }
}

/// The clip `path` full frame, untouched.
fn export(path: &Path, settings: ExportSettings, destination: PathBuf) {
    let dur = 2.0 / 24.0;
    let mut doc = project(
        W,
        H,
        dur,
        no_fill(),
        vec![layer(
            "v",
            "video",
            json!({ "asset": "clip", "fit": "stretch", "w": W, "h": H, "trim": 0, "speed": 1 }),
            f64::from(W) / 2.0,
            f64::from(H) / 2.0,
            dur,
        )],
    );
    doc["fps"] = json!(24);
    doc["colorDepth"] = json!(32);
    asset(&mut doc, "clip", "video", "mov", W, H, dur);
    let mut media = MediaPaths::default();
    media.insert("clip", path.to_path_buf());
    let job = ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination,
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 3,
    };
    let mut renderer = Renderer::headless().unwrap();
    run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{}: {e}", job.destination.display()));
}

/// Normalized Y′ of every pixel.
fn luma(input: &EncoderInput) -> Vec<f64> {
    match (&input.format, &input.planes[0].data) {
        (InputFormat::V216, PlaneData::U16(p)) => p
            .chunks_exact(4)
            .flat_map(|q| [q[1], q[3]])
            .map(|v| (f64::from(v) - 4096.0) / 56064.0)
            .collect(),
        (InputFormat::X420, PlaneData::U16(p)) => p
            .iter()
            .map(|&v| (f64::from(v >> 6) - 64.0) / 876.0)
            .collect(),
        (InputFormat::V420, PlaneData::U8(p)) => {
            p.iter().map(|&v| (f64::from(v) - 16.0) / 219.0).collect()
        }
        (f, _) => panic!("{f:?}"),
    }
}

/// `ffprobe`'s reading of the first video stream (`-show_streams`, the
/// container's `colr`) or its first decoded frame (`-show_frames`, the
/// bitstream's own tags).
fn ffprobe(path: &Path, entries: &str) -> String {
    let section = if entries == "stream" {
        "-show_streams"
    } else {
        "-show_frames"
    };
    let mut cmd = Command::new("ffprobe");
    cmd.args(["-v", "error", "-select_streams", "v:0", section]);
    if entries == "frame" {
        cmd.args(["-read_intervals", "%+#1"]);
    }
    let out = cmd.arg(path).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    ["color_primaries", "color_transfer", "color_space"]
        .map(|k| {
            text.lines()
                .find_map(|l| l.strip_prefix(&format!("{k}=")))
                .unwrap_or("absent")
                .to_owned()
        })
        .join("/")
}

#[test]
fn rec709_is_a_label_the_codes_of_an_untouched_clip_stay_the_same() {
    let dir = scratch("retag");
    let clip = dir.join("source.mov");
    source(&clip);
    let reference = luma(
        &decode_frame(
            &clip,
            0,
            video_timing(Rate::FPS_24).unwrap(),
            InputFormat::V216,
        )
        .unwrap(),
    );
    let formats = [
        (
            "prores",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            InputFormat::V216,
            // (codes per unit, worst, mean difference from the source):
            // ProRes 422 HQ of its own decode is exact.
            (876.0, 1.0, 0.1),
        ),
        // Lossy codecs: their loss at the bands' edges, the same under both.
        (
            "h264",
            ExportFormat::H264,
            InputFormat::V420,
            (219.0, 8.0, 0.75),
        ),
        (
            "hevc",
            ExportFormat::Hevc {
                ten_bit: true,
                container: WireContainer::Mov,
            },
            InputFormat::X420,
            (876.0, 4.0, 1.5),
        ),
    ];
    let choices = [
        (
            "1-13-1",
            OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            },
            13u16,
            "bt709/iec61966-2-1/bt709",
        ),
        (
            "1-1-1",
            OutputColor::Rec709Broadcast,
            1,
            "bt709/bt709/bt709",
        ),
    ];
    for (name, format, input, (scale, allowed, allowed_mean)) in formats {
        let mut pictures = Vec::new();
        for (label, color, transfer, probed) in choices {
            let out = dir.join(format!(
                "{name}-{label}.{}",
                if format == ExportFormat::H264 {
                    "mp4"
                } else {
                    "mov"
                }
            ));
            export(&clip, settings(format, color), out.clone());
            let decoded =
                luma(&decode_frame(&out, 0, video_timing(Rate::FPS_24).unwrap(), input).unwrap());
            // In codes of the output's depth.
            let differences: Vec<f64> = decoded
                .iter()
                .zip(&reference)
                .map(|(d, r)| ((d - r) * scale).abs())
                .collect();
            let worst = differences.iter().cloned().fold(0.0, f64::max);
            let mean = differences.iter().sum::<f64>() / differences.len() as f64;
            let colr = Movie::read(&out)
                .unwrap()
                .track(b"vide")
                .and_then(|t| t.visual.clone())
                .and_then(|v| v.colr)
                .map(|c| (c.primaries, c.transfer, c.matrix));
            let (stream, frame) = (ffprobe(&out, "stream"), ffprobe(&out, "frame"));
            eprintln!(
                "MEASURE {name} as {label}: luma {mean:.2} codes from the source on average, {worst:.2} at most; colr {colr:?}; ffprobe stream {stream}, frame {frame}"
            );
            assert!(
                worst <= allowed + 0.5 && mean <= allowed_mean,
                "{name} as {label}: luma {mean:.2} codes from the source's on average, {worst:.2} at most"
            );
            pictures.push(decoded);
            assert_eq!(colr, Some((1, transfer, 1)), "{name} as {label}: colr");
            assert_eq!(stream, probed, "{name} as {label}: container");
            assert_eq!(frame, probed, "{name} as {label}: bitstream");
        }
        assert!(
            pictures[0] == pictures[1],
            "{name}: the two labels carry different pictures"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
