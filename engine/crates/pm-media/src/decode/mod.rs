//! Decoders behind one interface, backend selection, sessions with exact
//! random access, hardware arbitration, and the decode service.
//!
//! Backend order per codec (section 13, decision 12, section 26 amendments
//! 6 and 13):
//!
//! | Codec | Backends, in order |
//! |---|---|
//! | ProRes (all) | VideoToolbox only |
//! | H.264 4:2:0 | VideoToolbox, FFmpeg |
//! | H.264 4:2:2 or 4:4:4 | FFmpeg, VideoToolbox (section 13: software) |
//! | HEVC, HEVC with alpha | VideoToolbox, FFmpeg |
//! | AV1 | VideoToolbox where the GPU decodes AV1 (M3 and later), dav1d |
//! | VP8, VP9 | FFmpeg native; with alpha, two paired FFmpeg native decoders |
//! | Everything else (DNxHD/DNxHR, CineForm, MJPEG, DV, MPEG-2, ...) | FFmpeg |

pub mod arbiter;
pub mod scrub;
pub mod service;
pub mod session;
pub mod skip;

use crate::dav1d::Dav1dDecoder;
use crate::error::{MediaError, Result};
use crate::ff::alpha::AlphaPairDecoder;
use crate::ff::decoder::FfDecoder;
use crate::ff::{CodecInfo, Input, Packet};
use crate::frame::DecodedFrame;
use crate::index::FrameIndex;
use crate::pool::FramePool;
use crate::vt::{DecodePurpose, VtConfig, VtDecoder};
use arbiter::{HardwareArbiter, HwCodec, HwPermit};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Backend {
    VideoToolbox,
    Ffmpeg,
    Dav1d,
    /// VP8 or VP9 with alpha: two paired FFmpeg native decoders.
    FfmpegAlphaPair,
}

/// What an open decoder is.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecoderInfo {
    pub backend: Backend,
    pub name: String,
    pub hardware: bool,
    /// Frames reach GPU-shareable memory without a copy.
    pub zero_copy: bool,
    /// VideoToolbox output format, when it applies.
    pub output: Option<String>,
}

/// One open decoder.
pub enum Decoder {
    Vt {
        dec: VtDecoder,
        cfg: VtConfig,
        _permit: Option<HwPermit>,
        /// Accounts for the CoreVideo buffers this decoder outputs.
        pool: FramePool,
    },
    Ff(FfDecoder),
    Dav1d(Dav1dDecoder),
    Alpha(AlphaPairDecoder),
}

impl Decoder {
    pub fn info(&self) -> DecoderInfo {
        match self {
            Decoder::Vt { dec, cfg, .. } => DecoderInfo {
                backend: Backend::VideoToolbox,
                name: format!("VideoToolbox {}", cfg.codec_type),
                hardware: dec.hardware(),
                zero_copy: true,
                output: Some(cfg.pixel_format.clone()),
            },
            Decoder::Ff(d) => DecoderInfo {
                backend: Backend::Ffmpeg,
                name: d.name().to_string(),
                hardware: false,
                zero_copy: d.direct(),
                output: None,
            },
            Decoder::Dav1d(_) => DecoderInfo {
                backend: Backend::Dav1d,
                name: "dav1d".into(),
                hardware: false,
                zero_copy: true,
                output: None,
            },
            Decoder::Alpha(d) => DecoderInfo {
                backend: Backend::FfmpegAlphaPair,
                name: d.name(),
                hardware: false,
                zero_copy: true,
                output: None,
            },
        }
    }

    /// Sends one sample; appends any frames that are ready.
    pub fn send(&mut self, p: &Packet, out: &mut Vec<DecodedFrame>) -> Result<()> {
        match self {
            Decoder::Vt { dec, pool, .. } => {
                let from = out.len();
                let r = dec.send(p, out);
                track(pool, &mut out[from..]);
                r
            }
            Decoder::Ff(d) => d.send(Some(p), out),
            Decoder::Dav1d(d) => d.send(Some(p), out),
            Decoder::Alpha(d) => d.send(Some(p), out),
        }
    }

    /// Whether every sent sample's frame is available after [`sync`]
    /// without further input (no reorder delay inside the decoder).
    ///
    /// [`sync`]: Decoder::sync
    pub fn syncs(&self) -> bool {
        matches!(self, Decoder::Vt { .. } | Decoder::Dav1d(_))
    }

    /// Outputs every frame the decoder can produce from the samples sent so
    /// far without ending the stream (VideoToolbox waits for asynchronous
    /// frames; dav1d drains its frame threads).
    pub fn sync(&mut self, out: &mut Vec<DecodedFrame>) -> Result<()> {
        match self {
            Decoder::Vt { dec, pool, .. } => {
                let from = out.len();
                let r = dec.sync(out);
                track(pool, &mut out[from..]);
                r
            }
            Decoder::Dav1d(d) => d.send(None, out),
            _ => Ok(()),
        }
    }

    /// End of stream: outputs every remaining frame. The decoder must be
    /// [`reset`](Decoder::reset) before more input.
    pub fn drain(&mut self, out: &mut Vec<DecodedFrame>) -> Result<()> {
        match self {
            Decoder::Vt { .. } => self.sync(out),
            Decoder::Ff(d) => d.send(None, out),
            Decoder::Dav1d(d) => d.send(None, out),
            Decoder::Alpha(d) => d.send(None, out),
        }
    }

    /// Prepares for decoding from another random-access point: an
    /// end-of-sequence sample for VideoToolbox, a flush for software decoders.
    pub fn reset(&mut self) -> Result<()> {
        match self {
            Decoder::Vt { dec, .. } => dec.start_sequence(),
            Decoder::Ff(d) => {
                d.flush();
                Ok(())
            }
            Decoder::Dav1d(d) => {
                d.flush();
                Ok(())
            }
            Decoder::Alpha(d) => {
                d.flush();
                Ok(())
            }
        }
    }

    /// Samples whose decode failed (VideoToolbox reports per frame).
    pub fn failures(&mut self) -> Vec<(i64, i32)> {
        match self {
            Decoder::Vt { dec, .. } => std::mem::take(&mut dec.failures),
            _ => vec![],
        }
    }

    /// Frames copied into GPU-shareable memory so far.
    pub fn copies(&self) -> usize {
        match self {
            Decoder::Ff(d) => d.copies,
            Decoder::Alpha(d) => d.copies(),
            _ => 0,
        }
    }
}

/// Counts VideoToolbox's output buffers in the frame pool's report.
fn track(pool: &FramePool, frames: &mut [DecodedFrame]) {
    for f in frames {
        if let crate::frame::Pixels::CoreVideo(b) = &mut f.pixels {
            b.track(pool);
        }
    }
}

/// Backend preferences for a stream, most preferred first.
pub fn backends(info: &CodecInfo, idx: &FrameIndex) -> Vec<Backend> {
    let prores = idx
        .config
        .fourcc
        .is_some_and(|f| crate::vt::is_prores_fourcc(&f))
        || info.codec == "prores";
    match info.codec.as_str() {
        _ if prores => vec![Backend::VideoToolbox],
        "h264" if (info.log2_chroma_w, info.log2_chroma_h) != (1, 1) => {
            vec![Backend::Ffmpeg, Backend::VideoToolbox]
        }
        "h264" | "hevc" => vec![Backend::VideoToolbox, Backend::Ffmpeg],
        "av1" => {
            if unsafe {
                crate::apple::cf::VTIsHardwareDecodeSupported(crate::apple::cf::fcc(b"av01"))
            } != 0
            {
                vec![Backend::VideoToolbox, Backend::Dav1d]
            } else {
                vec![Backend::Dav1d]
            }
        }
        "vp8" | "vp9" if info.alpha_mode => vec![Backend::FfmpegAlphaPair],
        _ => vec![Backend::Ffmpeg],
    }
}

/// Everything needed to open decoders for one stream.
pub struct StreamSpec<'a> {
    pub input: &'a Input,
    pub stream: usize,
    pub info: &'a CodecInfo,
    pub index: &'a FrameIndex,
    pub extradata: &'a [u8],
}

/// Opens the first backend that works for `spec`; `prefer` overrides the
/// order (it must be one of [`backends`]). Returns the decoder and the
/// reasons earlier backends were skipped.
pub fn open(
    spec: &StreamSpec<'_>,
    purpose: DecodePurpose,
    pool: &FramePool,
    arbiter: &dyn HardwareArbiter,
    threads: i32,
    prefer: Option<Backend>,
) -> Result<(Decoder, Vec<String>)> {
    let mut order = backends(spec.info, spec.index);
    if let Some(p) = prefer {
        if !order.contains(&p) {
            return Err(MediaError::unsupported(format!(
                "{p:?} does not decode {}",
                spec.info.codec
            )));
        }
        order.retain(|b| *b != p);
        order.insert(0, p);
    }
    let mut skipped = Vec::new();
    for b in order {
        let r = match b {
            Backend::VideoToolbox => open_vt(spec, purpose, arbiter, pool),
            Backend::Ffmpeg => FfDecoder::new(spec.input, spec.stream, None, pool.clone(), threads)
                .map(Decoder::Ff),
            Backend::Dav1d => Dav1dDecoder::new(pool.clone(), threads).map(Decoder::Dav1d),
            Backend::FfmpegAlphaPair => {
                AlphaPairDecoder::new(spec.input, spec.stream, pool.clone(), threads)
                    .map(Decoder::Alpha)
            }
        };
        match r {
            Ok(d) => return Ok((d, skipped)),
            Err(e) => skipped.push(format!("{b:?}: {e}")),
        }
    }
    Err(MediaError::unsupported(format!(
        "no decoder for {}: {}",
        spec.info.codec,
        skipped.join("; ")
    )))
}

fn open_vt(
    spec: &StreamSpec<'_>,
    purpose: DecodePurpose,
    arbiter: &dyn HardwareArbiter,
    pool: &FramePool,
) -> Result<Decoder> {
    let cfg = VtConfig::new(spec.info, spec.index, spec.extradata, purpose)?;
    let codec = HwCodec::from_fourcc(cfg.codec_fcc);
    let permit = if cfg.hardware_supported {
        arbiter.try_acquire(codec)
    } else {
        None
    };
    // Without a hardware session: ProRes still decodes in VideoToolbox's
    // software decoder (decision 12 keeps ProRes in VideoToolbox); other
    // codecs fall back to the next backend.
    if cfg.hardware_supported && permit.is_none() && !cfg.prores {
        return Err(MediaError::unsupported(format!(
            "no hardware {codec:?} session available"
        )));
    }
    let dec = VtDecoder::new(&cfg, spec.index.timebase, permit.is_some())?;
    Ok(Decoder::Vt {
        dec,
        cfg,
        _permit: permit,
        pool: pool.clone(),
    })
}
