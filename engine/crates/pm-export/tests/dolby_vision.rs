//! What an HLG HEVC export carries of Dolby Vision today: the sample
//! entry's boxes and codec, and whether each frame has an RPU, read by an
//! independent parser (FFmpeg's).

#![cfg(target_os = "macos")]

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
