//! The job engine's WebCodecs encodes (decision EXPORT-1; contract 14).
//!
//! Today's exporter writes WebM through Chromium's own encoders: the
//! compositor canvas goes to a WebCodecs `VideoEncoder` (VP9 profile 0, else
//! VP8, the quality tier's bit rate, `latencyMode: "quality"`, a forced
//! keyframe every rounded two seconds), the `OfflineAudioContext` mix to an
//! `AudioEncoder` (Opus, 160 kbps, 20 ms chunks), and a small EBML writer
//! muxes the chunks. Other encoders (libvpx and libopus builds, other
//! settings) give other pictures and samples, so an opaque WebM export from
//! the job engine is encoded by those same Chromium encoders: the engine
//! renders every frame and mixes the audio, then hands them to the host, which
//! feeds them to the encoders in its encoder window and writes the muxed file
//! into the export's output.
//!
//! One encode per job, in order: [`EncodeStep::Open`], one
//! [`EncodeStep::Video`] per frame (index 0, 1, …, each answered once the
//! encoder has it), any number of [`EncodeStep::Audio`] chunks
//! (interleaved `f32`), then [`EncodeStep::Finish`] (answered once the
//! output file is written) or [`EncodeStep::Abort`]. Every step is answered
//! with `ack`; a failure is the error answer, after which the encode is gone.

use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::common::{Bytes, Count, FrameRate, JobId};
use crate::host::CapabilityHandle;
use crate::snapshot::{INLINE_PIXEL_BYTES, PixelPayload, shared_name_ok};
use crate::validate::{Validate, Violation, ensure};
use crate::version::ContractId;

/// Largest frame edge an encode takes (the export size limit).
pub const MAX_EDGE: u32 = 8192;
/// Largest audio chunk in one message: 2 s of 48 kHz stereo `f32`.
pub const MAX_AUDIO_BYTES: usize = 48_000 * 2 * 4 * 2;
/// Bit rates WebCodecs is asked for (the quality tiers are 4, 16 and 40 Mbps).
pub const BITRATES: std::ops::RangeInclusive<u32> = 100_000..=800_000_000;

/// The video codec. VP9 asks for `vp09.00.31.08` and falls back to `vp8`
/// where Chromium cannot encode VP9, as today's exporter does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub enum EncodeVideoCodec {
    Vp9,
    Vp8,
}

/// The audio track: the engine's mix at `sample_rate` with `channels`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct EncodeAudio {
    pub sample_rate: u32,
    pub channels: u8,
}

/// One step of an encode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    tag = "step"
)]
pub enum EncodeStep {
    /// Starts the encode. `output` is the export's temporary output (the
    /// `outputDestination` handle) the host writes the muxed file into.
    Open {
        codec: EncodeVideoCodec,
        width: u32,
        height: u32,
        frame_rate: FrameRate,
        bitrate: u32,
        frames: Count,
        audio: Option<EncodeAudio>,
        output: CapabilityHandle,
    },
    /// Frame `index`: top-down RGBA8, `width × height × 4` bytes,
    /// premultiplied and over black where the picture is transparent (the
    /// compositor canvas's bytes).
    Video { index: Count, pixels: PixelPayload },
    /// The next interleaved little-endian `f32` samples of the mix.
    Audio { samples: Bytes },
    /// Flushes the encoders, muxes, and writes the output.
    Finish,
    /// Abandons the encode; nothing is written.
    Abort,
}

/// Job engine → host: one step of job `job`'s encode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS))]
#[serde(rename_all = "camelCase")]
pub struct HostEncode {
    pub job: JobId,
    #[serde(flatten)]
    #[cfg_attr(feature = "ts", ts(flatten))]
    pub step: EncodeStep,
}

impl Validate for HostEncode {
    fn validate(&self) -> Result<(), Violation> {
        let c = ContractId::HostServices;
        match &self.step {
            EncodeStep::Open {
                width,
                height,
                frame_rate,
                bitrate,
                frames,
                audio,
                ..
            } => {
                ensure(
                    (2..=MAX_EDGE).contains(width)
                        && (2..=MAX_EDGE).contains(height)
                        && width % 2 == 0
                        && height % 2 == 0,
                    c,
                    || format!("an encode of {width}×{height}"),
                )?;
                ensure(frame_rate.is_valid(), c, || {
                    "invalid encode frame rate".into()
                })?;
                ensure(BITRATES.contains(bitrate), c, || {
                    format!("an encode at {bitrate} bits per second")
                })?;
                ensure(frames.get() >= 1, c, || "an encode of no frames".into())?;
                ensure(
                    audio.is_none_or(|a| {
                        (8_000..=192_000).contains(&a.sample_rate) && (1..=2).contains(&a.channels)
                    }),
                    c,
                    || "invalid encode audio".into(),
                )
            }
            EncodeStep::Video { pixels, .. } => match pixels {
                PixelPayload::Inline { bytes } => {
                    ensure(bytes.0.len() <= INLINE_PIXEL_BYTES, c, || {
                        "inline pixels over the inline limit".into()
                    })
                }
                PixelPayload::SharedMemory { name, length } => {
                    ensure(shared_name_ok(name), c, || {
                        "invalid shared-memory name".into()
                    })?;
                    ensure(
                        length.get() <= u64::from(MAX_EDGE) * u64::from(MAX_EDGE) * 4,
                        c,
                        || "an encode frame over the size limit".into(),
                    )
                }
            },
            EncodeStep::Audio { samples } => ensure(
                !samples.0.is_empty()
                    && samples.0.len() <= MAX_AUDIO_BYTES
                    && samples.0.len() % 4 == 0,
                c,
                || format!("{} audio bytes in one encode step", samples.0.len()),
            ),
            EncodeStep::Finish | EncodeStep::Abort => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::ByteSize;

    fn open(width: u32, bitrate: u32) -> HostEncode {
        HostEncode {
            job: JobId::from("job-1"),
            step: EncodeStep::Open {
                codec: EncodeVideoCodec::Vp9,
                width,
                height: 180,
                frame_rate: FrameRate::new(30_000, 1001),
                bitrate,
                frames: Count::new(30).unwrap(),
                audio: Some(EncodeAudio {
                    sample_rate: 48_000,
                    channels: 2,
                }),
                output: CapabilityHandle::from_random_bytes([7; 16]),
            },
        }
    }

    #[test]
    fn steps_validate_their_limits() {
        assert!(open(320, 16_000_000).validate().is_ok());
        assert!(open(321, 16_000_000).validate().is_err(), "odd width");
        assert!(open(320, 10).validate().is_err(), "bit rate");
        let video = |pixels| HostEncode {
            job: JobId::from("job-1"),
            step: EncodeStep::Video {
                index: Count::ZERO,
                pixels,
            },
        };
        assert!(
            video(PixelPayload::Inline {
                bytes: Bytes(vec![0; 16])
            })
            .validate()
            .is_ok()
        );
        assert!(
            video(PixelPayload::SharedMemory {
                name: "/tmp/x".into(),
                length: ByteSize::new(16).unwrap()
            })
            .validate()
            .is_err()
        );
        let audio = |n| HostEncode {
            job: JobId::from("job-1"),
            step: EncodeStep::Audio {
                samples: Bytes(vec![0; n]),
            },
        };
        assert!(audio(3840).validate().is_ok());
        assert!(audio(3841).validate().is_err(), "whole f32 samples");
        assert!(audio(0).validate().is_err());
        assert!(audio(MAX_AUDIO_BYTES + 4).validate().is_err());
    }

    #[test]
    fn steps_are_flat_and_tagged() {
        let json = serde_json::to_value(open(320, 16_000_000)).unwrap();
        assert_eq!(json["step"], "open");
        assert_eq!(json["job"], "job-1");
        assert_eq!(json["frameRate"]["den"], 1001);
        assert_eq!(json["codec"], "vp9");
        let finish = HostEncode {
            job: JobId::from("job-1"),
            step: EncodeStep::Finish,
        };
        let bytes = rmp_serde::to_vec_named(&finish).unwrap();
        assert_eq!(rmp_serde::from_slice::<HostEncode>(&bytes).unwrap(), finish);
    }
}
