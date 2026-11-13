//! A delivered file's sound ends with its picture: through the job engine's
//! export pipeline, a two-second composition with a tone is exported at
//! 23.976, 29.97, 25 and 60 fps to H.264 and HEVC (MP4, AAC) and ProRes 422
//! HQ and 4444 (MOV, PCM), and `ffprobe` reads both streams' durations from
//! the container: the audio lasts the frames' duration (frames ÷ rate) to
//! within one sample.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, Container as WireContainer, ExportFormat, ExportSettings,
    ProResProfile as WireProRes, QualityTier,
};
use pm_render::Renderer;
use serde_json::{Value, json};
use support::doc::*;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-avlen-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Each stream's (codec type, duration in seconds) as the container says.
fn streams(path: &Path) -> Vec<(String, f64)> {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream=codec_type,duration",
            "-of",
            "csv=p=0",
        ])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| {
            let (kind, duration) = l.split_once(',')?;
            Some((kind.to_owned(), duration.parse().ok()?))
        })
        .collect()
}

fn document(fps: f64, dur: f64) -> Value {
    let mut doc = project(
        64,
        36,
        dur,
        solid_fill("#14161C"),
        vec![audio("tone", "tone", 0.0, dur)],
    );
    doc["fps"] = json!(fps);
    asset(&mut doc, "tone", "audio", "wav", 0, 0, dur);
    doc
}

#[test]
fn sound_lasts_as_long_as_the_frames_at_every_rate() {
    let dir = scratch("rates");
    let dur = 2.0;
    let formats = [
        ("h264", ExportFormat::H264, "mp4"),
        (
            "hevc",
            ExportFormat::Hevc {
                ten_bit: true,
                container: WireContainer::Mp4,
            },
            "mp4",
        ),
        (
            "prores422",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            "mov",
        ),
        (
            "prores4444",
            ExportFormat::ProRes {
                profile: WireProRes::P4444,
            },
            "mov",
        ),
    ];
    let mut off = Vec::new();
    for (num, den) in [(24000u32, 1001u32), (30000, 1001), (25, 1), (60, 1)] {
        for (name, format, ext) in formats {
            let fps = f64::from(num) / f64::from(den);
            let mut media = MediaPaths::default();
            media.insert("tone", repo_root().join("engine/fixtures/tone.wav"));
            let destination = dir.join(format!("{name}-{num}-{den}.{ext}"));
            let job = ExportJob {
                document: Arc::new(load(&document(fps, dur))),
                composition: "Ptest-root".into(),
                settings: ExportSettings {
                    format,
                    alpha: WireAlpha::None,
                    quality: QualityTier::Draft,
                    size: Size::new(64, 36),
                    frame_rate: FrameRate::new(num, den),
                    range: TickRange {
                        start: Ticks::ZERO,
                        end: Ticks::new(2 * TICKS_PER_SECOND).unwrap(),
                    },
                    audio: true,
                    audio_index_playhead: None,
                    motion_blur: false,
                    color: OutputColor::Rec709QuickTime {
                        candidate: QuickTimeTag::Cicp1_13_1,
                    },
                    precision: Precision::Float16,
                    dither: true,
                    draft_from_proxies: false,
                    delivery: Default::default(),
                },
                media: Arc::new(media),
                destination: destination.clone(),
                overwrite: Overwrite::Fail,
                mode: None,
                seed: 1,
            };
            let mut renderer = Renderer::headless().unwrap();
            let frames = plan_export(&job).unwrap().timeline.frames;
            run_export(
                &mut renderer,
                &job,
                &Unrestricted,
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap_or_else(|e| panic!("{name} at {fps}: {e}"));
            let s = streams(&destination);
            let duration = |kind: &str| {
                s.iter()
                    .find(|(k, _)| k == kind)
                    .map(|(_, d)| *d)
                    .unwrap_or_else(|| panic!("{name} at {fps}: no {kind} stream in {s:?}"))
            };
            let picture = frames as f64 * f64::from(den) / f64::from(num);
            let (video, sound) = (duration("video"), duration("audio"));
            let samples = (sound - picture) * 48_000.0;
            eprintln!(
                "MEASURE {name} at {fps:.3} fps: {frames} frames = {picture:.6} s; video stream {video:.6} s, audio {sound:.6} s ({samples:+.1} samples)"
            );
            if (video - picture).abs() > 0.5 / 48_000.0 || samples.abs() > 1.0 {
                off.push(format!(
                    "{name} at {fps:.3}: video {video:.6} s, audio {sound:.6} s, frames {picture:.6} s"
                ));
            }
        }
    }
    assert!(off.is_empty(), "{off:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}
