//! Detailed picture is delivered, not refused.
//!
//! After an export the file is decoded and block means are held against
//! what the encoder was handed. A codec at its data-rate limit codes fine
//! detail coarsely, and a busy block's mean moves with the detail it
//! drops: every format must still deliver heavy noise and film-like grain
//! at HD, while a flat picture keeps the tight tolerance that catches a
//! file with the wrong levels.

#![cfg(target_os = "macos")]

use std::path::PathBuf;

use pm_export::convert::WorkingFrame;
use pm_export::verify::{ACTIVITY_TOLERANCE, pixel_tolerance};
use pm_export::*;
use pm_time::Rate;

const W: u32 = 1920;
const H: u32 = 1080;
const FRAMES: u64 = 5;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-detail-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A ramp with noise of `amount` on every pixel and channel, new each frame.
fn picture(amount: f32, frame: u64) -> Vec<f32> {
    let mut state = 0x9e37_79b9_7f4a_7c15u64 ^ (frame.wrapping_mul(0x2545_f491_4f6c_dd1d));
    let mut noise = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        ((state >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 2.0 * amount
    };
    (0..W * H)
        .flat_map(|i| {
            let ramp = (i % W) as f32 / W as f32;
            [
                0.3 + 0.4 * ramp + noise(),
                0.5 + noise(),
                0.7 - 0.4 * ramp + noise(),
                1.0,
            ]
        })
        .collect()
}

fn formats() -> Vec<(&'static str, Codec, Container)> {
    let mbps = |m: u32| m * 1_000_000;
    let mut out = vec![];
    for (name, profile) in [
        ("ProRes 422 Proxy", ProResProfile::Proxy),
        ("ProRes 422 LT", ProResProfile::Lt),
        ("ProRes 422", ProResProfile::Standard),
        ("ProRes 422 HQ", ProResProfile::Hq),
        ("ProRes 4444", ProResProfile::P4444),
        ("ProRes 4444 XQ", ProResProfile::P4444Xq),
    ] {
        out.push((name, Codec::ProRes(profile), Container::Mov));
    }
    out.extend([
        (
            "H.264 4 Mb/s",
            Codec::H264 {
                bitrate_bps: mbps(4),
            },
            Container::Mp4,
        ),
        (
            "H.264 16 Mb/s",
            Codec::H264 {
                bitrate_bps: mbps(16),
            },
            Container::Mp4,
        ),
        (
            "H.264 40 Mb/s",
            Codec::H264 {
                bitrate_bps: mbps(40),
            },
            Container::Mp4,
        ),
        (
            "HEVC 4 Mb/s",
            Codec::Hevc {
                bitrate_bps: mbps(4),
                ten_bit: false,
            },
            Container::Mp4,
        ),
        (
            "HEVC Main10 16 Mb/s",
            Codec::Hevc {
                bitrate_bps: mbps(16),
                ten_bit: true,
            },
            Container::Mp4,
        ),
        (
            "VP9 4 Mb/s",
            Codec::Vp9 {
                bitrate_bps: mbps(4),
            },
            Container::Webm,
        ),
        (
            "VP9 16 Mb/s",
            Codec::Vp9 {
                bitrate_bps: mbps(16),
            },
            Container::Webm,
        ),
    ]);
    out
}

#[test]
fn every_format_delivers_noise_and_grain() {
    let dir = scratch("noise");
    let mut problems = Vec::new();
    for (label, amount) in [("heavy noise", 0.3f32), ("grain", 0.04), ("flat", 0.0)] {
        let frames: Vec<Vec<f32>> = (0..FRAMES).map(|n| picture(amount, n)).collect();
        for (name, codec, container) in formats() {
            let what = format!("{name}, {label}");
            let mut job = pm_export::export(ExportSpec {
                width: W,
                height: H,
                rate: Rate::new(25, 1).unwrap(),
                frames: FRAMES,
                codec,
                container,
                tags: ColorTags::Rec709QuickTime,
                alpha: AlphaOutput::None,
                working: WorkingEncoding::Standard,
                dither: Dither::Default,
                audio: None,
                timecode: None,
                destination: dir.join(format!("out.{}", container.extension())),
                overwrite: Overwrite::Replace,
                mode: None,
            })
            .unwrap();
            for rgba in &frames {
                job.push_frame(&WorkingFrame {
                    width: W,
                    height: H,
                    rgba,
                })
                .unwrap();
            }
            match job.finish() {
                Ok(delivered) => {
                    let base = pixel_tolerance(codec);
                    let (mut worst, mut ratio, mut spread) = (0.0f64, 0.0f64, 0.0f64);
                    for p in &delivered.report.pixels {
                        worst = worst.max(p.error_cv);
                        spread = spread.max(p.spread_cv);
                        if p.spread_cv > 0.5 {
                            ratio = ratio.max((p.error_cv - base).max(0.0) / p.spread_cv);
                        }
                    }
                    eprintln!(
                        "MEASURE {what}: delivered, {:.0} Mb/s; block means within {worst:.2} codes (flat tolerance {base}), block spread up to {spread:.0} codes; beyond the flat tolerance by {ratio:.3} of the spread at most (allowed {ACTIVITY_TOLERANCE})",
                        delivered.report.bytes as f64 * 8.0 * 25.0 / FRAMES as f64 / 1e6
                    );
                    // A flat picture has no spread to lean on.
                    if amount == 0.0 && worst > base {
                        problems.push(format!("{what}: {worst:.2} codes on flat picture"));
                    }
                }
                Err(e) => problems.push(format!("{what}: {e}")),
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    let _ = std::fs::remove_dir_all(&dir);
}
