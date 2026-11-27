//! Section 2's time bar through the job engine's export pipeline: an export
//! burns in a binary frame code (24 solid squares whose opacity expressions
//! read the composition time), 10,000 consecutive frames per rate plus
//! 100-frame windows at N = 10⁶ and 10⁷, and every decoded frame must carry
//! exactly the frame number its position says; the `tmcd` track names the
//! window's first frame in the rate's timecode (drop-frame at 29.97 and
//! 59.94).
//!
//! `time_bar_short` runs 600 frames per rate on every `cargo test`;
//! `time_bar_full` is the gate's 10,000 (`--ignored`, about 100,000 frames).

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::Overwrite;
use pm_export::render::{
    ExportJob, MediaPaths, Timeline, Unrestricted, run_export, start_timecode,
};
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput, ExportFormat, ExportSettings, ProResProfile, QualityTier,
};
use pm_render::Renderer;
use pm_time::{Rate, TimecodeFormat};
use serde_json::{Value, json};
use support::doc::*;

const W: u32 = 64;
const H: u32 = 32;
const BITS: u32 = 24;
const SQUARE: u32 = 8;

/// The rates of the bar (29.97 counts drop-frame, the timeline's default).
const RATES: &[(u32, u32)] = &[
    (24000, 1001),
    (24, 1),
    (25, 1),
    (30000, 1001),
    (48000, 1001),
    (60000, 1001),
    (60, 1),
    (120000, 1001),
    (240000, 1001),
];

fn ffmpeg_dir() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap())
                .join("Library/Caches/powermove-engine/ffmpeg/out")
        })
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-time-bar-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Square `k` (bit `k` of the frame number) is white when the bit is set.
fn bit_layer(k: u32, num: u32, den: u32, dur: f64) -> Value {
    let x = f64::from((k % (W / SQUARE)) * SQUARE);
    let y = f64::from((k / (W / SQUARE)) * SQUARE);
    let mut l = layer(
        &format!("bit{k}"),
        "solid",
        json!({ "color": "#FFFFFF", "w": SQUARE, "h": SQUARE }),
        x,
        y,
        dur,
    );
    let pow = 2u64.pow(k);
    l["p"]["opacity"] = json!({
        "v": 100,
        "kf": [],
        "expr": format!("Math.floor(Math.floor(time * {num} / {den} + 0.5) / {pow}) % 2 * 100")
    });
    l
}

fn frame_code_project(num: u32, den: u32, dur: f64) -> Value {
    let layers = (0..BITS).map(|k| bit_layer(k, num, den, dur)).collect();
    let mut doc = project(W, H, dur, solid_fill("#000000"), layers);
    doc["fps"] = json!(f64::from(num) / f64::from(den));
    doc["fpsRational"] = json!([num, den]);
    doc
}

fn ticks_of_frame(n: u64, num: u32, den: u32) -> Ticks {
    let t = i128::from(n) * i128::from(den) * i128::from(TICKS_PER_SECOND);
    let num = i128::from(num);
    // Round half up onto the tick grid (exact for rates that divide the flick).
    Ticks::new(((2 * t + num) / (2 * num)) as i64).unwrap()
}

/// Exports frames `[first, first + count)` and returns the decoded codes and
/// the `tmcd` timecode ffprobe reads.
fn export_window(dir: &Path, num: u32, den: u32, first: u64, count: u64) -> (Vec<u64>, String) {
    let end_frame = first + count;
    let dur = (end_frame as f64 + 2.0) * f64::from(den) / f64::from(num);
    let doc = frame_code_project(num, den, dur);
    let settings = ExportSettings {
        format: ExportFormat::ProRes {
            profile: ProResProfile::Proxy,
        },
        alpha: AlphaOutput::None,
        quality: QualityTier::High,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(num, den),
        range: TickRange {
            start: ticks_of_frame(first, num, den),
            end: ticks_of_frame(end_frame, num, den),
        },
        audio: false,
        audio_index_playhead: None,
        motion_blur: false,
        color: OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        precision: Precision::Float32,
        dither: false,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    let destination = dir.join(format!("code-{num}-{den}-{first}.mov"));
    let job = ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(MediaPaths::default()),
        destination,
        overwrite: Overwrite::Fail,
        mode: Some(0o600),
        seed: 1,
    };
    let mut renderer = Renderer::headless().unwrap();
    let delivered = run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{num}/{den} at {first}: {e}"));
    assert_eq!(delivered.report.frames, count, "{num}/{den} at {first}");
    let bin = ffmpeg_dir().join("bin");
    let raw = Command::new(bin.join("ffmpeg"))
        .args(["-v", "error", "-i"])
        .arg(&delivered.path)
        .args(["-f", "rawvideo", "-pix_fmt", "gray", "-"])
        .output()
        .unwrap();
    assert!(
        raw.status.success(),
        "{}",
        String::from_utf8_lossy(&raw.stderr)
    );
    let frame_bytes = (W * H) as usize;
    assert_eq!(
        raw.stdout.len(),
        frame_bytes * count as usize,
        "{num}/{den} at {first}"
    );
    let codes = raw
        .stdout
        .chunks(frame_bytes)
        .map(|f| {
            (0..BITS).fold(0u64, |code, k| {
                let x = (k % (W / SQUARE)) * SQUARE + SQUARE / 2;
                let y = (k / (W / SQUARE)) * SQUARE + SQUARE / 2;
                let v = f[(y * W + x) as usize];
                assert!(
                    !(40..=200).contains(&v),
                    "{num}/{den} at {first}: bit {k} reads {v}"
                );
                code | (u64::from(v > 128) << k)
            })
        })
        .collect();
    let probe = Command::new(bin.join("ffprobe"))
        .args([
            "-v",
            "error",
            "-show_entries",
            "stream_tags=timecode",
            "-of",
            "csv=p=0",
        ])
        .arg(&delivered.path)
        .output()
        .unwrap();
    let timecode = String::from_utf8_lossy(&probe.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_owned();
    std::fs::remove_file(&delivered.path).unwrap();
    (codes, timecode)
}

fn time_bar(per_rate: u64) {
    let dir = scratch(&per_rate.to_string());
    let mut checked = 0u64;
    for &(num, den) in RATES {
        let rate = Rate::new(i64::from(num), i64::from(den)).unwrap();
        for (first, count) in [(0, per_rate), (1_000_000, 100), (10_000_000, 100)] {
            let (codes, timecode) = export_window(&dir, num, den, first, count);
            for (i, code) in codes.iter().enumerate() {
                assert_eq!(
                    *code,
                    first + i as u64,
                    "{num}/{den}: frame {i} of the window at {first}"
                );
            }
            checked += codes.len() as u64;
            let timeline = Timeline::new(
                ticks_of_frame(first, num, den),
                ticks_of_frame(first + count, num, den),
                rate,
            )
            .unwrap();
            let expected = TimecodeFormat::for_rate(rate);
            let start = start_timecode(&timeline);
            assert_eq!(start.format, expected);
            assert_eq!(
                start.start_frame,
                (first as i64).rem_euclid(expected.frames_per_day())
            );
            assert_eq!(
                timecode,
                expected.format(start.start_frame),
                "{num}/{den} at {first}"
            );
        }
    }
    eprintln!("time bar: {checked} frames carried their own frame code");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn time_bar_short() {
    time_bar(600);
}

#[test]
#[ignore = "gate: 10,000 frames per rate (about 100,000 frames); run with --ignored"]
fn time_bar_full() {
    time_bar(10_000);
}

/// The vectors of section 2's time bar, as the export's `tmcd` labels them.
#[test]
fn drop_frame_vectors_label_tmcd_starts() {
    let r2997 = Rate::new(30000, 1001).unwrap();
    let r5994 = Rate::new(60000, 1001).unwrap();
    let label = |rate: Rate, frame: u64| {
        let t = Timeline::new(
            ticks_of_frame(frame, rate.num() as u32, rate.den() as u32),
            ticks_of_frame(frame + 1, rate.num() as u32, rate.den() as u32),
            rate,
        )
        .unwrap();
        let s = start_timecode(&t);
        s.format.format(s.start_frame)
    };
    assert_eq!(label(r2997, 1799), "00:00:59;29");
    assert_eq!(label(r2997, 1800), "00:01:00;02");
    assert_eq!(label(r2997, 17982), "00:10:00;00");
    assert_eq!(label(r2997, 2_589_408), "00:00:00;00");
    assert_eq!(label(r5994, 3600), "00:01:00;04");
    assert_eq!(label(r5994, 35964), "00:10:00;00");
    let r23976 = Rate::new(24000, 1001).unwrap();
    assert_eq!(label(r23976, 86_400), "01:00:00:00");
}
