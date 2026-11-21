//! Portable export contract mapping shared by the sheet and job renderer.
use crate::settings::{AudioCodec, Codec, Container, ProResProfile};
use pm_protocol::priority::{
    AudioCoding, Container as WireContainer, ExportFormat, ExportSettings,
    ProResProfile as WireProRes, WebmCodec,
};

/// The encoder and container for `settings` (section 17, "Formats at cutover").
pub fn codec_of(settings: &ExportSettings) -> (Codec, Container) {
    let bitrate_bps = settings.quality.bitrate_mbps() * 1_000_000;
    match settings.format {
        ExportFormat::ProRes { profile } => (
            Codec::ProRes(match profile {
                WireProRes::Proxy => ProResProfile::Proxy,
                WireProRes::Lt => ProResProfile::Lt,
                WireProRes::Standard => ProResProfile::Standard,
                WireProRes::Hq => ProResProfile::Hq,
                WireProRes::P4444 => ProResProfile::P4444,
                WireProRes::P4444Xq => ProResProfile::P4444Xq,
            }),
            Container::Mov,
        ),
        ExportFormat::H264 => (Codec::H264 { bitrate_bps }, Container::Mp4),
        ExportFormat::H264Mov => (Codec::H264 { bitrate_bps }, Container::Mov),
        ExportFormat::Hevc { ten_bit, container } => (
            Codec::Hevc {
                bitrate_bps,
                ten_bit,
            },
            match container {
                WireContainer::Mp4 => Container::Mp4,
                WireContainer::Mov => Container::Mov,
            },
        ),
        ExportFormat::Webm { codec } => (
            match codec {
                WebmCodec::Vp9 => Codec::Vp9 { bitrate_bps },
                WebmCodec::Vp8 => Codec::Vp8 { bitrate_bps },
            },
            Container::Webm,
        ),
        ExportFormat::PngSequence { sixteen_bit } => {
            (Codec::Png { sixteen_bit }, Container::PngSequence)
        }
        ExportFormat::PngStill { sixteen_bit } => (Codec::Png { sixteen_bit }, Container::PngStill),
    }
}

/// The audio coding of an export: the delivery option, or today's audio for
/// the container (24-bit PCM in MOV, AAC in MP4, Opus in WebM).
pub fn audio_codec_of(settings: &ExportSettings, container: Container) -> AudioCodec {
    match (container, settings.delivery.audio_coding) {
        (Container::Webm, _) => AudioCodec::Opus,
        (_, Some(AudioCoding::Pcm16)) => AudioCodec::Pcm16,
        (_, Some(AudioCoding::Pcm24)) => AudioCodec::Pcm24,
        (_, Some(AudioCoding::PcmFloat)) => AudioCodec::PcmF32,
        (_, Some(AudioCoding::Aac)) => AudioCodec::Aac,
        (Container::Mov, None) => AudioCodec::Pcm24,
        _ => AudioCodec::Aac,
    }
}
