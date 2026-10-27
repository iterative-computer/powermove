//! What an export produces: codec, container, color tags, alpha, dither, audio,
//! timecode, destination (plan section 17, "Formats at cutover"; section 9, "Output
//! tags"; section 26 amendments 6 and 7).
//!
//! | Format | Codec | Container | Encoder input | Audio |
//! |---|---|---|---|---|
//! | ProRes 422 Proxy, LT, 422, HQ | VideoToolbox | MOV | `v216` | PCM 16, 24, 32f; AAC |
//! | ProRes 4444, 4444 XQ (alpha straight or premultiplied) | VideoToolbox | MOV | `y416` | PCM; AAC |
//! | H.264 High | VideoToolbox | MP4 (MOV) | `420v` | AAC (PCM in MOV) |
//! | HEVC Main, Main10; Main with alpha | VideoToolbox | MP4, MOV | `420v`, `x420`, `v0a8` | AAC (PCM in MOV) |
//! | VP9, VP8 (alpha) | libvpx through FFmpeg | WebM | `yuv420p`, `yuva420p` | Opus |
//! | PNG sequence and still, 8 or 16-bit | engine (`png`) | folder, file | RGBA | — |

use std::path::PathBuf;

use pm_time::{Rate, TimecodeFormat};

use crate::Error;
use crate::timescale::{VideoTiming, video_timing};

/// Container. MOV and MP4 are muxed by AVAssetWriter, WebM by FFmpeg (section 17);
/// PNG outputs are written by the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Container {
    Mov,
    Mp4,
    Webm,
    /// A folder of numbered PNG files (`<name>_00000.png`, …).
    PngSequence,
    /// One PNG file.
    PngStill,
}

impl Container {
    pub fn extension(self) -> &'static str {
        match self {
            Container::Mov => "mov",
            Container::Mp4 => "mp4",
            Container::Webm => "webm",
            // A sequence is a folder; its files are PNGs.
            Container::PngSequence | Container::PngStill => "png",
        }
    }

    /// Whether the container is muxed by AVAssetWriter.
    pub fn is_quicktime_family(self) -> bool {
        matches!(self, Container::Mov | Container::Mp4)
    }

    /// Whether the output is a folder rather than one file.
    pub fn is_folder(self) -> bool {
        self == Container::PngSequence
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

    pub fn name(self) -> &'static str {
        match self {
            ProResProfile::Proxy => "ProRes 422 Proxy",
            ProResProfile::Lt => "ProRes 422 LT",
            ProResProfile::Standard => "ProRes 422",
            ProResProfile::Hq => "ProRes 422 HQ",
            ProResProfile::P4444 => "ProRes 4444",
            ProResProfile::P4444Xq => "ProRes 4444 XQ",
        }
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
    /// HEVC Main (8-bit) or Main10 (10-bit), 4:2:0 SDR. With alpha the stream is
    /// HEVC with alpha at 8 bits (S2: 10-bit alpha streams do not play back on
    /// macOS 27).
    Hevc {
        bitrate_bps: u32,
        ten_bit: bool,
    },
    /// VP9 profile 0 (8-bit 4:2:0) through libvpx, alpha as a second stream.
    Vp9 {
        bitrate_bps: u32,
    },
    /// VP8 through libvpx, alpha as a second stream.
    Vp8 {
        bitrate_bps: u32,
    },
    /// PNG, 8 or 16 bits per channel, RGB or RGBA (straight alpha).
    Png {
        sixteen_bit: bool,
    },
}

impl Codec {
    /// The sample entry (QuickTime family), Matroska-style id (WebM), or chunk
    /// signature this codec is recognized by.
    pub fn fourcc(self) -> [u8; 4] {
        match self {
            Codec::ProRes(p) => p.fourcc(),
            Codec::H264 { .. } => *b"avc1",
            Codec::Hevc { .. } => *b"hvc1",
            Codec::Vp9 { .. } => *b"vp09",
            Codec::Vp8 { .. } => *b"vp08",
            Codec::Png { .. } => *b"png ",
        }
    }

    /// The encoder input the engine hands the encoder (amendment 6, S2).
    pub fn input_format(self, alpha: bool) -> InputFormat {
        match self {
            Codec::ProRes(p) if p.is_4444() => InputFormat::Y416,
            Codec::ProRes(_) => InputFormat::V216,
            Codec::H264 { .. } => InputFormat::V420,
            Codec::Hevc { ten_bit: true, .. } => InputFormat::X420,
            Codec::Hevc { .. } if alpha => InputFormat::V0a8,
            Codec::Hevc { .. } => InputFormat::V420,
            Codec::Vp9 { .. } | Codec::Vp8 { .. } if alpha => InputFormat::I420a,
            Codec::Vp9 { .. } | Codec::Vp8 { .. } => InputFormat::I420,
            Codec::Png { sixteen_bit: true } => InputFormat::Rgba16,
            Codec::Png { .. } => InputFormat::Rgba8,
        }
    }

    /// Nominal bit depth of the coded samples.
    pub fn depth(self) -> u32 {
        match self {
            Codec::ProRes(p) if p.is_4444() => 12,
            Codec::ProRes(_) => 10,
            Codec::Hevc { ten_bit: true, .. } => 10,
            Codec::H264 { .. } | Codec::Hevc { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. } => 8,
            Codec::Png { sixteen_bit: true } => 16,
            Codec::Png { .. } => 8,
        }
    }

    /// Whether the codec can carry alpha (with the depth it would be coded at).
    pub fn supports_alpha(self) -> bool {
        match self {
            Codec::ProRes(p) => p.is_4444(),
            Codec::H264 { .. } => false,
            Codec::Hevc { ten_bit, .. } => !ten_bit,
            Codec::Vp9 { .. } | Codec::Vp8 { .. } | Codec::Png { .. } => true,
        }
    }

    /// Whether the codec writes Y′CbCr (limited range); PNG writes full-range RGB.
    pub fn is_ycbcr(self) -> bool {
        !matches!(self, Codec::Png { .. })
    }

    /// The codec's average bit rate, for codecs with rate control.
    pub fn bitrate_bps(self) -> Option<u32> {
        match self {
            Codec::H264 { bitrate_bps }
            | Codec::Hevc { bitrate_bps, .. }
            | Codec::Vp9 { bitrate_bps }
            | Codec::Vp8 { bitrate_bps } => Some(bitrate_bps),
            Codec::ProRes(_) | Codec::Png { .. } => None,
        }
    }

    pub fn name(self) -> String {
        match self {
            Codec::ProRes(p) => p.name().to_string(),
            Codec::H264 { .. } => "H.264 High".into(),
            Codec::Hevc { ten_bit: true, .. } => "HEVC Main10".into(),
            Codec::Hevc { .. } => "HEVC Main".into(),
            Codec::Vp9 { .. } => "VP9".into(),
            Codec::Vp8 { .. } => "VP8".into(),
            Codec::Png { sixteen_bit: true } => "PNG 16-bit".into(),
            Codec::Png { .. } => "PNG 8-bit".into(),
        }
    }
}

/// Pixel formats the engine writes as encoder input. The CoreVideo formats carry
/// their CoreVideo four-character code; the FFmpeg and PNG layouts are the
/// engine's own (tightly packed planes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InputFormat {
    /// `y416`: 16-bit 4:4:4:4, A Y′ Cb Cr per pixel, little-endian u16. ProRes 4444.
    Y416,
    /// `v216`: 16-bit 4:2:2, Cb Y′0 Cr Y′1 per pixel pair, little-endian u16. ProRes 422.
    V216,
    /// `420v`: 8-bit 4:2:0 video range, a Y′ plane and an interleaved CbCr plane.
    /// H.264 and HEVC Main.
    V420,
    /// `x420`: 10-bit 4:2:0 video range, a Y′ plane and an interleaved CbCr plane,
    /// each sample in the high 10 bits of a little-endian u16. HEVC Main10.
    X420,
    /// `v0a8`: `420v` plus an 8-bit full-range alpha plane. HEVC with alpha
    /// (S2: the engine writes it directly so it owns the 4:2:0 filter).
    V0a8,
    /// FFmpeg `yuv420p`: 8-bit 4:2:0 limited range, Y′, Cb, Cr planes. libvpx.
    I420,
    /// FFmpeg `yuva420p`: `yuv420p` plus an 8-bit full-range alpha plane. libvpx.
    I420a,
    /// 8-bit RGBA, full range, straight alpha (255 when opaque). PNG.
    Rgba8,
    /// 16-bit RGBA, full range, straight alpha, native-endian u16. PNG.
    Rgba16,
}

impl InputFormat {
    pub fn fourcc(self) -> [u8; 4] {
        match self {
            InputFormat::Y416 => *b"y416",
            InputFormat::V216 => *b"v216",
            InputFormat::V420 => *b"420v",
            InputFormat::X420 => *b"x420",
            InputFormat::V0a8 => *b"v0a8",
            InputFormat::I420 => *b"y420",
            InputFormat::I420a => *b"a420",
            InputFormat::Rgba8 => *b"RGBA",
            InputFormat::Rgba16 => *b"l64r",
        }
    }

    pub fn code(self) -> u32 {
        u32::from_be_bytes(self.fourcc())
    }

    /// Whether the format has an alpha channel.
    pub fn has_alpha(self) -> bool {
        matches!(
            self,
            InputFormat::Y416
                | InputFormat::V0a8
                | InputFormat::I420a
                | InputFormat::Rgba8
                | InputFormat::Rgba16
        )
    }

    /// Whether CoreVideo pools and VideoToolbox take this format.
    pub fn is_core_video(self) -> bool {
        matches!(
            self,
            InputFormat::Y416
                | InputFormat::V216
                | InputFormat::V420
                | InputFormat::X420
                | InputFormat::V0a8
        )
    }
}

/// Output color tags (section 9, "Output tags"; amendment 7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorTags {
    /// CICP 1-1-1, limited: Rec.709 primaries, BT.709 transfer, BT.709 matrix.
    Rec709Broadcast,
    /// CICP 1-13-1, limited: Rec.709 primaries, sRGB transfer, BT.709 matrix.
    /// "Rec.709 (QuickTime and Safari)": matches the viewer in QuickTime and Safari (S2).
    Rec709QuickTime,
    /// CICP 1-13-0, full range RGB: sRGB images (PNG `sRGB` and `cICP`).
    SrgbImage,
    /// CICP 1-1-0, full range RGB: Rec.709 images (PNG `cICP`).
    Rec709Image,
}

impl ColorTags {
    /// (colour primaries, transfer characteristics, matrix coefficients).
    pub fn cicp(self) -> (u16, u16, u16) {
        match self {
            ColorTags::Rec709Broadcast => (1, 1, 1),
            ColorTags::Rec709QuickTime => (1, 13, 1),
            ColorTags::SrgbImage => (1, 13, 0),
            ColorTags::Rec709Image => (1, 1, 0),
        }
    }

    /// Image tags describe full-range RGB; video tags limited-range Y′CbCr.
    pub fn is_image(self) -> bool {
        matches!(self, ColorTags::SrgbImage | ColorTags::Rec709Image)
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
    /// On for final 8-bit images (8-bit PNG); off for lossy codecs, which would
    /// code the noise as content (EXPORT-DITHER-1), and for 10 bits and above.
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
    /// The destination is a temporary file (or, for PNG sequences, folder) host
    /// services created for this job (contract 14, `outputDestination`): the
    /// export is written beside it and renamed over it, and the host moves it into
    /// place after `commitOutput`.
    InPlace,
}

/// The audio track's sample coding (section 16, "Export").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// 16-bit LPCM with deterministic TPDF dither (±1 LSB triangular, keyed by
    /// sample index and channel: `pm_audio::offline::tpdf`).
    Pcm16,
    /// 24-bit LPCM, rounded (its quantization sits at −144 dBFS; no dither).
    Pcm24,
    /// 32-bit float LPCM, the mix as it is.
    PcmF32,
    /// AAC LC through AudioToolbox at 128 kbps per channel, from the f32 mix,
    /// with an edit list for its priming.
    Aac,
    /// Opus through libopus at 48 kHz (WebM), 160 kbps for the stream with
    /// Chromium WebCodecs' AudioEncoder settings (20 ms, VBR, complexity 9).
    Opus,
}

impl AudioCodec {
    pub fn is_pcm(self) -> bool {
        matches!(
            self,
            AudioCodec::Pcm16 | AudioCodec::Pcm24 | AudioCodec::PcmF32
        )
    }

    pub fn name(self) -> &'static str {
        match self {
            AudioCodec::Pcm16 => "16-bit PCM",
            AudioCodec::Pcm24 => "24-bit PCM",
            AudioCodec::PcmF32 => "32-bit float PCM",
            AudioCodec::Aac => "AAC",
            AudioCodec::Opus => "Opus",
        }
    }
}

/// Audio handed to the exporter: interleaved `f32` at `sample_rate`, coded as `codec`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AudioSpec {
    pub sample_rate: u32,
    pub channels: u16,
    pub codec: AudioCodec,
}

impl AudioSpec {
    /// Today's audio for a container: 24-bit PCM in MOV, AAC in MP4, Opus in WebM.
    pub fn default_for(container: Container, sample_rate: u32, channels: u16) -> AudioSpec {
        let codec = match container {
            Container::Mov => AudioCodec::Pcm24,
            Container::Webm => AudioCodec::Opus,
            _ => AudioCodec::Aac,
        };
        AudioSpec {
            sample_rate,
            channels,
            codec,
        }
    }
}

/// The only sample rate Opus in WebM is written at.
pub const OPUS_SAMPLE_RATE: u32 = 48_000;

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
    /// The output file, or for PNG sequences the folder that will hold the frames.
    pub destination: PathBuf,
    pub overwrite: Overwrite,
    /// Unix permission bits for the delivered file (agent exports use 0o600).
    pub mode: Option<u32>,
}

/// The largest frame dimension accepted by the video encoders (VideoToolbox:
/// 8192 for H.264 and HEVC; today's exporter's limit for every format).
pub const MAX_DIMENSION: u32 = 8192;
/// PNG outputs may be as large as the export settings allow.
pub const MAX_IMAGE_DIMENSION: u32 = 16384;
/// VP8's frame size limit (14-bit dimensions).
pub const MAX_VP8_DIMENSION: u32 = 16383;

impl ExportSpec {
    /// The encoder input of this export.
    pub fn input_format(&self) -> InputFormat {
        self.codec.input_format(self.alpha != AlphaOutput::None)
    }

    /// Checks the combination is one this exporter produces.
    pub fn validate(&self) -> Result<(), Error> {
        let invalid = |m: &str| Err(Error::InvalidSpec(m.to_string()));
        let max = match self.codec {
            Codec::Png { .. } => MAX_IMAGE_DIMENSION,
            _ => MAX_DIMENSION,
        };
        if self.width < 1 || self.height < 1 || self.width > max || self.height > max {
            return invalid(&format!(
                "frame size must be 1 to {max} pixels on each side"
            ));
        }
        let subsampled = !matches!(
            self.input_format(),
            InputFormat::Y416 | InputFormat::Rgba8 | InputFormat::Rgba16
        );
        if subsampled && (self.width < 2 || self.height < 2) {
            return invalid("video frames are at least 2 pixels on each side");
        }
        if subsampled && (!self.width.is_multiple_of(2) || !self.height.is_multiple_of(2)) {
            return invalid("frame width and height must be even");
        }
        if self.frames == 0 {
            return invalid("an export needs at least one frame");
        }
        let containers: &[Container] = match self.codec {
            Codec::ProRes(_) => &[Container::Mov],
            Codec::H264 { .. } | Codec::Hevc { .. } => &[Container::Mp4, Container::Mov],
            Codec::Vp9 { .. } | Codec::Vp8 { .. } => &[Container::Webm],
            Codec::Png { .. } => &[Container::PngSequence, Container::PngStill],
        };
        if !containers.contains(&self.container) {
            return invalid(&format!(
                "{} is not written to {:?}",
                self.codec.name(),
                self.container
            ));
        }
        if self.container == Container::PngStill && self.frames != 1 {
            return invalid("a still is one frame");
        }
        if let Some(bitrate) = self.codec.bitrate_bps()
            && !(100_000..=800_000_000).contains(&bitrate)
        {
            return invalid("the bit rate must be 0.1 to 800 Mbps");
        }
        if matches!(self.codec, Codec::Vp8 { .. })
            && (self.width > MAX_VP8_DIMENSION || self.height > MAX_VP8_DIMENSION)
        {
            return invalid("VP8 frames are at most 16383 pixels on each side");
        }
        if self.alpha != AlphaOutput::None && !self.codec.supports_alpha() {
            return invalid(match self.codec {
                Codec::Hevc { .. } => "HEVC carries alpha at 8 bits only",
                _ => "alpha needs ProRes 4444, ProRes 4444 XQ, HEVC, WebM, or PNG",
            });
        }
        if self.alpha == AlphaOutput::Premultiplied
            && matches!(
                self.codec,
                Codec::Png { .. } | Codec::Vp9 { .. } | Codec::Vp8 { .. }
            )
        {
            return invalid("PNG and WebM carry straight alpha");
        }
        if self.tags.is_image() != matches!(self.codec, Codec::Png { .. }) {
            return invalid("images use image tags and video uses video tags");
        }
        if let Some(audio) = self.audio {
            if !pm_time::SUPPORTED_SAMPLE_RATES.contains(&(audio.sample_rate as i64)) {
                return invalid("unsupported audio sample rate");
            }
            if audio.channels == 0 || audio.channels > 2 {
                return invalid("audio must be mono or stereo");
            }
            let fits = match self.container {
                Container::Mov => audio.codec != AudioCodec::Opus,
                Container::Mp4 => audio.codec == AudioCodec::Aac,
                Container::Webm => audio.codec == AudioCodec::Opus,
                Container::PngSequence | Container::PngStill => false,
            };
            if !fits {
                return invalid(&format!(
                    "{} audio cannot go in {:?}",
                    audio.codec.name(),
                    self.container
                ));
            }
            if audio.codec == AudioCodec::Opus && audio.sample_rate != OPUS_SAMPLE_RATE {
                return invalid("Opus audio is mixed at 48 kHz");
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
            if tc.format.is_drop_frame()
                && self.rate != Rate::FPS_29_97
                && self.rate != Rate::FPS_59_94
            {
                return invalid("drop-frame timecode needs 29.97 or 59.94 fps");
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
        if self.container != Container::PngSequence && self.container != Container::PngStill {
            video_timing(self.rate)?;
        }
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
            Dither::Default => matches!(self.codec, Codec::Png { sixteen_bit: false }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn spec(codec: Codec) -> ExportSpec {
        let container = match codec {
            Codec::ProRes(_) => Container::Mov,
            Codec::Vp9 { .. } | Codec::Vp8 { .. } => Container::Webm,
            Codec::Png { .. } => Container::PngSequence,
            _ => Container::Mp4,
        };
        ExportSpec {
            width: 64,
            height: 32,
            rate: Rate::FPS_23_976,
            frames: 3,
            codec,
            container,
            tags: if matches!(codec, Codec::Png { .. }) {
                ColorTags::SrgbImage
            } else {
                ColorTags::Rec709QuickTime
            },
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
    fn encoder_inputs_follow_amendment_6_and_s2() {
        let hq = Codec::ProRes(ProResProfile::Hq);
        assert_eq!(hq.input_format(false), InputFormat::V216);
        assert_eq!(
            Codec::ProRes(ProResProfile::Proxy).input_format(false),
            InputFormat::V216
        );
        assert_eq!(
            Codec::ProRes(ProResProfile::P4444).input_format(true),
            InputFormat::Y416
        );
        assert_eq!(
            Codec::ProRes(ProResProfile::P4444Xq).input_format(false),
            InputFormat::Y416
        );
        let h264 = Codec::H264 {
            bitrate_bps: 16_000_000,
        };
        assert_eq!(h264.input_format(false), InputFormat::V420);
        let hevc = |ten_bit| Codec::Hevc {
            bitrate_bps: 16_000_000,
            ten_bit,
        };
        assert_eq!(hevc(false).input_format(false), InputFormat::V420);
        assert_eq!(hevc(false).input_format(true), InputFormat::V0a8);
        assert_eq!(hevc(true).input_format(false), InputFormat::X420);
        let vp9 = Codec::Vp9 {
            bitrate_bps: 16_000_000,
        };
        assert_eq!(vp9.input_format(false), InputFormat::I420);
        assert_eq!(vp9.input_format(true), InputFormat::I420a);
        assert_eq!(
            Codec::Png { sixteen_bit: true }.input_format(true),
            InputFormat::Rgba16
        );
        assert_eq!(InputFormat::V216.code(), 0x7632_3136);
        assert_eq!(hevc(true).depth(), 10);
        assert_eq!(Codec::Png { sixteen_bit: true }.depth(), 16);
    }

    #[test]
    fn dither_defaults_on_for_8_bit_png_only() {
        for lossy in [
            Codec::H264 {
                bitrate_bps: 4_000_000,
            },
            Codec::Hevc {
                bitrate_bps: 4_000_000,
                ten_bit: false,
            },
            Codec::Hevc {
                bitrate_bps: 4_000_000,
                ten_bit: true,
            },
            Codec::Vp9 {
                bitrate_bps: 4_000_000,
            },
            Codec::Vp8 {
                bitrate_bps: 4_000_000,
            },
        ] {
            assert!(!spec(lossy).dither_enabled(), "{lossy:?}");
        }
        assert!(spec(Codec::Png { sixteen_bit: false }).dither_enabled());
        assert!(!spec(Codec::Png { sixteen_bit: true }).dither_enabled());
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
            codec: AudioCodec::Aac,
        });
        assert!(s.validate().is_err(), "sample rate");
        s.audio = Some(AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::Aac,
        });
        assert!(s.validate().is_ok());
        s.audio = Some(AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::Pcm24,
        });
        assert!(s.validate().is_err(), "PCM in MP4");
        s.audio = None;
        s.timecode = Some(TimecodeSpec {
            start_frame: 0,
            format: TimecodeFormat::for_rate(s.rate),
            reel: None,
        });
        assert!(s.validate().is_err(), "timecode in MP4");
        s.container = Container::Mov;
        assert!(s.validate().is_ok(), "H.264 in MOV with timecode");
        s.timecode.as_mut().unwrap().start_frame = -1;
        assert!(s.validate().is_err(), "negative timecode");
        s.timecode.as_mut().unwrap().start_frame = 0;
        s.timecode.as_mut().unwrap().format = TimecodeFormat::for_rate(Rate::FPS_29_97);
        assert!(
            s.validate().is_err(),
            "timecode counting 30 labels on a 23.976 export"
        );
        s.rate = Rate::FPS_30;
        s.timecode.as_mut().unwrap().format = TimecodeFormat::new(30, true).unwrap();
        assert!(s.validate().is_err(), "drop-frame at 30 fps");
    }

    #[test]
    fn hevc_webm_and_png_combinations() {
        let mut s = spec(Codec::Hevc {
            bitrate_bps: 16_000_000,
            ten_bit: true,
        });
        assert!(s.validate().is_ok());
        s.alpha = AlphaOutput::Straight;
        assert!(s.validate().is_err(), "10-bit HEVC alpha");
        s.codec = Codec::Hevc {
            bitrate_bps: 16_000_000,
            ten_bit: false,
        };
        assert!(s.validate().is_ok(), "8-bit HEVC alpha");
        s.container = Container::Mov;
        assert!(s.validate().is_ok(), "HEVC in MOV");

        let mut s = spec(Codec::Vp9 {
            bitrate_bps: 16_000_000,
        });
        s.alpha = AlphaOutput::Straight;
        assert!(s.validate().is_ok());
        s.alpha = AlphaOutput::Premultiplied;
        assert!(s.validate().is_err(), "premultiplied WebM");
        s.alpha = AlphaOutput::None;
        s.audio = Some(AudioSpec {
            sample_rate: 44_100,
            channels: 2,
            codec: AudioCodec::Opus,
        });
        assert!(s.validate().is_err(), "Opus at 44.1 kHz");
        s.audio = Some(AudioSpec {
            sample_rate: 48_000,
            channels: 2,
            codec: AudioCodec::Opus,
        });
        assert!(s.validate().is_ok());
        s.container = Container::Mp4;
        assert!(s.validate().is_err(), "VP9 in MP4");

        let mut s = spec(Codec::Png { sixteen_bit: true });
        s.width = 33;
        s.height = 17;
        assert!(s.validate().is_ok(), "odd PNG sizes are fine");
        s.tags = ColorTags::Rec709QuickTime;
        assert!(s.validate().is_err(), "video tags on PNG");
        s.tags = ColorTags::Rec709Image;
        assert!(s.validate().is_ok());
        s.container = Container::PngStill;
        assert!(s.validate().is_err(), "a still of three frames");
        s.frames = 1;
        assert!(s.validate().is_ok());
        s.audio = Some(AudioSpec::default_for(Container::Mov, 48_000, 2));
        assert!(s.validate().is_err(), "audio in a PNG");
        let mut s = spec(Codec::H264 {
            bitrate_bps: 16_000_000,
        });
        s.tags = ColorTags::SrgbImage;
        assert!(s.validate().is_err(), "image tags on video");
    }
}
