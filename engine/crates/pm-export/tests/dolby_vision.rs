//! What an HLG HEVC export carries of Dolby Vision today: the sample
//! entry's boxes and codec, and whether each frame has an RPU, read by an
//! independent parser (FFmpeg's).

#![cfg(target_os = "macos")]

mod support;

use std::path::Path;
use std::process::Command;

use pm_export::convert::WorkingFrame;
use pm_export::*;
use pm_time::Rate;

const W: u32 = 1280;
const H: u32 = 720;
const FRAMES: u64 = 12;

/// The four-character codes of every box in `bytes`, depth first.
fn boxes(bytes: &[u8], out: &mut Vec<String>) {
    let mut i = 0;
    while i + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let kind = String::from_utf8_lossy(&bytes[i + 4..i + 8]).into_owned();
        if size < 8 || i + size > bytes.len() {
            break;
        }
        out.push(kind.clone());
        let body = &bytes[i + 8..i + size];
        match kind.as_str() {
            "moov" | "trak" | "mdia" | "minf" | "stbl" => boxes(body, out),
            // A sample description: version, flags, count, then entries.
            "stsd" if body.len() > 8 => boxes(&body[8..], out),
            // A visual sample entry: 78 bytes before its own boxes.
            "hvc1" | "hev1" | "dvh1" | "dvhe" if body.len() > 78 => boxes(&body[78..], out),
            _ => {}
        }
        i += size;
    }
}

fn hlg_hevc(path: &Path, dolby_vision: bool) -> Delivered {
    let mut job = pm_export::export(ExportSpec {
        width: W,
        height: H,
        rate: Rate::new(24, 1).unwrap(),
        frames: FRAMES,
        codec: Codec::Hevc {
            bitrate_bps: 20_000_000,
            ten_bit: true,
        },
        container: Container::Mov,
        tags: ColorTags::Rec2100Hlg {
            white_nits: 203,
            peak_nits: 1000,
            metadata: None,
            dolby_vision,
        },
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
    for n in 0..FRAMES {
        // A bright moving highlight over a dim ramp.
        let rgba: Vec<f32> = (0..H)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .flat_map(|(x, y)| {
                let ramp = 0.1 + 0.3 * x as f32 / W as f32;
                let spot = if (x as i64 - 200 - 60 * n as i64).abs() < 80 && y.abs_diff(H / 2) < 80
                {
                    1.0
                } else {
                    0.0
                };
                let v = ramp + spot;
                [v, v * 0.9, v * 0.8, 1.0]
            })
            .collect();
        job.push_frame(&WorkingFrame {
            width: W,
            height: H,
            rgba: &rgba,
        })
        .unwrap();
    }
    job.finish().unwrap()
}

/// Frames with a Dolby Vision RPU, as FFmpeg reads the file.
fn frames_with_rpu(path: &Path) -> (usize, usize) {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_frames",
            "-select_streams",
            "v",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    let frames: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let frames = frames["frames"].as_array().unwrap();
    let with = frames
        .iter()
        .filter(|f| {
            f["side_data_list"].as_array().is_some_and(|l| {
                l.iter()
                    .any(|s| s["side_data_type"] == "Dolby Vision RPU Data")
            })
        })
        .count();
    (frames.len(), with)
}

/// FFmpeg's reading of the stream: its tag, transfer, and Dolby Vision
/// configuration record.
fn stream(path: &Path) -> serde_json::Value {
    let out = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_streams",
            "-select_streams",
            "v",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["streams"][0].clone()
}

/// Every frame's 10-bit planes, as FFmpeg decodes the HLG base layer.
fn base_layer(path: &Path) -> Vec<u16> {
    let out = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "rawvideo", "-pix_fmt", "yuv420p10le", "-"])
        .output()
        .unwrap();
    out.stdout
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect()
}

/// An HLG HEVC export carries Dolby Vision Profile 8.4 (macOS's encoder
/// computes the RPUs; the file check holds the record), and none when the
/// person turns it off; the HLG stream players without Dolby Vision show is
/// tagged and coded alike either way.
#[test]
fn hlg_hevc_carries_dolby_vision_8_4_unless_turned_off() {
    let dir = std::env::temp_dir().join(format!("pm-export-dv-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (on, off) = (dir.join("on.mov"), dir.join("off.mov"));
    let delivered_on = hlg_hevc(&on, true);
    let delivered_off = hlg_hevc(&off, false);

    // The file check's own reading.
    let record = delivered_on
        .report
        .dolby_vision
        .expect("a Dolby Vision record");
    assert_eq!(
        (
            record.profile,
            record.compatibility,
            record.rpu,
            record.enhancement_layer
        ),
        (8, 4, true, false)
    );
    assert_eq!(delivered_off.report.dolby_vision, None);
    let mut found = vec![];
    boxes(&std::fs::read(&on).unwrap(), &mut found);
    assert!(found.iter().any(|b| b == "dvvC"), "{found:?}");
    found.clear();
    boxes(&std::fs::read(&off).unwrap(), &mut found);
    assert!(
        !found.iter().any(|b| b == "dvvC" || b == "dvcC"),
        "{found:?}"
    );

    // FFmpeg's: the record, an RPU on every frame, and the HLG base layer.
    let (s_on, s_off) = (stream(&on), stream(&off));
    let record = &s_on["side_data_list"][0];
    assert_eq!(record["side_data_type"], "DOVI configuration record");
    assert_eq!(
        (
            &record["dv_profile"],
            &record["dv_bl_signal_compatibility_id"],
            &record["rpu_present_flag"],
            &record["el_present_flag"]
        ),
        (
            &serde_json::json!(8),
            &serde_json::json!(4),
            &serde_json::json!(1),
            &serde_json::json!(0)
        )
    );
    assert!(
        s_off["side_data_list"].is_null(),
        "{}",
        s_off["side_data_list"]
    );
    for s in [&s_on, &s_off] {
        assert_eq!(s["codec_tag_string"], "hvc1");
        assert_eq!(s["color_transfer"], "arib-std-b67");
        assert_eq!(s["color_primaries"], "bt2020");
    }
    assert_eq!(frames_with_rpu(&on), (FRAMES as usize, FRAMES as usize));
    assert_eq!(frames_with_rpu(&off), (FRAMES as usize, 0));
    let (a, b) = (base_layer(&on), base_layer(&off));
    assert_eq!(a.len(), b.len());
    let differing = a.iter().zip(&b).filter(|(x, y)| x != y).count();
    let worst = a
        .iter()
        .zip(&b)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0);
    eprintln!(
        "MEASURE HLG base layer, Dolby Vision on against off: {differing} of {} samples differ, by at most {worst} codes",
        a.len()
    );

    // Apple's reading (AVFoundation's format description).
    let apple = pm_export::mac::reader::video_extensions(&on).unwrap();
    eprintln!("MEASURE AVFoundation extensions with Dolby Vision on: {apple:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The composition's frames drawing `clip` full frame, as an export hands
/// them to its encoder.
fn drawn(clip: &Path) -> Vec<pm_export::convert::EncoderInput> {
    use pm_export::pipeline::test_sinks::MemorySink;
    use pm_export::render::*;
    use pm_protocol::color::{OutputColor, Precision, QuickTimeTag};
    use pm_protocol::common::{FrameRate, Size, TICKS_PER_SECOND, TickRange, Ticks};
    use pm_protocol::priority::{
        AlphaOutput as WireAlpha, ExportFormat, ExportSettings, QualityTier,
    };
    use serde_json::json;
    use support::doc::*;
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
    asset(&mut doc, "clip", "video", "mov", W, H, dur);
    let mut media = MediaPaths::default();
    media.insert("clip", clip.to_path_buf());
    let job = ExportJob {
        document: std::sync::Arc::new(load(&doc)),
        composition: "Ptest-root".into(),
        settings: ExportSettings {
            format: ExportFormat::H264,
            alpha: WireAlpha::None,
            quality: QualityTier::Max,
            size: Size::new(W, H),
            frame_rate: FrameRate::new(24, 1),
            range: TickRange {
                start: Ticks::ZERO,
                end: Ticks::new(FRAMES as i64 * TICKS_PER_SECOND / 24).unwrap(),
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
        media: std::sync::Arc::new(media),
        destination: "/unused.mp4".into(),
        overwrite: Overwrite::Fail,
        mode: None,
        seed: 1,
    };
    let plan = plan_export(&job).unwrap();
    let mut renderer = pm_render::Renderer::headless().unwrap();
    run_export_into(
        &mut renderer,
        &job,
        plan,
        MemorySink::default(),
        &Unrestricted,
        &std::sync::atomic::AtomicBool::new(false),
        &mut |_| {},
    )
    .unwrap()
    .frames
}

/// A Dolby Vision 8.4 clip (as an iPhone or an HLG export writes it) reads
/// as its HLG base layer: probed as HLG BT.2020 with Dolby Vision 8.4, so
/// named in Interpret Footage and the Project panel, and drawn exactly as
/// the same HLG picture without the metadata.
#[test]
fn a_dolby_vision_8_4_clip_reads_as_its_hlg_base_layer() {
    let dir = std::env::temp_dir().join(format!("pm-export-dv-in-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (on, off) = (dir.join("on.mov"), dir.join("off.mov"));
    hlg_hevc(&on, true);
    hlg_hevc(&off, false);
    let probed = pm_media::probe::probe(&on).unwrap().probe;
    let video = probed.video.as_ref().unwrap();
    assert_eq!(
        video.color.hdr,
        Some(pm_media::probe::color::HdrFormat::DolbyVision {
            profile: 8,
            level: 1,
            base_compatibility: 4
        })
    );
    assert_eq!(
        video.color.hdr.as_ref().unwrap().label(),
        "Dolby Vision 8.4"
    );
    assert_eq!(
        video.color.interpreted.transfer,
        pm_protocol::color::Transfer::Hlg
    );
    assert_eq!(
        video.color.interpreted.primaries,
        pm_protocol::color::Primaries::Bt2020
    );
    let facts = pm_media::facts::media_facts(&on, None).unwrap();
    assert!(
        facts
            .facts
            .iter()
            .any(|f| f.label == "HDR" && f.value.starts_with("Dolby Vision profile 8.4")),
        "{:?}",
        facts.facts
    );
    let (a, b) = (drawn(&on), drawn(&off));
    assert_eq!(a.len(), FRAMES as usize);
    for (n, (x, y)) in a.iter().zip(&b).enumerate() {
        assert!(
            x.planes == y.planes,
            "frame {n} differs from the same HLG picture without Dolby Vision"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
