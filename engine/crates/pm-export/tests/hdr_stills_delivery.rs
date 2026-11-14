//! A still with a gain map delivers its HDR picture: exported from an HDR
//! project to BT.2100 PQ, the file holds the light the map describes (the
//! fixture's right half is three times SDR white, 609 cd/m²); exported from
//! a Standard project to Rec.709, it holds the picture tone mapped to SDR
//! (that half is the content peak, so SDR white). Read back by FFmpeg.

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

const W: u32 = 64;
const H: u32 = 36;

fn ffmpeg() -> PathBuf {
    std::env::var_os("PM_FFMPEG_DIR")
        .map(|d| PathBuf::from(d).join("bin/ffmpeg"))
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

/// The first frame's signal at the middle of the left and right halves,
/// 0…1. The picture is neutral grey, so R′ = G′ = B′ = Y′: read from the
/// file's 10-bit limited-range Y′ as FFmpeg decodes it, no matrix applied
/// (FFmpeg's conversion to RGB moves it by up to three codes).
fn halves(path: &Path) -> [f64; 2] {
    let out = Command::new(ffmpeg())
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-frames:v",
            "1",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "yuv444p16le",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    [8usize, 56].map(|x| {
        let i = (18 * W as usize + x) * 2;
        let y = f64::from(u16::from_le_bytes([out.stdout[i], out.stdout[i + 1]])) / 64.0;
        (y - 64.0) / 876.0
    })
}

fn export(dir: &Path, hdr: bool, format: ExportFormat, color: OutputColor, name: &str) -> PathBuf {
    let still = layer(
        "still",
        "image",
        json!({ "asset": "clip", "w": W, "h": H }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        0.2,
    );
    let mut doc = project(W, H, 0.2, solid_fill("#000000"), vec![still]);
    asset(&mut doc, "clip", "image", "heic", W, H, 0.2);
    if hdr {
        doc["color"] = json!({ "mode": "hdr", "masteringPeak": 1000 });
        doc["colorDepth"] = json!(32);
    }
    let mut media = MediaPaths::default();
    media.insert(
        "clip",
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hdr-stills/still-gainmap.heic"),
    );
    let destination = dir.join(name);
    let job = ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings: ExportSettings {
            format,
            alpha: WireAlpha::None,
            quality: QualityTier::High,
            size: Size::new(W, H),
            frame_rate: FrameRate::new(25, 1),
            range: TickRange {
                start: Ticks::ZERO,
                end: Ticks::new(TICKS_PER_SECOND / 5).unwrap(),
            },
            audio: false,
            audio_index_playhead: None,
            motion_blur: false,
            color,
            precision: Precision::Float32,
            dither: false,
            draft_from_proxies: false,
            delivery: Default::default(),
        },
        media: Arc::new(media),
        destination: destination.clone(),
        overwrite: Overwrite::Replace,
        mode: None,
        seed: 7,
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

#[test]
fn a_gain_map_still_delivers_its_hdr_picture() {
    let dir = std::env::temp_dir().join(format!("pm-hdr-still-delivery-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // HDR: PQ codes back to cd/m².
    let pq = export(
        &dir,
        true,
        ExportFormat::ProRes {
            profile: WireProRes::Hq,
        },
        OutputColor::Rec2100Pq,
        "pq.mov",
    );
    let [left, right] = halves(&pq);
    let nits = pm_color::tonemap::pq_eotf;
    eprintln!(
        "MEASURE pq: left {} right {} cd/m²",
        nits(left),
        nits(right)
    );
    // Within one 10-bit code of the signal (PQ is steep there: a code is
    // 1.3 % at 600 cd/m²).
    for (got, want) in [(left, 36.8), (right, 609.0)] {
        let code = pm_color::tonemap::pq_inverse_eotf(want);
        assert!(
            (got - code).abs() < 1.0 / 876.0,
            "PQ delivery: {} cd/m², {want} expected",
            nits(got)
        );
    }
    // SDR: the content peak is SDR white; 18 % grey stays 18 % grey.
    let sdr = export(
        &dir,
        false,
        ExportFormat::ProRes {
            profile: WireProRes::Hq,
        },
        OutputColor::Rec709QuickTime {
            candidate: QuickTimeTag::Cicp1_13_1,
        },
        "sdr.mov",
    );
    let [left, right] = halves(&sdr);
    eprintln!("MEASURE sdr: left {left} right {right}");
    let grey = pm_color::transfer::display_encode(0.18);
    assert!((left - grey).abs() < 1.0 / 876.0, "{left}");
    assert!((right - 1.0).abs() < 1.0 / 876.0, "{right}");
    let _ = std::fs::remove_dir_all(&dir);
}
