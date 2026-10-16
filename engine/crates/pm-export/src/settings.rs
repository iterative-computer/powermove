//! What an export produces: codec, container, color tags, alpha, dither, audio,
//! timecode, destination (plan section 17, "Formats at cutover"; section 9, "Output
//! tags"; section 26 amendments 6 and 7).

use std::path::PathBuf;

use pm_time::{Rate, TimecodeFormat};

use crate::Error;
use crate::timescale::{VideoTiming, video_timing};

/// Container. MOV and MP4 are muxed by AVAssetWriter (section 17).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    Mov,
    Mp4,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mov => "mov",
            Container::Mp4 => "mp4",
        }
    }
}

/// ProRes flavor. All are encoded by VideoToolbox (decision 12, section 24 item 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ProResProfile {
    Proxy,
    Lt,
    Standard,
    Hq,
    P4444,
    P4444Xq,
}

impl ProResProfile {
    /// The QuickTime sample entry / CMVideoCodecType.
    pub fn fourcc(self) -> [u8; 4] {
        match self {
            ProResProfile::Proxy => *b"apco",
            ProResProfile::Lt => *b"apcs",
            ProResProfile::Standard => *b"apcn",
            ProResProfile::Hq => *b"apch",
            ProResProfile::P4444 => *b"ap4h",
            ProResProfile::P4444Xq => *b"ap4x",
        }
    }

    /// 4444 family (4:4:4, optional alpha, `y416` input); otherwise 4:2:2 (`v216`).
    pub fn is_4444(self) -> bool {
        matches!(self, ProResProfile::P4444 | ProResProfile::P4444Xq)
    }
}

/// Video codec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Codec {
    ProRes(ProResProfile),
    /// H.264 High profile, 8-bit 4:2:0, at an average bit rate. Today's quality tiers
    /// are Draft, High and Max at 4, 16 and 40 Mbps.
    H264 {
        bitrate_bps: u32,
    },
}

impl Codec {
    pub fn fourcc(self) -> [u8; 4] {
        match self {
            Codec::ProRes(p) => p.fourcc(),
            Codec::H264 { .. } => *b"avc1",
        }
    }

    /// The encoder input the engine hands VideoToolbox (amendment 6, S2).
    pub fn input_format(self) -> InputFormat {
        match self {
            Codec::ProRes(p) if p.is_4444() => InputFormat::Y416,
            Codec::ProRes(_) => InputFormat::V216,
            Codec::H264 { .. } => InputFormat::V420,
        }
    }

    /// Nominal bit depth of the coded Y′CbCr samples.
    pub fn depth(self) -> u32 {
        match self {
            Codec::ProRes(p) if p.is_4444() => 12,
            Codec::ProRes(_) => 10,
            Codec::H264 { .. } => 8,
        }
    }
}

/// CoreVideo pixel formats the engine writes as encoder input.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputFormat {
    /// `y416`: 16-bit 4:4:4:4, A Y′ Cb Cr per pixel, little-endian u16. ProRes 4444.
    Y416,
    /// `v216`: 16-bit 4:2:2, Cb Y′0 Cr Y′1 per pixel pair, little-endian u16. ProRes 422.
    V216,
    /// `420v`: 8-bit 4:2:0 video range, a Y′ plane and an interleaved CbCr plane. H.264.
    V420,
}

impl InputFormat {
    pub fn fourcc(self) -> [u8; 4] {
        match self {
            InputFormat::Y416 => *b"y416",
            InputFormat::V216 => *b"v216",
            InputFormat::V420 => *b"420v",
        }
    }

    pub fn code(self) -> u32 {
        u32::from_be_bytes(self.fourcc())
    }
}

/// Output color tags (section 9, "Output tags"; amendment 7). Both are limited range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorTags {
    /// CICP 1-1-1: Rec.709 primaries, BT.709 transfer, BT.709 matrix.
    Rec709Broadcast,
    /// CICP 1-13-1: Rec.709 primaries, sRGB transfer, BT.709 matrix. "Rec.709
    /// (QuickTime and Safari)": matches the viewer in QuickTime and Safari (S2).
    Rec709QuickTime,
}

impl ColorTags {
    /// (colour primaries, transfer characteristics, matrix coefficients).
    pub fn cicp(self) -> (u16, u16, u16) {
        match self {
            ColorTags::Rec709Broadcast => (1, 1, 1),
            ColorTags::Rec709QuickTime => (1, 13, 1),
        }
    }
}

/// How alpha leaves the export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AlphaOutput {
    /// Opaque output: premultiplied working values, which is compositing over black.
    None,
    /// Straight (unassociated) alpha: color unpremultiplied (guarded) before encoding.
    Straight,
    /// Premultiplied alpha, premultiplied in the encoded domain.
    Premultiplied,
}

/// Working encoding of the frames handed to the exporter (section 9, "The rule").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkingEncoding {
    /// Linear blending off: working values are already E-encoded.
    Standard,
    /// Linear blending on: working values are linear light; E is applied on output.
    Linear,
}

/// Dither (section 17): blue-noise RPDF of ±0.5 LSB after the matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dither {
    /// On for 8-bit outputs, off for 10-bit and above.
    Default,
    On,
    Off,
}

/// What happens when the destination already exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Overwrite {
    /// Fail, leaving the existing file untouched (the move is no-clobber).
    Fail,
    /// Atomically replace the existing file.
    Replace,
    /// The destination is a temporary file host services created for this job
    /// (contract 14, `outputDestination`): the export is written beside it and
    /// renamed over it, and the host moves it into place after `commitOutput`.
    InPlace,
}

/// Linear PCM audio handed to the exporter: interleaved `f32` at `sample_rate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioSpec {
    pub sample_rate: u32,
    pub channels: u16,
}

/// A `tmcd` track (section 17, "Comfort": start timecode and reel name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TimecodeSpec {
    /// Frame number of the first frame in `format`'s counting (for example
    /// `format.parse("01:00:00;00")`).
    pub start_frame: i64,
    pub format: TimecodeFormat,
    /// Reel (source reference) name.
    pub reel: Option<String>,
}

/// One export.
#[derive(Clone, Debug)]
pub struct ExportSpec {
    pub width: u32,
    pub height: u32,
    pub rate: Rate,
    /// Frames to be delivered. Known up front: it sizes the timecode track and is
    /// what verification checks.
    pub frames: u64,
    pub codec: Codec,
    pub container: Container,
    pub tags: ColorTags,
    pub alpha: AlphaOutput,
    pub working: WorkingEncoding,
    pub dither: Dither,
    pub audio: Option<AudioSpec>,
    pub timecode: Option<TimecodeSpec>,
    pub destination: PathBuf,
    pub overwrite: Overwrite,
    /// Unix permission bits for the delivered file (agent exports use 0o600).
    pub mode: Option<u32>,
}

/// The largest frame dimension accepted (today's exporter: 8192).
pub const MAX_DIMENSION: u32 = 8192;

impl ExportSpec {
    /// Checks the combination is one this exporter produces.
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |m: &str| Err(Error::InvalidSpec(m.to_string()));
        if self.width < 2
            || self.height < 2
            || self.width > MAX_DIMENSION
            || self.height > MAX_DIMENSION
        {
            return invalid("frame size must be 2 to 8192 pixels on each side");
        }
        if !self.width.is_multiple_of(2) || !self.height.is_multiple_of(2) {
            return invalid("frame width and height must be even");
        }
        if self.frames == 0 {
            return invalid("an export needs at least one frame");
        }
        match self.codec {
            Codec::ProRes(_) if self.container != Container::Mov => {
                return invalid("ProRes is written to MOV");
            }
            Codec::H264 { bitrate_bps } if !(100_000..=800_000_000).contains(&bitrate_bps) => {
                return invalid("H.264 bit rate must be 0.1 to 800 Mbps");
            }
            _ => {}
        }
        if self.alpha != AlphaOutput::None && !matches!(self.codec, Codec::ProRes(p) if p.is_4444())
        {
            return invalid("alpha needs ProRes 4444 or 4444 XQ");
        }
        if let Some(audio) = self.audio {
            if !pm_time::SUPPORTED_SAMPLE_RATES.contains(&(audio.sample_rate as i64)) {
                return invalid("unsupported audio sample rate");
            }
            if audio.channels == 0 || audio.channels > 2 {
                return invalid("audio must be mono or stereo");
            }
        }
        if let Some(tc) = &self.timecode {
            // AVAssetWriter refuses a timecode track in MPEG-4 (measured on macOS 27).
            if self.container != Container::Mov {
                return invalid("timecode tracks are written to MOV only");
            }
            if tc.format.base() != TimecodeFormat::non_drop(self.rate).base() {
                return invalid("timecode must count the export's own frame rate");
            }
            if tc.start_frame < 0 || tc.start_frame >= tc.format.frames_per_day() {
                return invalid("start timecode must lie within one day");
            }
            if tc.reel.as_ref().is_some_and(|r| r.len() > 255) {
                return invalid("reel name is longer than 255 bytes");
            }
        }
        if self.destination.file_name().is_none() {
            return invalid("destination has no file name");
        }
        video_timing(self.rate)?;
        Ok(())
    }

    /// Movie and media timescales and the frame duration (section 17).
    pub fn timing(&self) -> Result<VideoTiming, Error> {
        video_timing(self.rate)
    }

    /// Whether the encoder input is dithered.
    pub fn dither_enabled(&self) -> bool {
        match self.dither {
            Dither::On => true,
            Dither::Off => false,
            Dither::Default => self.codec.depth() <= 8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn spec(codec: Codec) -> ExportSpec {
        ExportSpec {
            width: 64,
            height: 32,
            rate: Rate::FPS_23_976,
            frames: 3,
            codec,
            container: if matches!(codec, Codec::ProRes(_)) {
                Container::Mov
            } else {
                Container::Mp4
            },
            tags: ColorTags::Rec709QuickTime,
            alpha: AlphaOutput::None,
            working: WorkingEncoding::Standard,
            dither: Dither::Default,
            audio: None,
            timecode: None,
            destination: PathBuf::from("/tmp/out.mov"),
            overwrite: Overwrite::Fail,
            mode: None,
        }
    }

    #[test]
    fn encoder_inputs_follow_amendment_6() {
        assert_eq!(
            Codec::ProRes(ProResProfile::Hq).input_format(),
            InputFormat::V216
        );
        assert_eq!(
            Codec::ProRes(ProResProfile::Proxy).input_format(),
            InputFormat::V216
        );
        assert_eq!(
            Codec::ProRes(ProResProfile::P4444).input_format(),
            InputFormat::Y416
        );
        assert_eq!(
            Codec::ProRes(ProResProfile::P4444Xq).input_format(),
            InputFormat::Y416
        );
        assert_eq!(
            Codec::H264 {
                bitrate_bps: 16_000_000
            }
            .input_format(),
            InputFormat::V420
        );
        assert_eq!(InputFormat::V216.code(), 0x7632_3136);
    }

    #[test]
    fn dither_defaults_on_for_8_bit_only() {
        assert!(
            spec(Codec::H264 {
                bitrate_bps: 4_000_000
            })
            .dither_enabled()
        );
        assert!(!spec(Codec::ProRes(ProResProfile::Hq)).dither_enabled());
        let mut s = spec(Codec::ProRes(ProResProfile::Hq));
        s.dither = Dither::On;
        assert!(s.dither_enabled());
        s.codec = Codec::H264 {
            bitrate_bps: 4_000_000,
        };
        s.dither = Dither::Off;
        assert!(!s.dither_enabled());
    }

    #[test]
    fn validation_rejects_impossible_combinations() {
        assert!(spec(Codec::ProRes(ProResProfile::Hq)).validate().is_ok());
        let mut s = spec(Codec::ProRes(ProResProfile::Hq));
        s.alpha = AlphaOutput::Straight;
        assert!(s.validate().is_err(), "alpha on 4:2:2");
        s.codec = Codec::ProRes(ProResProfile::P4444);
        assert!(s.validate().is_ok());
        s.container = Container::Mp4;
        assert!(s.validate().is_err(), "ProRes in MP4");
        let mut s = spec(Codec::H264 {
            bitrate_bps: 16_000_000,
        });
        s.width = 63;
        assert!(s.validate().is_err(), "odd width");
        s.width = 64;
        s.frames = 0;
        assert!(s.validate().is_err(), "no frames");
        s.frames = 1;
        s.audio = Some(AudioSpec {
            sample_rate: 12_345,
            channels: 2,
        });
        assert!(s.validate().is_err(), "sample rate");
        s.audio = Some(AudioSpec {
            sample_rate: 48_000,
            channels: 2,
        });
        assert!(s.validate().is_ok());
        s.timecode = Some(TimecodeSpec {
            start_frame: 0,
            format: TimecodeFormat::for_rate(s.rate),
            reel: None,
        });
        assert!(s.validate().is_err(), "timecode in MP4");
        s.container = Container::Mov;
        assert!(s.validate().is_ok());
        s.timecode.as_mut().unwrap().start_frame = -1;
        assert!(s.validate().is_err(), "negative timecode");
        s.timecode.as_mut().unwrap().start_frame = 0;
        s.timecode.as_mut().unwrap().format = TimecodeFormat::for_rate(Rate::FPS_29_97);
        assert!(
            s.validate().is_err(),
            "timecode counting 30 labels on a 23.976 export"
        );
    }
}
