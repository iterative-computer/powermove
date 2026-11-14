//! A source frame that cannot be read stops the export, and the person is
//! told where: the export's frame, the clip, the file and where in it.
//!
//! Nothing stands in for the frame: a picture the export cannot make is
//! certain, and a frame quietly frozen or left black would be delivered as
//! if it were the composition's.

#![cfg(target_os = "macos")]

mod support;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use pm_export::convert::WorkingFrame;
use pm_export::render::*;
use pm_export::*;
use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
use pm_protocol::priority::{AlphaOutput as WireAlpha, ExportFormat, ExportSettings, QualityTier};
use pm_render::Renderer;
use pm_time::Rate;
use serde_json::json;
use support::doc::*;

const W: u32 = 320;
const H: u32 = 180;
/// Frames of the source clip, and the one that is damaged.
const SOURCE: u64 = 12;
const DAMAGED: usize = 7;

/// A 12-frame ProRes clip with frame 7's header zeroed (as a bad copy or a
/// failing card leaves a frame): the decoder rejects that frame alone.
fn damaged_clip(dir: &std::path::Path) -> PathBuf {
    let path = dir.join("interview.mov");
    let mut job = pm_export::export(ExportSpec {
        width: W,
        height: H,
        rate: Rate::new(24, 1).unwrap(),
        frames: SOURCE,
        codec: Codec::ProRes(ProResProfile::Hq),
        container: Container::Mov,
        tags: ColorTags::Rec709QuickTime,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination: path.clone(),
        overwrite: Overwrite::Replace,
        mode: None,
    })
    .unwrap();
    for n in 0..SOURCE {
        let v = 0.2 + 0.05 * n as f32;
        let rgba = [v, 0.5, 1.0 - v, 1.0].repeat((W * H) as usize);
        job.push_frame(&WorkingFrame {
            width: W,
            height: H,
            rgba: &rgba,
        })
        .unwrap();
    }
    job.finish().unwrap();
    let mut bytes = std::fs::read(&path).unwrap();
    // Each ProRes frame starts with its size and "icpf".
    let frames: Vec<usize> = bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| w == b"icpf")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(frames.len(), SOURCE as usize);
    bytes[frames[DAMAGED]..frames[DAMAGED] + 64].fill(0);
    std::fs::write(&path, bytes).unwrap();
    path
}

#[test]
fn a_damaged_source_frame_stops_the_export_and_says_where() {
    let dir = std::env::temp_dir().join(format!("pm-export-damaged-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = damaged_clip(&dir);
    // One second at 24 fps; the clip from half a second on, so export
    // frame 19 (00:00:00:19) reads the clip's frame 7 (0.292 s into it).
    let dur = 1.0;
    let mut clip = layer(
        "Linterview",
        "video",
        json!({ "asset": "clip", "fit": "stretch", "w": W, "h": H, "trim": 0, "speed": 1 }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        0.5,
    );
    clip["name"] = json!("Interview");
    clip["from"] = json!(0.5);
    let mut doc = project(W, H, dur, no_fill(), vec![clip]);
    doc["fps"] = json!(24);
    asset(&mut doc, "clip", "video", "mov", W, H, 0.5);
    doc["assets"]["clip"]["name"] = json!("interview.mov");
    let mut media = MediaPaths::default();
    media.insert("clip", source);
    let destination = dir.join("out.mov");
    let job = ExportJob {
        document: Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings: ExportSettings {
            format: ExportFormat::ProRes {
                profile: pm_protocol::priority::ProResProfile::Hq,
            },
            alpha: WireAlpha::None,
            quality: QualityTier::Max,
            size: Size::new(W, H),
            frame_rate: FrameRate::new(24, 1),
            range: TickRange {
                start: Ticks::ZERO,
                end: Ticks::new(TICKS_PER_SECOND).unwrap(),
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
        },
        media: Arc::new(media),
        destination: destination.clone(),
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 1,
    };
    let mut renderer = Renderer::headless().unwrap();
    let error = run_export(
        &mut renderer,
        &job,
        &Unrestricted,
        &AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap_err()
    .to_string();
    for part in [
        "00:00:00:19",
        "“Interview”",
        "interview.mov",
        "00:00:00.292",
    ] {
        assert!(error.contains(part), "{part} in: {error}");
    }
    assert!(!error.contains("cannot render this yet"), "{error}");
    assert!(!destination.exists(), "no file is delivered");
    let _ = std::fs::remove_dir_all(&dir);
}
