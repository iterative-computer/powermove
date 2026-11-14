//! What a delivered H.264 or HEVC stream says about its own picture.
//!
//! A player or a transcoder may read the stream's parameter sets and never
//! the container's `colr`, and a decoder's frame metadata can be inherited
//! from the container. So these checks parse the parameter sets out of the
//! file's `avcC` / `hvcC` themselves: the SPS VUI's colour description
//! (primaries, transfer, matrix), its range flag, and its chroma sample
//! location, for the hardware encoder and the software one, SDR and HDR.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::process::Command;

use pm_export::convert::WorkingFrame;
use pm_export::mp4box::{Movie, Vui};
use pm_export::pipeline::{Pipeline, Sink};
use pm_export::*;
use pm_time::Rate;

const W: u32 = 64;
const H: u32 = 32;
const FRAMES: u64 = 3;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-export-sets-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn spec(destination: PathBuf, codec: Codec, container: Container, tags: ColorTags) -> ExportSpec {
    ExportSpec {
        width: W,
        height: H,
        rate: Rate::FPS_24,
        frames: FRAMES,
        codec,
        container,
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

fn write<S: Sink>(mut job: Pipeline<S>) {
    let rgba: Vec<f32> = (0..W * H)
        .flat_map(|i| {
            let v = (i % W) as f32 / W as f32;
            [v, 0.5 * v, 1.0 - v, 1.0]
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

/// The stream's own statement, and the parameter set types an `hvcC` carries.
fn stream(path: &Path) -> (Vui, Vec<u8>) {
    let movie = Movie::read(path).unwrap();
    let visual = movie.track(b"vide").unwrap().visual.clone().unwrap();
    match (visual.sps, visual.hevc) {
        (Some(sps), _) => (
            Vui {
                full_range: sps.full_range,
                colour_description: sps.colour_description,
                chroma_location: sps.chroma_location,
            },
            Vec::new(),
        ),
        (None, Some(hevc)) => (
            hevc.vui.expect("an HEVC SPS this reader follows"),
            hevc.parameter_sets,
        ),
        (None, None) => panic!("{} has neither avcC nor hvcC", path.display()),
    }
}

fn software_has(encoder: &str) -> bool {
    Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains(encoder))
}

fn check(name: &str, codec: Codec, container: Container, tags: ColorTags) {
    let dir = scratch(name);
    let (p, t, m) = tags.cicp();
    let want_colour = Some((p as u8, t as u8, m as u8));
    // Type 0 is the default a stream may leave out; anything else is stated.
    let want_location: &[Option<(u32, u32)>] = if tags.is_hdr() {
        &[Some((2, 2))]
    } else {
        &[None, Some((0, 0))]
    };
    let hevc = matches!(codec, Codec::Hevc { .. });
    let software = software_has(if hevc { "libx265" } else { "libx264" });
    for encoder in ["VideoToolbox", "software"] {
        if encoder == "software" && !software {
            eprintln!("SKIP {name} software: ffmpeg lacks the encoder");
            continue;
        }
        let path = dir.join(format!("{encoder}.{}", container.extension()));
        let tags = if encoder == "software" {
            plain(tags)
        } else {
            tags
        };
        let s = spec(path.clone(), codec, container, tags);
        if encoder == "software" {
            let sink = pm_export::software::SoftwareSink::new(&s).unwrap();
            write(Pipeline::new(s, OutputSink::Software(Box::new(sink))).unwrap());
        } else {
            write(pm_export::export(s).unwrap());
        }
        let (vui, sets) = stream(&path);
        eprintln!(
            "MEASURE {name} {encoder}: SPS VUI colour {:?}, full range {:?}, chroma location {:?}; parameter sets {sets:?}",
            vui.colour_description, vui.full_range, vui.chroma_location
        );
        assert_eq!(
            vui.colour_description, want_colour,
            "{name} {encoder}: the stream's colour description"
        );
        assert_eq!(
            vui.full_range,
            Some(false),
            "{name} {encoder}: the stream's range flag"
        );
        assert!(
            want_location.contains(&vui.chroma_location),
            "{name} {encoder}: the stream's chroma location {:?}",
            vui.chroma_location
        );
        if hevc {
            for kind in [32, 33, 34] {
                assert!(
                    sets.contains(&kind),
                    "{name} {encoder}: hvcC carries parameter set type {kind} ({sets:?})"
                );
            }
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

const H264: Codec = Codec::H264 {
    bitrate_bps: 8_000_000,
};
const HEVC8: Codec = Codec::Hevc {
    bitrate_bps: 8_000_000,
    ten_bit: false,
};
const HEVC10: Codec = Codec::Hevc {
    bitrate_bps: 8_000_000,
    ten_bit: true,
};

#[test]
fn h264_states_rec709_quicktime() {
    check(
        "h264-1-13-1",
        H264,
        Container::Mp4,
        ColorTags::Rec709QuickTime,
    );
}

#[test]
fn h264_states_rec709_broadcast() {
    check(
        "h264-1-1-1",
        H264,
        Container::Mov,
        ColorTags::Rec709Broadcast,
    );
}

#[test]
fn hevc_states_rec709() {
    check(
        "hevc8-1-13-1",
        HEVC8,
        Container::Mp4,
        ColorTags::Rec709QuickTime,
    );
    check(
        "hevc10-1-1-1",
        HEVC10,
        Container::Mov,
        ColorTags::Rec709Broadcast,
    );
}

#[test]
fn hevc_states_pq() {
    check(
        "hevc10-pq",
        HEVC10,
        Container::Mov,
        ColorTags::Rec2100Pq {
            white_nits: 203,
            metadata: None,
        },
    );
}

/// The tags without Dolby Vision, which only the VideoToolbox sink writes.
fn plain(tags: ColorTags) -> ColorTags {
    match tags {
        ColorTags::Rec2100Hlg {
            white_nits,
            peak_nits,
            metadata,
            ..
        } => ColorTags::Rec2100Hlg {
            white_nits,
            peak_nits,
            metadata,
            dolby_vision: false,
        },
        tags => tags,
    }
}
#[test]
fn hevc_states_hlg() {
    check(
        "hevc10-hlg",
        HEVC10,
        Container::Mp4,
        ColorTags::Rec2100Hlg {
            white_nits: 203,
            peak_nits: 1000,
            metadata: None,
            dolby_vision: true,
        },
    );
}
