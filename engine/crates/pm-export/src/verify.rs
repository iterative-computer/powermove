//! Verification (section 17): after muxing and before the move into place, the file is
//! probed for codec, profile, pixel format, depth, tags, range, frame count, duration,
//! audio samples, alpha, timecode, and sampled pixel values, and compared with what
//! the export asked for and what the encoder was handed. Any mismatch fails the export.
//!
//! The picture itself is held to what the encoder was handed in every frame and, more
//! closely, in sampled regions of the first, middle and last frame (`evidence`). Only
//! what no codec loss can explain fails the export; what is a matter of degree is
//! reported in [`VerifyReport::notes`] with the file kept.

pub use crate::evidence::{
    BLOCK, BlockSample, Decoded, Evidence, FrameSignature, PictureReport, PixelCheck, REGION,
    block_means, channel_scale, check_frames, check_pixels, frame_signature, level_tolerance,
    pixel_tolerance, sampled_frames,
};
pub(crate) use crate::evidence::{block_regions, check_pixel_blocks};
use crate::mp4box::Movie;
use crate::settings::{
    AlphaOutput, AudioCodec, AudioSpec, Codec, Container, ExportSpec, TimecodeSpec,
};
use crate::timescale::VideoTiming;

/// What the export asked for and handed the encoder.
#[derive(Clone, Debug)]
pub struct Expected {
    pub codec: Codec,
    pub container: Container,
    pub cicp: (u16, u16, u16),
    /// `chroma_sample_loc_type` of a 4:2:0 stream's chroma.
    pub chroma_location: u32,
    pub hdr: Option<crate::settings::HdrMetadata>,
    /// HLG HEVC: whether the file carries Dolby Vision 8.4 (`None`: other
    /// outputs, which are not held to it).
    pub dolby_vision: Option<bool>,
    pub alpha: AlphaOutput,
    pub frames: u64,
    pub timing: VideoTiming,
    pub audio: Option<(AudioSpec, u64)>,
    pub timecode: Option<TimecodeSpec>,
    pub evidence: Evidence,
}

/// The Dolby Vision record a file of `spec` carries: `Some(true)` the 8.4
/// record (HLG HEVC with it on), `Some(false)` none, `None` it does not
/// apply. Every route's file check asks this.
pub fn expected_dolby_vision(spec: &ExportSpec) -> Option<bool> {
    match (spec.codec, spec.tags) {
        (Codec::Hevc { .. }, crate::settings::ColorTags::Rec2100Hlg { dolby_vision, .. }) => {
            Some(dolby_vision)
        }
        _ => None,
    }
}

/// What is wrong with the Dolby Vision record `found` against `expected`:
/// profile 8, HLG compatible, an RPU and no enhancement layer, or nothing
/// when it was turned off.
pub fn dolby_vision_problem(
    expected: Option<bool>,
    found: Option<crate::mp4box::Dovi>,
) -> Option<String> {
    match (expected, found) {
        (Some(true), Some(d))
            if d.profile == 8
                && d.compatibility == 4
                && d.rpu
                && d.base_layer
                && !d.enhancement_layer =>
        {
            None
        }
        (Some(true), found) => Some(format!(
            "Dolby Vision {found:?}, expected profile 8.4 (HLG compatible)"
        )),
        (Some(false), Some(d)) => Some(format!("Dolby Vision {d:?}, expected none")),
        _ => None,
    }
}

impl Expected {
    pub fn new(
        spec: &ExportSpec,
        timing: VideoTiming,
        audio_frames: u64,
        evidence: Evidence,
    ) -> Self {
        Self {
            codec: spec.codec,
            container: spec.container,
            cicp: spec.tags.cicp(),
            chroma_location: spec.tags.chroma_location().sample_loc_type(),
            hdr: spec.tags.hdr_metadata(),
            dolby_vision: expected_dolby_vision(spec),
            alpha: spec.alpha,
            frames: spec.frames,
            timing,
            audio: spec.audio.map(|a| (a, audio_frames)),
            timecode: spec.timecode.clone(),
            evidence,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AudioReport {
    pub codec: String,
    pub sample_rate: f64,
    pub channels: u32,
    pub bits_per_channel: u32,
    pub float: bool,
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

/// How loud the mixed sound got: its largest sample, and how much of it was
/// over full scale. An integer PCM, AAC or Opus track cannot hold a sample
/// over full scale and clips it; a 32-bit float track keeps it. The person
/// exporting is told how much, and which.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MixLevels {
    /// The largest absolute sample of the mix (1.0 is full scale).
    pub peak: f32,
    /// Samples, counted in every channel, above full scale.
    pub overs: u64,
    /// The track holds the mix as it is (32-bit float PCM): what is over
    /// full scale is in the file, not clipped.
    pub kept: bool,
}

impl MixLevels {
    /// The levels of a mix delivered as `codec`, before any of it is heard.
    pub fn of(codec: AudioCodec) -> Self {
        Self {
            kept: codec == AudioCodec::PcmF32,
            ..Self::default()
        }
    }

    /// Takes in a block of the mix.
    pub fn add(&mut self, samples: &[f32]) {
        for s in samples {
            let level = s.abs();
            self.peak = self.peak.max(level);
            self.overs += u64::from(level > 1.0);
        }
    }

    /// The peak in decibels relative to full scale (`None` for silence).
    pub fn peak_db(&self) -> Option<f64> {
        (self.peak > 0.0).then(|| 20.0 * f64::from(self.peak).log10())
    }

    /// One line for the person, when part of the sound is over full scale:
    /// "Audio clipped: 1,204 samples over full scale, peak +2.1 dB.", or for
    /// a float track, which keeps them, "Audio over full scale: 1,204
    /// samples, peak +2.1 dB (kept in the 32-bit float track)."
    pub fn warning(&self) -> Option<String> {
        (self.overs > 0).then(|| {
            let samples = format!(
                "{} sample{}",
                self.overs,
                if self.overs == 1 { "" } else { "s" }
            );
            let peak = self.peak_db().unwrap_or(0.0);
            if self.kept {
                format!(
                    "Audio over full scale: {samples}, peak +{peak:.1} dB (kept in the 32-bit float track)."
                )
            } else {
                format!("Audio clipped: {samples} over full scale, peak +{peak:.1} dB.")
            }
        })
    }
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
    /// What the picture checks measured over the file.
    pub picture: PictureReport,
    /// What the person is told with the file kept: differences that are a
    /// matter of degree (fine detail a low data rate does not keep).
    pub notes: Vec<String>,
    /// The Dolby Vision configuration the file carries.
    pub dolby_vision: Option<crate::mp4box::Dovi>,
    /// An HDR file's MaxCLL and MaxFALL, measured from the frames it was
    /// given (what its `clli` says).
    pub content_light: Option<crate::convert::FrameLight>,
    /// Whether VideoToolbox reported a hardware encoder.
    pub hardware_encoder: Option<bool>,
    /// Bytes delivered (every file of a PNG sequence).
    pub bytes: u64,
    /// The mixed sound's levels, for an export job with sound.
    pub mix: Option<MixLevels>,
}

impl VerifyReport {
    pub fn max_pixel_error_cv(&self) -> f64 {
        self.pixels.iter().map(|p| p.error_cv).fold(0.0, f64::max)
    }
}

fn profile_name(codec: Codec) -> String {
    codec.name()
}

fn fourcc(b: [u8; 4]) -> String {
    String::from_utf8_lossy(&b).into_owned()
}

/// The pixel format the codec's stream is expected to carry.
pub fn expected_pixel_format(codec: Codec, alpha: bool) -> &'static str {
    match codec {
        Codec::ProRes(p) if p.is_4444() => {
            if alpha {
                "4:4:4:4"
            } else {
                "4:4:4"
            }
        }
        Codec::ProRes(_) => "4:2:2",
        Codec::H264 { .. } | Codec::Hevc { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. } => {
            if alpha { "4:2:0:4" } else { "4:2:0" }
        }
        Codec::Png { .. } => {
            if alpha {
                "RGBA"
            } else {
                "RGB"
            }
        }
    }
}

/// What a track's own parameter sets (H.264 or HEVC SPS VUI) say against
/// what the export asked for: a player may read them and not the container.
/// Returns the problems, and whether the stream says full range.
pub(crate) fn stream_problems(
    visual: &crate::mp4box::Visual,
    cicp: (u16, u16, u16),
    chroma_location: u32,
) -> (Vec<String>, bool) {
    let stream = match (&visual.sps, &visual.hevc) {
        (Some(sps), _) => Some(crate::mp4box::Vui {
            full_range: sps.full_range,
            colour_description: sps.colour_description,
            chroma_location: sps.chroma_location,
        }),
        (None, Some(hevc)) => hevc.vui,
        (None, None) => None,
    };
    let mut problems = Vec::new();
    let Some(vui) = stream else {
        return (problems, false);
    };
    if let Some((p, t, m)) = vui.colour_description
        && (p as u16, t as u16, m as u16) != cicp
    {
        problems.push(format!(
            "stream VUI colour {p}-{t}-{m}, expected {}-{}-{}",
            cicp.0, cicp.1, cicp.2
        ));
    }
    // A stream that leaves the location out means type 0.
    let said = vui.chroma_location.unwrap_or((0, 0));
    if said != (chroma_location, chroma_location) {
        problems.push(format!(
            "stream chroma location {said:?}, expected type {chroma_location}"
        ));
    }
    (problems, vui.full_range.unwrap_or(false))
}

fn chroma_name(idc: u32) -> &'static str {
    match idc {
        0 => "4:0:0",
        1 => "4:2:0",
        2 => "4:2:2",
        _ => "4:4:4",
    }
}

/// Compares the probed file with the expectation. `decoded` holds what the file
/// decoded to, in the encoder-input format.
pub fn check(
    expected: &Expected,
    movie: &Movie,
    decoded: &Decoded,
) -> Result<VerifyReport, crate::Error> {
    let (report, problems) = examine(expected, movie, decoded)?;
    if problems.is_empty() {
        Ok(report)
    } else {
        Err(crate::Error::Verify(problems.join("; ")))
    }
}

/// [`check`], with what was measured and every problem found.
pub fn examine(
    expected: &Expected,
    movie: &Movie,
    decoded: &Decoded,
) -> Result<(VerifyReport, Vec<String>), crate::Error> {
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
                profile_name(expected.codec),
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
                (
                    format!("H.264 profile {} level {}", sps.profile_idc, sps.level_idc),
                    chroma_name(sps.chroma_format_idc).to_string(),
                    sps.bit_depth_luma,
                    false,
                )
            }
            None => {
                fail("H.264 track has no parsable SPS".into());
                (String::new(), String::new(), 0, false)
            }
        },
        Codec::Hevc { ten_bit, .. } => match &visual.hevc {
            Some(h) => {
                let want_profile = if ten_bit { 2 } else { 1 };
                if h.profile_idc != want_profile {
                    fail(format!(
                        "HEVC general_profile_idc {}, expected {want_profile}",
                        h.profile_idc
                    ));
                }
                if h.bit_depth_chroma != h.bit_depth_luma {
                    fail(format!(
                        "HEVC chroma depth {}, luma {}",
                        h.bit_depth_chroma, h.bit_depth_luma
                    ));
                }
                // HEVC with alpha: a 32-bit sample description with an `almo`
                // box whose last byte is the alpha mode (measured on macOS 27:
                // 0 straight, 2 premultiplied).
                let alpha = visual.depth == 32;
                if alpha {
                    let mode = visual.alpha_mode.as_ref().and_then(|m| m.last().copied());
                    let want = match expected.alpha {
                        AlphaOutput::Premultiplied => Some(2),
                        _ => Some(0),
                    };
                    if mode != want {
                        fail(format!(
                            "HEVC alpha mode {:?}, expected {want:?} ({:?})",
                            visual.alpha_mode, expected.alpha
                        ));
                    }
                }
                let mut fmt = chroma_name(u32::from(h.chroma_format_idc)).to_string();
                if alpha {
                    fmt.push_str(":4");
                }
                (
                    format!(
                        "HEVC {} level {}",
                        if h.profile_idc == 2 { "Main10" } else { "Main" },
                        f64::from(h.level_idc) / 30.0
                    ),
                    fmt,
                    u32::from(h.bit_depth_luma),
                    alpha,
                )
            }
            None => {
                fail("HEVC track has no parsable hvcC".into());
                (String::new(), String::new(), 0, false)
            }
        },
        Codec::Vp9 { .. } | Codec::Vp8 { .. } | Codec::Png { .. } => {
            return Err(crate::Error::Verify(format!(
                "{} is not a QuickTime-family codec",
                expected.codec.name()
            )));
        }
    };
    let want_format = expected_pixel_format(expected.codec, want_alpha);
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
            "alpha channel {}, expected {} (depth {}, sample entry boxes {:?})",
            if alpha { "present" } else { "absent" },
            if want_alpha { "present" } else { "absent" },
            visual.depth,
            visual.boxes.iter().map(|b| fourcc(*b)).collect::<Vec<_>>()
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
    if let Some(hdr) = expected.hdr {
        if visual.mdcv != Some(hdr.mdcv()) {
            fail(format!("mdcv {:?}, expected {:?}", visual.mdcv, hdr.mdcv()));
        }
        if hdr.max_cll != 0 && visual.clli != Some(hdr.clli()) {
            fail(format!("clli {:?}, expected {:?}", visual.clli, hdr.clli()));
        }
    }
    if let Some(problem) = dolby_vision_problem(expected.dolby_vision, visual.dolby_vision) {
        fail(problem);
    }
    let mut full_range = visual
        .colr
        .as_ref()
        .and_then(|c| c.full_range)
        .unwrap_or(false);
    let (stream, stream_full_range) =
        stream_problems(&visual, expected.cicp, expected.chroma_location);
    stream.into_iter().for_each(&mut fail);
    full_range |= stream_full_range;
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
    let expected_movie_timescale = t.timescale as u64;
    if video.media_timescale as i32 != t.timescale
        || movie.movie_timescale as u64 != expected_movie_timescale
    {
        fail(format!(
            "timescales movie {} media {}, expected movie {} media {}",
            movie.movie_timescale, video.media_timescale, expected_movie_timescale, t.timescale
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
            bits_per_channel: entry.as_ref().map(|e| e.bits_per_channel).unwrap_or(0),
            float: entry.as_ref().is_some_and(|e| e.float),
            timescale: a.media_timescale,
            samples: presented,
        }
    });
    match (&expected.audio, &audio) {
        (None, None) => {}
        (None, Some(_)) => fail("unexpected audio track".into()),
        (Some(_), None) => fail("audio track missing".into()),
        (Some((spec, pushed)), Some(got)) => {
            let want_codec = match spec.codec {
                AudioCodec::Aac => "mp4a",
                _ => "lpcm",
            };
            if got.codec != want_codec {
                fail(format!(
                    "audio codec '{}', expected '{want_codec}'",
                    got.codec
                ));
            }
            let want_bits = match spec.codec {
                AudioCodec::Pcm16 => Some((16, false)),
                AudioCodec::Pcm24 => Some((24, false)),
                AudioCodec::PcmF32 => Some((32, true)),
                _ => None,
            };
            if let Some((bits, float)) = want_bits
                && (got.bits_per_channel != bits || got.float != float)
            {
                fail(format!(
                    "audio is {}-bit{}, expected {bits}-bit{}",
                    got.bits_per_channel,
                    if got.float { " float" } else { "" },
                    if float { " float" } else { "" }
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
    let mut picture = PictureReport::default();
    check_frames(
        expected.codec,
        &|n| expected.evidence.frames.get(usize::try_from(n).ok()?),
        decoded.complete.then_some(expected.evidence.frames.len()),
        &decoded.frames,
        &mut picture,
        &mut problems,
    );
    let mut notes = vec![];
    let pixels = check_pixels(
        expected.codec,
        &expected.evidence.blocks,
        &decoded.sampled,
        &mut picture,
        &mut problems,
        &mut notes,
    );
    let report = VerifyReport {
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
        picture,
        notes,
        dolby_vision: visual.dolby_vision,
        content_light: None,
        hardware_encoder: None,
        bytes: 0,
        mix: None,
    };
    Ok((report, problems))
}
