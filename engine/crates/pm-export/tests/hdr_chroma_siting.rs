//! Where 4:2:0 chroma sits in a delivered file, and what the stream says.
//!
//! BT.2100 (PQ and HLG) 4:2:0 chroma is co-sited with the top-left luma
//! sample of each 2 × 2 block (`chroma_sample_loc_type` 2); Rec.709 4:2:0 is
//! co-sited across and centred between rows (type 0, the codecs' default).
//! A one-row saturated rule tells the two apart: at an even row a top-left
//! sample carries half of it and a centred one three eighths.
//!
//! Checked on the encoder input (exact), on the file decoded by
//! VideoToolbox, and on the software encoder's file, with the location read
//! from the stream's own SPS.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use pm_export::convert::{
    Chroma, ConvertParams, EncoderInput, WorkingFrame, convert_tagged, sample_ycbcra,
};
use pm_export::mac::reader::decode_frame;
use pm_export::mp4box::Movie;
use pm_export::pipeline::Pipeline;
use pm_export::pipeline::test_sinks::MemorySink;
use pm_export::timescale::video_timing;
use pm_export::*;
use pm_time::Rate;

const W: usize = 64;
const H: usize = 32;
const FRAMES: u64 = 4;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-siting-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Black with a saturated one-row rule: at an even row in even frames, at
/// the odd row below it in odd frames.
fn rule_row(n: u64) -> usize {
    12 + (n % 2) as usize
}

fn picture(n: u64) -> Vec<f32> {
    (0..H)
        .flat_map(|y| (0..W).map(move |_| y))
        .flat_map(|y| {
            if y == rule_row(n) {
                [0.9, 0.05, 0.05, 1.0]
            } else {
                [0.0, 0.0, 0.0, 1.0]
            }
        })
        .collect()
}

fn pq() -> ColorTags {
    ColorTags::Rec2100Pq {
        white_nits: 203,
        metadata: None,
    }
}

fn hlg() -> ColorTags {
    ColorTags::Rec2100Hlg {
        white_nits: 203,
        peak_nits: 1000,
        metadata: None,
        dolby_vision: true,
    }
}

fn spec(destination: PathBuf, tags: ColorTags) -> ExportSpec {
    ExportSpec {
        width: W as u32,
        height: H as u32,
        rate: Rate::FPS_24,
        frames: FRAMES,
        codec: Codec::Hevc {
            bitrate_bps: 20_000_000,
            ten_bit: true,
        },
        container: Container::Mov,
        tags,
        alpha: AlphaOutput::None,
        working: WorkingEncoding::Standard,
        dither: Dither::Off,
        audio: None,
        timecode: None,
        destination,
        overwrite: Overwrite::Replace,
        mode: None,
    }
}

fn push_all<S: pm_export::pipeline::Sink>(mut job: Pipeline<S>) -> S::Output {
    for n in 0..FRAMES {
        job.push_frame(&WorkingFrame {
            width: W as u32,
            height: H as u32,
            rgba: &picture(n),
        })
        .unwrap();
    }
    job.finish().unwrap()
}

/// (Cb, Cr) per chroma sample of an `x420` frame, in 10-bit codes.
fn chroma(input: &EncoderInput) -> Vec<[f64; 2]> {
    let c = input.planes[1].u16().expect("x420 chroma is 16-bit");
    c.chunks_exact(2)
        .map(|p| [f64::from(p[0] >> 6), f64::from(p[1] >> 6)])
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Siting {
    TopLeft,
    Centred,
}

/// The band-limited 2:1 decimation of frame `n`'s full-resolution Y′CbCr in
/// `tags`, in 10-bit codes, for chroma sited at `siting`.
fn reference(n: u64, tags: ColorTags, siting: Siting) -> Vec<[f64; 2]> {
    let full = convert_tagged(
        &WorkingFrame {
            width: W as u32,
            height: H as u32,
            rgba: &picture(n),
        },
        InputFormat::Y416,
        &ConvertParams {
            working: WorkingEncoding::Standard,
            alpha: AlphaOutput::None,
            tags,
            dither: false,
            frame: n as u32,
        },
        tags,
        Chroma::Filtered,
    )
    .unwrap();
    let across = [(-1i64, 0.25), (0, 0.5), (1, 0.25)];
    let down: &[(i64, f64)] = match siting {
        Siting::TopLeft => &[(-1, 0.25), (0, 0.5), (1, 0.25)],
        Siting::Centred => &[(-1, 0.125), (0, 0.375), (1, 0.375), (2, 0.125)],
    };
    let mut out = Vec::with_capacity(W * H / 4);
    for cy in 0..H as i64 / 2 {
        for cx in 0..W as i64 / 2 {
            let mut acc = [0.0; 2];
            for &(dy, wy) in down {
                for &(dx, wx) in &across {
                    let x = (2 * cx + dx).clamp(0, W as i64 - 1) as usize;
                    let y = (2 * cy + dy).clamp(0, H as i64 - 1) as usize;
                    let v = sample_ycbcra(&full, x, y);
                    acc[0] += wx * wy * v[1];
                    acc[1] += wx * wy * v[2];
                }
            }
            out.push(acc.map(|c| c * 896.0 + 512.0));
        }
    }
    out
}

/// The largest distance of `frames`' chroma from the reference at `siting`.
fn distance(frames: &[EncoderInput], tags: ColorTags, siting: Siting) -> f64 {
    frames
        .iter()
        .enumerate()
        .flat_map(|(n, f)| {
            let want = reference(n as u64, tags, siting);
            chroma(f)
                .into_iter()
                .zip(want)
                .map(|(got, want)| (got[0] - want[0]).abs().max((got[1] - want[1]).abs()))
                .collect::<Vec<_>>()
        })
        .fold(0.0, f64::max)
}

fn decoded(path: &Path) -> Vec<EncoderInput> {
    (0..FRAMES)
        .map(|n| {
            decode_frame(
                path,
                n,
                video_timing(Rate::FPS_24).unwrap(),
                InputFormat::X420,
            )
            .unwrap()
        })
        .collect()
}

/// The chroma location the stream's own SPS states.
fn signalled(path: &Path) -> Option<(u32, u32)> {
    let movie = Movie::read(path).unwrap();
    let visual = movie.track(b"vide").unwrap().visual.clone().unwrap();
    let hevc = visual.hevc.expect("an hvcC");
    hevc.vui.expect("a parsable SPS").chroma_location
}

fn software_hevc() -> bool {
    let Ok(out) = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
    else {
        return false;
    };
    String::from_utf8_lossy(&out.stdout).contains("libx265")
}

/// What a tag set's 4:2:0 chroma should be: its siting, and what a stream
/// may say about it (type 0 is the default a stream may leave out).
fn expected(tags: ColorTags) -> (Siting, &'static [Option<(u32, u32)>]) {
    if tags.is_hdr() {
        (Siting::TopLeft, &[Some((2, 2))])
    } else {
        (Siting::Centred, &[None, Some((0, 0))])
    }
}

fn check(name: &str, tags: ColorTags) {
    let dir = scratch(name);
    let (siting, locations) = expected(tags);
    let other = match siting {
        Siting::TopLeft => Siting::Centred,
        Siting::Centred => Siting::TopLeft,
    };

    // The encoder input: the declared decimation, to the rounding.
    let input =
        push_all(Pipeline::new(spec(dir.join("unused.mov"), tags), MemorySink::default()).unwrap())
            .frames;
    let (at, away) = (
        distance(&input, tags, siting),
        distance(&input, tags, other),
    );
    eprintln!(
        "MEASURE {name} encoder input: {at:.2} codes from the {siting:?} decimation, {away:.1} from the {other:?} one"
    );
    assert!(
        at <= 0.51,
        "{name}: encoder input {at:.2} codes from {siting:?}"
    );
    assert!(away > 8.0, "the rule tells the sitings apart ({away:.1})");

    // The hardware encoder's file.
    let hardware = dir.join("hardware.mov");
    push_all(pm_export::export(spec(hardware.clone(), tags)).unwrap());
    let (at, said) = (
        distance(&decoded(&hardware), tags, siting),
        signalled(&hardware),
    );
    eprintln!(
        "MEASURE {name} VideoToolbox file: decoded {at:.1} codes from the {siting:?} decimation; SPS chroma location {said:?}"
    );
    assert!(at <= 3.0, "{name}: decoded {at:.1} codes from {siting:?}");
    assert!(
        locations.contains(&said),
        "{name}: the hardware encoder's stream says chroma location {said:?}"
    );

    // The software encoder's file.
    if software_hevc() {
        let software = dir.join("software.mov");
        let s = spec(software.clone(), tags);
        push_all(
            Pipeline::new(
                s.clone(),
                OutputSink::Software(Box::new(
                    pm_export::software::SoftwareSink::new(&s).unwrap(),
                )),
            )
            .unwrap(),
        );
        let (at, said) = (
            distance(&decoded(&software), tags, siting),
            signalled(&software),
        );
        eprintln!(
            "MEASURE {name} software file: decoded {at:.1} codes from the {siting:?} decimation; SPS chroma location {said:?}"
        );
        assert!(
            at <= 3.0,
            "{name}: software decoded {at:.1} codes from {siting:?}"
        );
        assert!(
            locations.contains(&said),
            "{name}: the software encoder's stream says chroma location {said:?}"
        );
    } else {
        eprintln!("SKIP {name} software file: ffmpeg lacks libx265");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pq_420_chroma_is_top_left_and_says_so() {
    check("pq", pq());
}

#[test]
fn hlg_420_chroma_is_top_left_and_says_so() {
    check("hlg", hlg());
}

#[test]
fn rec709_420_chroma_stays_centred_between_rows() {
    check("rec709", ColorTags::Rec709QuickTime);
}
