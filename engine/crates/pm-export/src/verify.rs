//! Verification (section 17): after muxing and before the move into place, the file is
//! probed for codec, profile, pixel format, depth, tags, range, frame count, duration,
//! audio samples, alpha, timecode, and sampled pixel values, and compared with what
//! the export asked for and what the encoder was handed. Any mismatch fails the export.
//!
//! Pixels are compared as 16×16 block means at nine places in the first, middle, and
//! last frames, against the means of the encoder input itself: lossy coding noise
//! averages out, while a wrong matrix, range, channel order, alpha mode, or a black or
//! stale frame moves a block mean by tens of code values.

use crate::convert::{EncoderInput, sample_ycbcra};
use crate::mp4box::Movie;
use crate::settings::{
    AlphaOutput, AudioSpec, Codec, Container, ExportSpec, ProResProfile, TimecodeSpec,
};
use crate::timescale::VideoTiming;

/// Block side for pixel checks.
pub const BLOCK: u32 = 16;

/// A block mean of normalized Y′, Cb, Cr, A.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockSample {
    pub frame: u64,
    pub x: u32,
    pub y: u32,
    pub mean: [f64; 4],
}

/// The frames whose pixels are checked: first, middle, last.
pub fn sampled_frames(frames: u64) -> Vec<u64> {
    let mut v = vec![0, frames / 2, frames.saturating_sub(1)];
    v.dedup();
    v
}

/// Block means at a 3 × 3 grid (block corners aligned to 16 pixels).
pub fn block_means(input: &EncoderInput, frame: u64) -> Vec<BlockSample> {
    let (w, h) = (input.width, input.height);
    let size_x = BLOCK.min(w);
    let size_y = BLOCK.min(h);
    let place = |extent: u32, size: u32, i: u32| -> u32 {
        let c = (extent - size) * i / 2;
        (c / BLOCK * BLOCK).min(extent - size)
    };
    let mut out = vec![];
    for j in 0..3 {
        for i in 0..3 {
            let (x0, y0) = (place(w, size_x, i), place(h, size_y, j));
            if out.iter().any(|b: &BlockSample| b.x == x0 && b.y == y0) {
                continue;
            }
            let mut sum = [0.0; 4];
            for y in y0..y0 + size_y {
                for x in x0..x0 + size_x {
                    let s = sample_ycbcra(input, x as usize, y as usize);
                    for k in 0..4 {
                        sum[k] += s[k];
                    }
                }
            }
            let n = (size_x * size_y) as f64;
            out.push(BlockSample {
                frame,
                x: x0,
                y: y0,
                mean: sum.map(|v| v / n),
            });
        }
    }
    out
}

/// What the export asked for and handed the encoder.
#[derive(Clone, Debug)]
pub struct Expected {
    pub codec: Codec,
    pub container: Container,
    pub cicp: (u16, u16, u16),
    pub alpha: AlphaOutput,
    pub frames: u64,
    pub timing: VideoTiming,
    pub audio: Option<(AudioSpec, u64)>,
    pub timecode: Option<TimecodeSpec>,
    pub blocks: Vec<BlockSample>,
}

impl Expected {
    pub fn new(
        spec: &ExportSpec,
        timing: VideoTiming,
        audio_frames: u64,
        blocks: Vec<BlockSample>,
    ) -> Self {
        Self {
            codec: spec.codec,
            container: spec.container,
            cicp: spec.tags.cicp(),
            alpha: spec.alpha,
            frames: spec.frames,
            timing,
            audio: spec.audio.map(|a| (a, audio_frames)),
            timecode: spec.timecode.clone(),
            blocks,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioReport {
    pub codec: String,
    pub sample_rate: f64,
    pub channels: u32,
    pub timescale: u32,
    /// Presented sample frames (after the edit list).
    pub samples: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimecodeReport {
    pub start_frame: u32,
    pub drop_frame: bool,
    pub frames_per_second: u8,
    pub reel: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PixelCheck {
    pub frame: u64,
    pub x: u32,
    pub y: u32,
    pub expected: [f64; 4],
    pub decoded: [f64; 4],
    /// Largest channel difference in code values at the codec's depth.
    pub error_cv: f64,
}

/// What the probe found.
#[derive(Clone, Debug, PartialEq)]
pub struct VerifyReport {
    pub codec: String,
    pub profile: String,
    pub pixel_format: String,
    pub depth: u32,
    pub cicp: (u16, u16, u16),
    pub full_range: bool,
    pub alpha: bool,
    pub frames: u64,
    pub movie_timescale: u32,
    pub media_timescale: u32,
    /// Video duration in the media timescale.
    pub duration: u64,
    pub audio: Option<AudioReport>,
    pub timecode: Option<TimecodeReport>,
    pub pixels: Vec<PixelCheck>,
    /// Whether VideoToolbox reported a hardware encoder.
    pub hardware_encoder: Option<bool>,
}

impl VerifyReport {
    pub fn max_pixel_error_cv(&self) -> f64 {
        self.pixels.iter().map(|p| p.error_cv).fold(0.0, f64::max)
    }
}

/// Block-mean tolerance in code values at the codec's depth.
pub fn pixel_tolerance(codec: Codec) -> f64 {
    match codec {
        Codec::ProRes(_) => 4.0,
        Codec::H264 { .. } => 6.0,
    }
}

fn profile_name(codec: Codec) -> &'static str {
    match codec {
        Codec::ProRes(ProResProfile::Proxy) => "ProRes 422 Proxy",
        Codec::ProRes(ProResProfile::Lt) => "ProRes 422 LT",
        Codec::ProRes(ProResProfile::Standard) => "ProRes 422",
        Codec::ProRes(ProResProfile::Hq) => "ProRes 422 HQ",
        Codec::ProRes(ProResProfile::P4444) => "ProRes 4444",
        Codec::ProRes(ProResProfile::P4444Xq) => "ProRes 4444 XQ",
        Codec::H264 { .. } => "H.264 High",
    }
}

fn fourcc(b: [u8; 4]) -> String {
    String::from_utf8_lossy(&b).into_owned()
}

/// Compares the probed file with the expectation. `decoded` holds the sampled frames
/// decoded to the encoder-input format.
pub fn check(
    expected: &Expected,
    movie: &Movie,
    decoded: &[(u64, EncoderInput)],
) -> Result<VerifyReport, crate::Error> {
    let mut problems: Vec<String> = vec![];
    let mut fail = |m: String| problems.push(m);
    let Some(video) = movie.track(b"vide") else {
        return Err(crate::Error::Verify("the file has no video track".into()));
    };
    let visual = video.visual.clone().unwrap_or_default();
    let codec = fourcc(video.sample_entry);
    if video.sample_entry != expected.codec.fourcc() {
        fail(format!(
            "codec '{codec}', expected '{}'",
            fourcc(expected.codec.fourcc())
        ));
    }
    let want_alpha = expected.alpha != AlphaOutput::None;
    let (profile, pixel_format, depth, alpha) = match expected.codec {
        Codec::ProRes(p) => {
            let alpha = visual.depth == 32;
            let fmt = if p.is_4444() {
                if alpha { "4:4:4:4" } else { "4:4:4" }
            } else {
                "4:2:2"
            };
            (
                profile_name(expected.codec).to_string(),
                fmt.to_string(),
                expected.codec.depth(),
                alpha,
            )
        }
        Codec::H264 { .. } => match &visual.sps {
            Some(sps) => {
                if sps.profile_idc != 100 {
                    fail(format!(
                        "H.264 profile_idc {}, expected 100 (High)",
                        sps.profile_idc
                    ));
                }
                let fmt = match sps.chroma_format_idc {
                    0 => "4:0:0",
                    1 => "4:2:0",
                    2 => "4:2:2",
                    _ => "4:4:4",
                };
                (
                    format!("H.264 profile {} level {}", sps.profile_idc, sps.level_idc),
                    fmt.to_string(),
                    sps.bit_depth_luma,
                    false,
                )
            }
            None => {
                fail("H.264 track has no parsable SPS".into());
                (String::new(), String::new(), 0, false)
            }
        },
    };
    let want_format = match expected.codec {
        Codec::ProRes(p) if p.is_4444() => {
            if want_alpha {
                "4:4:4:4"
            } else {
                "4:4:4"
            }
        }
        Codec::ProRes(_) => "4:2:2",
        Codec::H264 { .. } => "4:2:0",
    };
    if pixel_format != want_format {
        fail(format!(
            "pixel format {pixel_format}, expected {want_format}"
        ));
    }
    if depth != expected.codec.depth() {
        fail(format!(
            "depth {depth} bits, expected {}",
            expected.codec.depth()
        ));
    }
    if alpha != want_alpha {
        fail(format!(
            "alpha channel {}, expected {}",
            if alpha { "present" } else { "absent" },
            if want_alpha { "present" } else { "absent" }
        ));
    }
    // Tags and range.
    let cicp = match &visual.colr {
        Some(c) => (c.primaries, c.transfer, c.matrix),
        None => {
            fail("no colr box".into());
            (0, 0, 0)
        }
    };
    if cicp != expected.cicp {
        fail(format!(
            "colr {}-{}-{}, expected {}-{}-{}",
            cicp.0, cicp.1, cicp.2, expected.cicp.0, expected.cicp.1, expected.cicp.2
        ));
    }
    let mut full_range = visual
        .colr
        .as_ref()
        .and_then(|c| c.full_range)
        .unwrap_or(false);
    if let Some(sps) = &visual.sps {
        if let Some((p, t, m)) = sps.colour_description
            && (p as u16, t as u16, m as u16) != expected.cicp
        {
            fail(format!(
                "H.264 VUI colour {p}-{t}-{m}, expected {}-{}-{}",
                expected.cicp.0, expected.cicp.1, expected.cicp.2
            ));
        }
        full_range |= sps.full_range.unwrap_or(false);
    }
    if full_range {
        fail("video is flagged full range; exports are limited range".into());
    }
    // Frames, timescales, duration.
    let t = expected.timing;
    if video.sample_count != expected.frames {
        fail(format!(
            "{} video frames, expected {}",
            video.sample_count, expected.frames
        ));
    }
    if video
        .stts
        .iter()
        .any(|&(_, d)| d as i64 != t.frame_duration)
    {
        fail(format!(
            "frame durations {:?}, expected {} at {}",
            video.stts, t.frame_duration, t.timescale
        ));
    }
    if video.media_timescale as i32 != t.timescale || movie.movie_timescale as i32 != t.timescale {
        fail(format!(
            "timescales movie {} media {}, expected {}",
            movie.movie_timescale, video.media_timescale, t.timescale
        ));
    }
    let duration = video.stts_duration();
    if duration as i64 != t.duration(expected.frames) {
        fail(format!(
            "video duration {duration}, expected {}",
            t.duration(expected.frames)
        ));
    }
    // Audio.
    let audio = movie.track(b"soun").map(|a| {
        let entry = a.audio.clone();
        let presented = match a.edits.iter().find(|e| e.media_time >= 0) {
            Some(edit) if movie.movie_timescale > 0 => (edit.segment_duration as f64
                * a.media_timescale as f64
                / movie.movie_timescale as f64)
                .round() as u64,
            _ => a.stts_duration(),
        };
        AudioReport {
            codec: fourcc(a.sample_entry),
            sample_rate: entry.as_ref().map(|e| e.sample_rate).unwrap_or(0.0),
            channels: entry.as_ref().map(|e| e.channels).unwrap_or(0),
            timescale: a.media_timescale,
            samples: presented,
        }
    });
    match (&expected.audio, &audio) {
        (None, None) => {}
        (None, Some(_)) => fail("unexpected audio track".into()),
        (Some(_), None) => fail("audio track missing".into()),
        (Some((spec, pushed)), Some(got)) => {
            let want_codec = match expected.container {
                Container::Mov => "lpcm",
                Container::Mp4 => "mp4a",
            };
            if got.codec != want_codec {
                fail(format!(
                    "audio codec '{}', expected '{want_codec}'",
                    got.codec
                ));
            }
            if got.sample_rate != spec.sample_rate as f64 || got.timescale != spec.sample_rate {
                fail(format!(
                    "audio at {} Hz (timescale {}), expected {}",
                    got.sample_rate, got.timescale, spec.sample_rate
                ));
            }
            if got.channels != spec.channels as u32 {
                fail(format!(
                    "{} audio channels, expected {}",
                    got.channels, spec.channels
                ));
            }
            // The session ends with the video, so audio beyond it is trimmed.
            let video_samples = (t.duration(expected.frames) as f64 * spec.sample_rate as f64
                / t.timescale as f64)
                .round() as u64;
            let want = (*pushed).min(video_samples);
            let slack =
                (spec.sample_rate as f64 / movie.movie_timescale.max(1) as f64).ceil() as u64 + 1;
            if got.samples.abs_diff(want) > slack {
                fail(format!("{} audio samples, expected {want}", got.samples));
            }
        }
    }
    // Timecode.
    let timecode = movie
        .track(b"tmcd")
        .and_then(|tc| tc.tmcd.clone())
        .map(|tc| TimecodeReport {
            start_frame: tc.start_frame.unwrap_or(u32::MAX),
            drop_frame: tc.drop_frame(),
            frames_per_second: tc.frames_per_second,
            reel: tc.reel.clone(),
        });
    match (&expected.timecode, &timecode) {
        (None, None) => {}
        (None, Some(_)) => fail("unexpected timecode track".into()),
        (Some(_), None) => fail("timecode track missing".into()),
        (Some(spec), Some(got)) => {
            if got.start_frame as i64 != spec.start_frame {
                fail(format!(
                    "start timecode frame {}, expected {}",
                    got.start_frame, spec.start_frame
                ));
            }
            if got.drop_frame != spec.format.is_drop_frame() {
                fail(format!(
                    "timecode drop-frame flag {}, expected {}",
                    got.drop_frame,
                    spec.format.is_drop_frame()
                ));
            }
            if got.frames_per_second as u32 != spec.format.base() {
                fail(format!(
                    "timecode counts {} frames per second, expected {}",
                    got.frames_per_second,
                    spec.format.base()
                ));
            }
            if got.reel != spec.reel {
                fail(format!("reel {:?}, expected {:?}", got.reel, spec.reel));
            }
            let tc_id = movie.track(b"tmcd").map(|t| t.id).unwrap_or(0);
            if !video
                .references
                .iter()
                .any(|(kind, ids)| kind == b"tmcd" && ids.contains(&tc_id))
            {
                fail("the video track does not reference the timecode track".into());
            }
        }
    }
    // Pixels.
    let scale = |k: usize| -> f64 {
        let d = expected.codec.depth() as i32;
        match k {
            0 => 219.0 * 2f64.powi(d - 8),
            1 | 2 => 224.0 * 2f64.powi(d - 8),
            _ => 2f64.powi(d) - 1.0,
        }
    };
    let mut pixels = vec![];
    for (frame, input) in decoded {
        for got in block_means(input, *frame) {
            let Some(want) = expected
                .blocks
                .iter()
                .find(|b| b.frame == *frame && b.x == got.x && b.y == got.y)
            else {
                continue;
            };
            let error_cv = (0..4)
                .map(|k| (got.mean[k] - want.mean[k]).abs() * scale(k))
                .fold(0.0, f64::max);
            pixels.push(PixelCheck {
                frame: *frame,
                x: got.x,
                y: got.y,
                expected: want.mean,
                decoded: got.mean,
                error_cv,
            });
        }
    }
    if pixels.len() != expected.blocks.len() {
        fail(format!(
            "checked {} of {} pixel blocks",
            pixels.len(),
            expected.blocks.len()
        ));
    }
    let tolerance = pixel_tolerance(expected.codec);
    for p in pixels.iter().filter(|p| p.error_cv > tolerance) {
        fail(format!(
            "frame {} block ({}, {}) differs by {:.2} CV (Y′CbCrA {:?} vs {:?})",
            p.frame, p.x, p.y, p.error_cv, p.decoded, p.expected
        ));
    }
    if !problems.is_empty() {
        return Err(crate::Error::Verify(problems.join("; ")));
    }
    Ok(VerifyReport {
        codec,
        profile,
        pixel_format,
        depth,
        cicp,
        full_range,
        alpha,
        frames: video.sample_count,
        movie_timescale: movie.movie_timescale,
        media_timescale: video.media_timescale,
        duration,
        audio,
        timecode,
        pixels,
        hardware_encoder: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::convert::{ConvertParams, WorkingFrame, convert};
    use crate::settings::{InputFormat, WorkingEncoding};

    #[test]
    fn sampled_frames_are_first_middle_last() {
        assert_eq!(sampled_frames(1), vec![0]);
        assert_eq!(sampled_frames(2), vec![0, 1]);
        assert_eq!(sampled_frames(48), vec![0, 24, 47]);
    }

    #[test]
    fn block_means_cover_a_grid_and_match_flat_frames() {
        let (w, h) = (100u32, 48u32);
        let rgba: Vec<f32> = (0..w * h).flat_map(|_| [0.5f32, 0.25, 0.75, 1.0]).collect();
        let input = convert(
            &WorkingFrame {
                width: w,
                height: h,
                rgba: &rgba,
            },
            InputFormat::V420,
            &ConvertParams {
                working: WorkingEncoding::Standard,
                alpha: AlphaOutput::None,
                dither: false,
                frame: 0,
            },
        )
        .unwrap();
        let blocks = block_means(&input, 7);
        assert_eq!(blocks.len(), 9);
        assert!(
            blocks
                .iter()
                .all(|b| b.x % 16 == 0 && b.y % 16 == 0 && b.x + 16 <= w && b.y + 16 <= h)
        );
        let first = blocks[0].mean;
        assert!(blocks.iter().all(|b| b.mean == first && b.frame == 7));
        // Tiny frames still sample inside the frame.
        let tiny: Vec<f32> = (0..4 * 2).flat_map(|_| [0.0f32, 0.0, 0.0, 1.0]).collect();
        let input = convert(
            &WorkingFrame {
                width: 4,
                height: 2,
                rgba: &tiny,
            },
            InputFormat::Y416,
            &ConvertParams {
                working: WorkingEncoding::Standard,
                alpha: AlphaOutput::None,
                dither: false,
                frame: 0,
            },
        )
        .unwrap();
        assert_eq!(block_means(&input, 0).len(), 1);
    }
}
