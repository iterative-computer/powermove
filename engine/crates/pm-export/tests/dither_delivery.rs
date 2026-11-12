//! Dither as the app asks for it (`ExportSettings::dither` true, the Export
//! sheet's setting), through the job engine's export pipeline: a shallow grey
//! gradient (0.40 to 0.42 across 1,920 columns, from a 12-bit ProRes 4444 XQ
//! clip) exported to every format, decoded, and measured for banding (the
//! worst 16-column window of column means against the ideal ramp, in codes)
//! and bitrate.
//!
//! The rule is the engine's measured one (EXPORT-DITHER-1): 8-bit PNG is
//! dithered, which removes its banding; 8-bit lossy codecs are not (the noise
//! costs bitrate and the encoder smears it); 10-bit and deeper outputs are
//! not. "Dither" on means that rule, so an H.264, VP9, HEVC or ProRes export
//! with it on is the export with it off.

#![cfg(target_os = "macos")]

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::convert::WorkingFrame;
use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{
    AlphaOutput as WireAlpha, Container as WireContainer, ExportFormat, ExportSettings,
    ProResProfile as WireProRes, QualityTier, WebmCodec,
};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::json;
use support::doc::*;

const W: u32 = 1920;
const H: u32 = 64;
const FRAMES: u64 = 12;

fn ramp(x: u32) -> f64 {
    0.40 + 0.02 * f64::from(x) / f64::from(W - 1)
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-dither-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn source(path: &Path) {
    let mut job = pm_export::export(ExportSpec {
        width: W,
        height: H,
        rate: Rate::FPS_24,
        frames: FRAMES,
        codec: Codec::ProRes(ProResProfile::P4444Xq),
        container: Container::Mov,
        tags: ColorTags::Rec709QuickTime,
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
        .flat_map(|_| {
            (0..W).flat_map(|x| {
                let v = ramp(x) as f32;
                [v, v, v, 1.0]
            })
        })
        .collect();
    for _ in 0..FRAMES {
        job.push_frame(&WorkingFrame {
            width: W,
            height: H,
            rgba: &rgba,
        })
        .unwrap();
    }
    job.finish().unwrap();
}

fn export(clip: &Path, format: ExportFormat, dither: bool, destination: &Path) {
    let dur = FRAMES as f64 / 24.0;
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
    media.insert("clip", clip.to_path_buf());
    let image = format.is_image();
    let settings = ExportSettings {
        format,
        alpha: WireAlpha::None,
        quality: QualityTier::High,
        size: Size::new(W, H),
        frame_rate: FrameRate::new(24, 1),
        range: TickRange {
            start: Ticks::ZERO,
            end: Ticks::new(FRAMES as i64 * TICKS_PER_SECOND / 24).unwrap(),
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
        precision: Precision::Float32,
        dither,
        draft_from_proxies: false,
        delivery: Default::default(),
    };
    let job = ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings,
        media: Arc::new(media),
        destination: destination.to_path_buf(),
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
    .unwrap_or_else(|e| panic!("{}: {e}", destination.display()));
}

/// The first frame's luma plane as coded (Y′CbCr) or its green (RGB),
/// decoded by FFmpeg, in codes of the output's depth, with the ideal ramp's
/// codes.
fn decoded(path: &Path, ycbcr: bool, ten_bit: bool) -> (Vec<f64>, Box<dyn Fn(f64) -> f64>) {
    // Planar 4:2:0 output: its first plane is the coded Y′, copied.
    let pix = match (ycbcr, ten_bit) {
        (true, false) => "yuv420p",
        (true, true) => "yuv420p10le",
        (false, _) => "rgb24",
    };
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            pix,
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let b = out.stdout;
    let n = (W * H) as usize;
    let values: Vec<f64> = match pix {
        "yuv420p" => b[..n].iter().map(|&v| f64::from(v)).collect(),
        "yuv420p10le" => b[..2 * n]
            .chunks_exact(2)
            .map(|p| f64::from(u16::from_le_bytes([p[0], p[1]])))
            .collect(),
        _ => b.chunks_exact(3).map(|p| f64::from(p[1])).collect(),
    };
    let ideal: Box<dyn Fn(f64) -> f64> = match (ycbcr, ten_bit) {
        (true, false) => Box::new(|v| v * 219.0 + 16.0),
        (true, true) => Box::new(|v| v * 876.0 + 64.0),
        (false, _) => Box::new(|v| v * 255.0),
    };
    (values, ideal)
}

/// Worst 16-column window of column means against the ideal, in codes.
fn banding(values: &[f64], ideal: &dyn Fn(f64) -> f64) -> f64 {
    let (w, h) = (W as usize, values.len() / W as usize);
    let column: Vec<f64> = (0..w)
        .map(|x| (0..h).map(|y| values[y * w + x]).sum::<f64>() / h as f64)
        .collect();
    (0..w - 16)
        .map(|x0| {
            let got = column[x0..x0 + 16].iter().sum::<f64>() / 16.0;
            let want = (x0..x0 + 16).map(|x| ideal(ramp(x as u32))).sum::<f64>() / 16.0;
            (got - want).abs()
        })
        .fold(0.0, f64::max)
}

fn size(path: &Path) -> u64 {
    if path.is_dir() {
        std::fs::read_dir(path)
            .unwrap()
            .map(|e| e.unwrap().metadata().unwrap().len())
            .sum()
    } else {
        std::fs::metadata(path).unwrap().len()
    }
}

#[test]
fn dither_follows_the_measured_rule_for_every_format() {
    let dir = scratch("rule");
    let clip = dir.join("ramp.mov");
    source(&clip);
    // (name, format, file, Y′CbCr, 10-bit, dithered)
    let cases = [
        ("h264", ExportFormat::H264, "mp4", true, false, false),
        (
            "vp9",
            ExportFormat::Webm {
                codec: WebmCodec::Vp9,
            },
            "webm",
            true,
            false,
            false,
        ),
        (
            "hevc10",
            ExportFormat::Hevc {
                ten_bit: true,
                container: WireContainer::Mp4,
            },
            "mp4",
            true,
            true,
            false,
        ),
        (
            "prores422hq",
            ExportFormat::ProRes {
                profile: WireProRes::Hq,
            },
            "mov",
            true,
            true,
            false,
        ),
        (
            "png8",
            ExportFormat::PngStill { sixteen_bit: false },
            "png",
            false,
            false,
            true,
        ),
    ];
    for (name, format, ext, ycbcr, ten_bit, dithered) in cases {
        let mut measured = Vec::new();
        for dither in [false, true] {
            let out = dir.join(format!("{name}-{dither}.{ext}"));
            export(&clip, format, dither, &out);
            let (values, ideal) = decoded(&out, ycbcr, ten_bit);
            let bps = size(&out) as f64 * 8.0 * 24.0 / FRAMES as f64;
            measured.push((banding(&values, &*ideal), bps, values));
        }
        let [(off_band, off_bps, off), (on_band, on_bps, on)] =
            <[_; 2]>::try_from(measured).unwrap_or_else(|_| unreachable!());
        eprintln!(
            "MEASURE dither {name}: off banding {off_band:.3} codes, {:.0} kb/s; on banding {on_band:.3} codes, {:.0} kb/s",
            off_bps / 1000.0,
            on_bps / 1000.0
        );
        if dithered {
            assert!(on_band < 0.1, "{name}: dithered banding {on_band}");
            assert!(off_band > 0.4, "{name}: undithered banding {off_band}");
        } else {
            assert!(
                on == off,
                "{name}: dither on changed the picture (banding {off_band:.3} → {on_band:.3}, {:.0} → {:.0} kb/s)",
                off_bps / 1000.0,
                on_bps / 1000.0
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}
