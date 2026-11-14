//! A clip's picture and sound leave an export together.
//!
//! Each source holds one white frame and one chirp authored at the same
//! presented time (0.6 s into the file), with the soundtrack starting
//! somewhere else than the picture: a quarter second late (QuickTime), three
//! frames early and less than a frame early (Matroska presents sound before
//! zero as authored). The clip, placed at 0, is exported through the job
//! engine to ProRes with PCM, and the file is decoded by FFmpeg: the white
//! frame and the chirp must sit where the asset's reading places them
//! (docs/native/p0-decisions.md, clip timing), together for `timing: 2`.
//! An older project (no member) hears the program sample its edits
//! address, so a late soundtrack stays where the earlier app played it.

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
    AlphaOutput as WireAlpha, ExportFormat, ExportSettings, ProResProfile as WireProRes,
    QualityTier,
};
use pm_render::Renderer;
use serde_json::json;
use support::doc::*;

const RATE: u32 = 48_000;
const FPS: u32 = 25;
/// The white frame and the chirp, in seconds of the file's presentation.
const EVENT: f64 = 0.6;

fn ffmpeg() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(|d| PathBuf::from(d).join("bin/ffmpeg"))
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

fn run(args: &[&std::ffi::OsStr]) -> Vec<u8> {
    let out = Command::new(ffmpeg())
        .args(["-hide_banner", "-loglevel", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// A 20 ms chirp (500 Hz to 4 kHz) under a raised-cosine window, half scale.
fn chirp() -> Vec<f32> {
    let n = (RATE / 50) as usize;
    (0..n)
        .map(|i| {
            let t = i as f64 / f64::from(RATE);
            let sweep = 500.0 * t + 3500.0 * t * t / (2.0 * n as f64 / f64::from(RATE));
            let window = 0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / n as f64).cos();
            (0.5 * window * (std::f64::consts::TAU * sweep).sin()) as f32
        })
        .collect()
}

/// Two and a half seconds of mono 16-bit sound with the chirp at `at`.
fn write_wav(path: &Path, at: usize) {
    let mut mono = vec![0f32; RATE as usize * 5 / 2];
    for (i, v) in chirp().into_iter().enumerate() {
        mono[at + i] = v;
    }
    let data: Vec<u8> = mono
        .iter()
        .flat_map(|&v| ((f64::from(v) * 32767.0).round() as i16).to_le_bytes())
        .collect();
    let mut wav = b"RIFF".to_vec();
    wav.extend((36 + data.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend([1, 0, 1, 0]);
    wav.extend(RATE.to_le_bytes());
    wav.extend((RATE * 2).to_le_bytes());
    wav.extend([2, 0, 16, 0]);
    wav.extend(b"data");
    wav.extend((data.len() as u32).to_le_bytes());
    wav.extend(data);
    std::fs::write(path, wav).unwrap();
}

/// Two seconds of black 25 fps picture, white at frame 15 (0.6 s), with a
/// soundtrack starting at `sound_at` whose chirp is presented at 0.6 s.
fn source(dir: &Path, name: &str, sound_at: f64) -> PathBuf {
    let wav = dir.join(format!("{name}.wav"));
    write_wav(
        &wav,
        ((EVENT - sound_at) * f64::from(RATE)).round() as usize,
    );
    let path = dir.join(name);
    let offset = sound_at.to_string();
    let picture = "color=black:size=64x36:rate=25:duration=2,format=yuv420p,\
        drawbox=c=white:t=fill:enable='eq(n,15)'";
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "-f".as_ref(),
        "lavfi".as_ref(),
        "-i".as_ref(),
        picture.as_ref(),
        "-itsoffset".as_ref(),
        offset.as_ref(),
        "-i".as_ref(),
        wav.as_os_str(),
    ];
    for a in [
        "-map",
        "0:v",
        "-map",
        "1:a",
        "-c:v",
        "mpeg4",
        "-q:v",
        "2",
        "-g",
        "1",
        "-c:a",
        "pcm_s16le",
        "-avoid_negative_ts",
        "disabled",
    ] {
        args.push(a.as_ref());
    }
    args.push(path.as_os_str());
    run(&args);
    path
}

/// The exported clip of `path`, read with `timing` (the asset member).
fn export(dir: &Path, path: &Path, timing: Option<u64>) -> PathBuf {
    let dur = 2.0;
    let video = layer(
        "vid",
        "video",
        json!({ "asset": "clip", "fit": "cover", "w": 64, "h": 36, "trim": 0, "speed": 1,
            "embeddedAudio": true }),
        32.0,
        18.0,
        dur,
    );
    let mut doc = project(64, 36, dur, solid_fill("#000000"), vec![video]);
    doc["fps"] = json!(FPS);
    asset(&mut doc, "clip", "video", "mov", 64, 36, dur);
    doc["assets"]["clip"]["fps"] = json!(FPS);
    doc["assets"]["clip"]["hasAudio"] = json!(true);
    if let Some(t) = timing {
        doc["assets"]["clip"]["timing"] = json!(t);
    }
    let mut media = MediaPaths::default();
    media.insert("clip", path.to_path_buf());
    let name = path.file_name().unwrap().to_string_lossy();
    let destination = dir.join(format!("{name}-{}.mov", timing.unwrap_or(0)));
    let job = ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings: ExportSettings {
            format: ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            alpha: WireAlpha::None,
            quality: QualityTier::Draft,
            size: Size::new(64, 36),
            frame_rate: FrameRate::new(FPS, 1),
            range: TickRange {
                start: Ticks::ZERO,
                end: Ticks::new(TICKS_PER_SECOND * 2).unwrap(),
            },
            audio: true,
            audio_index_playhead: None,
            motion_blur: false,
            color: OutputColor::Rec709QuickTime {
                candidate: QuickTimeTag::Cicp1_13_1,
            },
            precision: Precision::Float16,
            dither: false,
            draft_from_proxies: false,
            delivery: Default::default(),
        },
        media: Arc::new(media),
        destination: destination.clone(),
        overwrite: Overwrite::Replace,
        mode: None,
        seed: 1,
    };
    let mut renderer = Renderer::headless().unwrap();
    run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_or_else(|e| panic!("{name}: {e}"));
    destination
}

/// The exported frames that are white.
fn white_frames(path: &Path) -> Vec<usize> {
    let gray = run(&[
        "-i".as_ref(),
        path.as_os_str(),
        "-map".as_ref(),
        "0:v".as_ref(),
        "-f".as_ref(),
        "rawvideo".as_ref(),
        "-pix_fmt".as_ref(),
        "gray".as_ref(),
        "pipe:1".as_ref(),
    ]);
    gray.chunks_exact(64 * 36)
        .enumerate()
        .filter(|(_, f)| f.iter().map(|&v| u32::from(v)).sum::<u32>() / (64 * 36) > 128)
        .map(|(i, _)| i)
        .collect()
}

/// Where the chirp starts in the exported sound: the sample of the largest
/// correlation.
fn chirp_at(path: &Path) -> usize {
    let raw = run(&[
        "-i".as_ref(),
        path.as_os_str(),
        "-map".as_ref(),
        "0:a".as_ref(),
        "-ac".as_ref(),
        "1".as_ref(),
        "-f".as_ref(),
        "f32le".as_ref(),
        "pipe:1".as_ref(),
    ]);
    let got: Vec<f32> = raw
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    let burst = chirp();
    (0..got.len().saturating_sub(burst.len()))
        .map(|at| {
            let score: f64 = burst
                .iter()
                .zip(&got[at..])
                .map(|(b, g)| f64::from(*b) * f64::from(*g))
                .sum();
            (at, score)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap()
        .0
}

#[test]
fn a_clips_picture_and_sound_leave_the_export_together() {
    let dir = std::env::temp_dir().join(format!("pm-export-clip-sync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let timing = Some(pm_media::clip::ASSET_TIMING);
    // (file, sound start, reading, where the white frame and the chirp sit
    // in the export, in seconds)
    let cases: [(&str, f64, Option<u64>, f64, f64); 4] = [
        // The picture is the origin: both at 0.6 s.
        ("late-sound.mov", 0.25, timing, EVENT, EVENT),
        // The sound, three frames early, is the origin: both 0.12 s later.
        ("early-sound.mkv", -0.12, timing, EVENT + 0.12, EVENT + 0.12),
        // The sound less than a frame early gives way to the picture.
        ("sound-a-frame-early.mkv", -0.02, timing, EVENT, EVENT),
        // An older project hears program sample t at source time t: the
        // chirp where the earlier app played it, a quarter second early.
        ("late-sound.mov", 0.25, None, EVENT, EVENT - 0.25),
    ];
    for (name, sound_at, reading, picture, sound) in cases {
        let file = dir.join(name);
        if !file.exists() {
            source(&dir, name, sound_at);
        }
        let exported = export(&dir, &file, reading);
        let what = format!("{name}, timing {reading:?}");
        let frames = white_frames(&exported);
        let heard = chirp_at(&exported);
        eprintln!("MEASURE {what}: white frames {frames:?}, chirp at sample {heard}");
        let frame = (picture * f64::from(FPS)).round() as usize;
        assert_eq!(frames, [frame], "{what}: the white frame");
        let sample = (sound * f64::from(RATE)).round() as i64;
        assert!(
            (heard as i64 - sample).abs() <= 1,
            "{what}: the chirp at sample {heard}, {sample} expected"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
