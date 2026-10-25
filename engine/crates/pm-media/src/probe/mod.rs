//! Probe (section 13 "Probe and index"): streams, timecode, reel, UMID or
//! content identifier, pixel aspect, clean aperture; color; the frame index;
//! timing (edit lists, composition offsets, start offsets, priming); the
//! nominal rate; decode cost, poster, and filmstrip seed. Waveform peaks are
//! in [`peaks`]; the content hash in [`crate::hash`].

pub mod cache;
pub mod color;
pub mod peaks;
pub mod timing;

use crate::decode::{Backend, backends};
use crate::error::{MediaError, Result};
use crate::ff::{CodecInfo, Input};
use crate::index::{self, FrameIndex, IndexSource};
use crate::sys::ffmpeg as ff;
use crate::time::{NominalRate, nominal_rate};
use color::{ColorInfo, SideData};
use pm_protocol::color::AlphaMode;
use pm_time::{Rate, TimecodeFormat};
use serde::Serialize;
use std::path::{Path, PathBuf};
use timing::Priming;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum StreamKind {
    Video,
    Audio,
    Subtitle,
    Data,
    Attachment,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldOrder {
    Progressive,
    TopFirst,
    BottomFirst,
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldInfo {
    pub order: FieldOrder,
    /// "fiel" (MOV), "container or bitstream" (FFmpeg's stream parameters:
    /// MXF descriptors, Matroska FieldOrder, codec headers), or "decoded frame".
    pub source: String,
}

/// A clean aperture (`clap`): an exact rectangle in pixels.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanAperture {
    pub width: f64,
    pub height: f64,
    /// Center offset from the frame center, in pixels.
    pub horizontal_offset: f64,
    pub vertical_offset: f64,
    /// The rectangle rounded to whole pixels (x, y, width, height).
    pub rect: (u32, u32, u32, u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AlphaSource {
    /// The pixel format carries alpha (ProRes 4444, PNG, QuickTime Animation, ...).
    PixelFormat,
    /// HEVC with an alpha layer (`v0a8` through VideoToolbox).
    HevcAlphaLayer,
    /// VP8 or VP9 with alpha in Matroska BlockAdditions.
    MatroskaBlockAdditional,
}

/// How expensive a stream is to decode and seek, from its codec and GOP
/// structure (no decoding).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DecodeCost {
    pub backend: Backend,
    /// The preferred backend decodes in hardware on this Mac.
    pub hardware: bool,
    pub intra_only: bool,
    /// Decode-order samples between sync samples: the longest and the mean.
    pub max_gop: usize,
    pub mean_gop: f64,
    /// The longest decode plan for any presented frame (samples fed to show it).
    pub worst_seek_samples: usize,
    pub pixels_per_frame: u64,
    pub class: CostClass,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CostClass {
    /// Intra-only, or hardware long-GOP up to HD.
    Light,
    /// Hardware long-GOP above HD, or software up to HD.
    Medium,
    /// Software long-GOP above HD, or seeks longer than 120 samples.
    Heavy,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexSummary {
    pub source: IndexSource,
    pub build_ms: f64,
    pub samples: usize,
    pub presented: usize,
    pub sync_samples: usize,
    pub hidden_samples: usize,
    pub edit_list: Option<String>,
    pub open_gop: Option<String>,
    pub fallback_reason: Option<String>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoProbe {
    pub codec: CodecInfo,
    pub width: u32,
    pub height: u32,
    /// Sample (pixel) aspect ratio; 1/1 when the file does not say.
    pub pixel_aspect: (i64, i64),
    pub pixel_aspect_source: Option<String>,
    pub clean_aperture: Option<CleanAperture>,
    pub color: ColorInfo,
    pub alpha: Option<AlphaSource>,
    pub fields: FieldInfo,
    /// Display matrix rotation in degrees, when present.
    pub rotation: Option<f64>,
    pub nominal_rate: Option<NominalRate>,
    /// Frame durations differ from the nominal frame duration by more than
    /// one timebase unit somewhere (variable frame rate).
    pub variable_rate: bool,
    pub min_frame_duration: i64,
    pub max_frame_duration: i64,
    pub frames: usize,
    pub index: IndexSummary,
    pub decode: DecodeCost,
    /// Presented frame for the poster: the first sync frame at or after 10%.
    pub poster_frame: usize,
    /// Presented sync frames spread over the clip, for filmstrips.
    pub filmstrip_seed: Vec<usize>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioProbe {
    pub codec: String,
    pub sample_rate: i64,
    pub channels: i32,
    pub channel_layout: Option<String>,
    pub sample_format: Option<String>,
    pub priming: Option<Priming>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamProbe {
    pub index: usize,
    pub id: u64,
    pub kind: StreamKind,
    pub codec: String,
    /// Seconds per timestamp unit as [num, den].
    pub timebase: [i64; 2],
    /// First timestamp (stream start offset), in the stream timebase.
    pub start_pts: Option<i64>,
    pub duration_pts: Option<i64>,
    pub default: bool,
    pub attached_picture: bool,
    pub metadata: Vec<(String, String)>,
    pub audio: Option<AudioProbe>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimecodeInfo {
    pub text: String,
    pub drop_frame: bool,
    /// Frame number of the timecode at the nominal rate, when it parses.
    pub frame: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaProbe {
    pub path: PathBuf,
    pub size: u64,
    pub container: String,
    /// Container duration and start in microseconds (FFmpeg's AV_TIME_BASE).
    pub duration_us: Option<i64>,
    pub start_us: Option<i64>,
    pub streams: Vec<StreamProbe>,
    /// FFmpeg stream index of the best video and audio streams.
    pub video_stream: Option<usize>,
    pub audio_stream: Option<usize>,
    pub video: Option<VideoProbe>,
    pub timecode: Option<TimecodeInfo>,
    pub reel: Option<String>,
    pub umid: Option<String>,
    pub content_identifier: Option<String>,
    pub creation_time: Option<String>,
    pub sampled_fingerprint: String,
}

/// Everything probe learns, plus the video index (kept for decoding).
pub struct Probed {
    pub probe: MediaProbe,
    pub index: Option<FrameIndex>,
}

/// Container and stream timing without building a video index or probing
/// decoded video geometry/color. Used when only a playback clock is needed.
pub struct TimingProbe {
    pub duration_us: Option<i64>,
    pub start_us: Option<i64>,
    pub streams: Vec<StreamProbe>,
    pub video_stream: Option<usize>,
    pub audio_stream: Option<usize>,
}

pub fn probe_timing(path: &Path) -> Result<TimingProbe> {
    let input = Input::open(path)?;
    Ok(input_timing(path, &input, &input.format_name()))
}

fn input_timing(path: &Path, input: &Input, container: &str) -> TimingProbe {
    let mut streams = Vec::new();
    for i in 0..input.streams() {
        let st = input.st(i);
        let (tb, start, dur, disposition) = unsafe {
            (
                (*st).time_base,
                (*st).start_time,
                (*st).duration,
                (*st).disposition,
            )
        };
        let k = kind(input.media_type(i));
        let audio = (k == StreamKind::Audio).then(|| audio_probe(path, input, i, container));
        streams.push(StreamProbe {
            index: i,
            id: input.stream_id(i),
            kind: k,
            codec: input.codec_name(i),
            timebase: [tb.num as i64, tb.den as i64],
            start_pts: (start != ff::AV_NOPTS_VALUE).then_some(start),
            duration_pts: (dur != ff::AV_NOPTS_VALUE && dur > 0).then_some(dur),
            default: disposition & ff::AV_DISPOSITION_DEFAULT as i32 != 0,
            attached_picture: disposition & ff::AV_DISPOSITION_ATTACHED_PIC as i32 != 0,
            metadata: input.all_metadata(Some(i)),
            audio,
        });
    }
    let video_stream = input.best_video().filter(|&v| !streams[v].attached_picture);
    let audio_stream = input.best_audio();
    let (duration_us, start_us) = unsafe {
        let d = (*input.fmt).duration;
        let s = (*input.fmt).start_time;
        (
            (d != ff::AV_NOPTS_VALUE).then_some(d),
            (s != ff::AV_NOPTS_VALUE).then_some(s),
        )
    };
    TimingProbe {
        duration_us,
        start_us,
        streams,
        video_stream,
        audio_stream,
    }
}

fn kind(t: ff::AVMediaType) -> StreamKind {
    match t {
        ff::AVMEDIA_TYPE_VIDEO => StreamKind::Video,
        ff::AVMEDIA_TYPE_AUDIO => StreamKind::Audio,
        ff::AVMEDIA_TYPE_SUBTITLE => StreamKind::Subtitle,
        ff::AVMEDIA_TYPE_DATA => StreamKind::Data,
        ff::AVMEDIA_TYPE_ATTACHMENT => StreamKind::Attachment,
        _ => StreamKind::Other,
    }
}

/// Probes a file.
pub fn probe(path: &Path) -> Result<Probed> {
    probe_with_input(path).map(|(probed, _)| probed)
}

/// Probe while retaining its opened demuxer for the first decoder session.
/// This avoids reopening the same source after indexing on first use.
pub(crate) fn probe_with_input(path: &Path) -> Result<(Probed, Input)> {
    let size = std::fs::metadata(path)?.len();
    // The sampled identity is independent of FFmpeg's stream probe and the
    // container index: for larger files its reads overlap them. Tiny clips
    // stay inline, where a thread would cost more than it saves.
    if size >= 128 << 10 {
        return std::thread::scope(|scope| {
            let fingerprint = scope.spawn(|| crate::hash::sampled_fingerprint(path));
            let probed = probe_with_input_fingerprint(path, size, || Ok(String::new()));
            let value = fingerprint
                .join()
                .map_err(|_| MediaError::invalid("fingerprint worker panicked"))?;
            let (mut probed, input) = probed?;
            probed.probe.sampled_fingerprint = value?;
            Ok((probed, input))
        });
    }
    probe_with_input_fingerprint(path, size, || crate::hash::sampled_fingerprint(path))
}

fn probe_with_input_fingerprint(
    path: &Path,
    size: u64,
    fingerprint: impl FnOnce() -> Result<String>,
) -> Result<(Probed, Input)> {
    let input = Input::open(path)?;
    let container = input.format_name();
    let TimingProbe {
        duration_us,
        start_us,
        streams,
        video_stream,
        audio_stream,
    } = input_timing(path, &input, &container);
    let (video, index) = match video_stream {
        Some(v) => {
            let (p, idx) = video_probe(path, &input, v)?;
            (Some(p), Some(idx))
        }
        None => (None, None),
    };
    let fmt_meta = input.all_metadata(None);
    let find = |pred: &dyn Fn(&str) -> bool| -> Option<String> {
        fmt_meta
            .iter()
            .chain(streams.iter().flat_map(|s| s.metadata.iter()))
            .find(|(k, _)| pred(&k.to_ascii_lowercase()))
            .map(|(_, v)| v.clone())
    };
    let timecode = video_stream
        .and_then(|v| input.stream_metadata(v, "timecode"))
        .or_else(|| find(&|k| k == "timecode"))
        .map(|text| {
            let drop_frame = text.contains(';') || text.contains(',');
            let frame = video
                .as_ref()
                .and_then(|v| v.nominal_rate.as_ref())
                .and_then(|n| {
                    let f = if drop_frame {
                        TimecodeFormat::drop_frame(n.rate).ok()?
                    } else {
                        TimecodeFormat::non_drop(n.rate)
                    };
                    f.parse(&text.replace(',', ";")).ok()
                });
            TimecodeInfo {
                text,
                drop_frame,
                frame,
            }
        });
    let probe = MediaProbe {
        path: path.to_path_buf(),
        size,
        container,
        duration_us,
        start_us,
        video_stream,
        audio_stream,
        timecode,
        reel: find(&|k| k.contains("reel") || k.contains("tape_name")),
        umid: find(&|k| k.contains("umid")),
        content_identifier: find(&|k| {
            k.contains("content.identifier") || k == "content_identifier"
        }),
        creation_time: find(&|k| k == "creation_time"),
        sampled_fingerprint: fingerprint()?,
        streams,
        video,
    };
    Ok((Probed { probe, index }, input))
}

fn audio_probe(path: &Path, input: &Input, i: usize, container: &str) -> AudioProbe {
    unsafe {
        let par = input.par(i);
        let rate = (*par).sample_rate as i64;
        let mut layout_buf = [0 as std::ffi::c_char; 128];
        let layout = (ff::av_channel_layout_describe(
            &(*par).ch_layout,
            layout_buf.as_mut_ptr(),
            layout_buf.len(),
        ) > 0)
            .then(|| {
                std::ffi::CStr::from_ptr(layout_buf.as_ptr())
                    .to_string_lossy()
                    .into_owned()
            });
        let codec = input.codec_name(i);
        AudioProbe {
            priming: timing::priming(
                path,
                container,
                input.stream_id(i),
                &codec,
                rate,
                (*par).initial_padding as i64,
            ),
            codec,
            sample_rate: rate,
            channels: (*par).ch_layout.nb_channels,
            channel_layout: layout,
            sample_format: crate::ff::cstr(ff::av_get_sample_fmt_name((*par).format)),
        }
    }
}

fn video_probe(path: &Path, input: &Input, v: usize) -> Result<(VideoProbe, FrameIndex)> {
    let info = input.codec_info(v);
    let idx = index::build(path, input, v)?;
    let side = SideData::read(input, v);
    let atom = |n: &[u8; 4]| idx.config.atom(n);
    let hvcc_layers = atom(b"hvcC").map(crate::vt::hevc_max_layers).unwrap_or(1);
    let alpha = if hvcc_layers > 1 {
        Some(AlphaSource::HevcAlphaLayer)
    } else if info.alpha_mode {
        Some(AlphaSource::MatroskaBlockAdditional)
    } else if info.pix_fmt_alpha {
        Some(AlphaSource::PixelFormat)
    } else {
        None
    };
    let alpha_mode = alpha.map(|_| hevc_alpha_mode(&info, &idx).unwrap_or(AlphaMode::Straight));
    let color = color::interpret(&info, atom(b"colr"), &side, alpha_mode);

    // Pixel aspect: `pasp`, else FFmpeg's sample aspect ratio.
    let (pixel_aspect, pixel_aspect_source) = match atom(b"pasp").filter(|p| p.len() >= 8) {
        Some(p) => {
            let h = u32::from_be_bytes(p[..4].try_into().unwrap()) as i64;
            let w = u32::from_be_bytes(p[4..8].try_into().unwrap()) as i64;
            if h > 0 && w > 0 {
                ((h, w), Some("pasp".into()))
            } else {
                ((1, 1), None)
            }
        }
        None if info.sample_aspect.0 > 0 && info.sample_aspect.1 > 0 => (
            (info.sample_aspect.0 as i64, info.sample_aspect.1 as i64),
            Some("container or bitstream (FFmpeg)".into()),
        ),
        None => ((1, 1), None),
    };
    let (w, h) = (info.width.max(0) as u32, info.height.max(0) as u32);
    let clean_aperture = atom(b"clap").and_then(|c| clean_aperture(c, w, h));
    let mut fields = field_info(&info, atom(b"fiel"));
    if fields.order == FieldOrder::Unknown
        && matches!(
            info.codec.as_str(),
            "dvvideo" | "mpeg2video" | "h264" | "dnxhd" | "mjpeg"
        )
        && let Some(f) = first_frame_fields(input, v, &idx, path)
    {
        fields = f;
    }

    let pts = idx.presented_pts();
    let nominal = nominal_rate(&pts, idx.timebase);
    let durations: Vec<i64> = pts.windows(2).map(|w| w[1] - w[0]).collect();
    let (min_d, max_d) = (
        durations.iter().copied().min().unwrap_or(0),
        durations.iter().copied().max().unwrap_or(0),
    );
    let variable_rate = match &nominal {
        Some(n) => {
            let (tn, td) = (idx.timebase.num() as i128, idx.timebase.den() as i128);
            // nominal duration in units = rate.den·tb.den / (rate.num·tb.num)
            let (a, b) = (n.rate.den() as i128 * td, n.rate.num() as i128 * tn);
            durations.iter().any(|&d| (d as i128 * b - a).abs() > b)
        }
        None => false,
    };
    let decode = decode_cost(&info, &idx, input.codec_is_intra_only(v));
    let presented = idx.frames.len();
    let sync_frames: Vec<usize> = (0..presented)
        .filter(|&n| idx.samples[idx.frames[n].sample].key)
        .collect();
    let poster_frame = sync_frames
        .iter()
        .copied()
        .find(|&n| n >= presented / 10)
        .unwrap_or(0);
    let filmstrip_seed = if sync_frames.len() <= 32 {
        sync_frames.clone()
    } else {
        (0..32)
            .map(|i| sync_frames[i * (sync_frames.len() - 1) / 31])
            .collect()
    };
    let summary = IndexSummary {
        source: idx.source,
        build_ms: idx.stats.build_ms,
        samples: idx.samples.len(),
        presented,
        sync_samples: idx.stats.sync_samples,
        hidden_samples: idx.hidden_samples(),
        edit_list: idx.stats.edit_list.clone(),
        open_gop: idx.stats.open_gop.clone(),
        fallback_reason: idx.stats.fallback_reason.clone(),
        detail: idx.stats.detail.clone(),
    };
    Ok((
        VideoProbe {
            width: w,
            height: h,
            pixel_aspect,
            pixel_aspect_source,
            clean_aperture,
            color,
            alpha,
            fields,
            rotation: side.rotation,
            nominal_rate: nominal,
            variable_rate,
            min_frame_duration: min_d,
            max_frame_duration: max_d,
            frames: presented,
            index: summary,
            decode,
            poster_frame,
            filmstrip_seed,
            codec: info,
        },
        idx,
    ))
}

/// HEVC with alpha: VideoToolbox's `AlphaChannelMode` format extension
/// (derived from the bitstream's alpha channel information SEI).
fn hevc_alpha_mode(info: &CodecInfo, idx: &FrameIndex) -> Option<AlphaMode> {
    use crate::apple::cf::*;
    if info.codec != "hevc" {
        return None;
    }
    let cfg = crate::vt::VtConfig::new(info, idx, &[], crate::vt::DecodePurpose::Display).ok()?;
    let fd = cfg.format.as_ref()?;
    let v = unsafe {
        cf_string_value(CMFormatDescriptionGetExtension(
            fd.as_mut_ptr(),
            cf_str("AlphaChannelMode").as_ptr(),
        ))
    }?;
    Some(if v.contains("Premultiplied") {
        AlphaMode::Premultiplied
    } else {
        AlphaMode::Straight
    })
}

/// `clap`: cleanApertureWidthN/D, HeightN/D, horizOffN/D, vertOffN/D.
fn clean_aperture(c: &[u8], w: u32, h: u32) -> Option<CleanAperture> {
    if c.len() < 32 {
        return None;
    }
    let r = |i: usize| {
        let n = i32::from_be_bytes(c[i..i + 4].try_into().unwrap()) as f64;
        let d = i32::from_be_bytes(c[i + 4..i + 8].try_into().unwrap()) as f64;
        (d != 0.0).then(|| n / d)
    };
    let (cw, ch, ho, vo) = (r(0)?, r(8)?, r(16)?, r(24)?);
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let x = (w as f64 - cw) / 2.0 + ho;
    let y = (h as f64 - ch) / 2.0 + vo;
    let rect = (
        x.round().max(0.0) as u32,
        y.round().max(0.0) as u32,
        cw.round() as u32,
        ch.round() as u32,
    );
    Some(CleanAperture {
        width: cw,
        height: ch,
        horizontal_offset: ho,
        vertical_offset: vo,
        rect,
    })
}

fn field_info(info: &CodecInfo, fiel: Option<&[u8]>) -> FieldInfo {
    if let Some(f) = fiel.filter(|f| f.len() >= 2) {
        let order = match (f[0], f[1]) {
            (1, _) => FieldOrder::Progressive,
            (2, 1) | (2, 9) => FieldOrder::TopFirst,
            (2, 6) | (2, 14) => FieldOrder::BottomFirst,
            _ => FieldOrder::Unknown,
        };
        if order != FieldOrder::Unknown {
            return FieldInfo {
                order,
                source: "fiel".into(),
            };
        }
    }
    let order = match info.field_order {
        ff::AV_FIELD_PROGRESSIVE => FieldOrder::Progressive,
        // Coded and displayed order agree for TT and BB; TB and BT are
        // coded one way and displayed the other: display order wins.
        ff::AV_FIELD_TT | ff::AV_FIELD_BT => FieldOrder::TopFirst,
        ff::AV_FIELD_BB | ff::AV_FIELD_TB => FieldOrder::BottomFirst,
        _ => FieldOrder::Unknown,
    };
    FieldInfo {
        order,
        source: if order == FieldOrder::Unknown {
            "unknown".into()
        } else {
            "container or bitstream".into()
        },
    }
}

/// Field order from the first decoded frame's flags (DV, MPEG-2, H.264,
/// DNxHD, and MJPEG signal it per picture).
fn first_frame_fields(input: &Input, v: usize, idx: &FrameIndex, path: &Path) -> Option<FieldInfo> {
    let pool = crate::pool::FramePool::new(0);
    let mut dec = crate::ff::decoder::FfDecoder::new(input, v, None, pool, 1).ok()?;
    let mut reader = crate::ff::PacketReader::new(path, v).ok()?;
    let plan = idx.decode_plan_for_frame(0);
    let pkts = reader.read(idx, &plan).ok()?;
    let mut out = Vec::new();
    for p in &pkts {
        dec.send(Some(p), &mut out).ok()?;
    }
    dec.send(None, &mut out).ok()?;
    out.first()?;
    let f = dec.last_fields;
    Some(FieldInfo {
        order: if !f.interlaced {
            FieldOrder::Progressive
        } else if f.top_field_first {
            FieldOrder::TopFirst
        } else {
            FieldOrder::BottomFirst
        },
        source: "decoded frame".into(),
    })
}

fn decode_cost(info: &CodecInfo, idx: &FrameIndex, intra_only: bool) -> DecodeCost {
    let order = backends(info, idx);
    let backend = order[0];
    let hardware = backend == Backend::VideoToolbox && {
        let f = idx
            .config
            .fourcc
            .map(u32::from_be_bytes)
            .unwrap_or(match info.codec.as_str() {
                "h264" => crate::apple::cf::fcc(b"avc1"),
                "hevc" => crate::apple::cf::fcc(b"hvc1"),
                "av1" => crate::apple::cf::fcc(b"av01"),
                _ => 0,
            });
        f != 0 && unsafe { crate::apple::cf::VTIsHardwareDecodeSupported(f) } != 0
    };
    let syncs: Vec<usize> = (0..idx.samples.len())
        .filter(|&k| idx.samples[k].key)
        .collect();
    let mut gops: Vec<usize> = syncs.windows(2).map(|w| w[1] - w[0]).collect();
    if let Some(&last) = syncs.last() {
        gops.push(idx.samples.len() - last);
    }
    let max_gop = gops.iter().copied().max().unwrap_or(idx.samples.len());
    let mean_gop = if gops.is_empty() {
        idx.samples.len() as f64
    } else {
        gops.iter().sum::<usize>() as f64 / gops.len() as f64
    };
    let worst_seek_samples = (0..idx.frames.len())
        .map(|n| idx.decode_plan_for_frame(n).len())
        .max()
        .unwrap_or(0);
    let pixels = info.width.max(0) as u64 * info.height.max(0) as u64;
    let above_hd = pixels > 1920 * 1080;
    let class = if worst_seek_samples > 120 || (!hardware && !intra_only && above_hd) {
        CostClass::Heavy
    } else if intra_only || (hardware && !above_hd) {
        CostClass::Light
    } else {
        CostClass::Medium
    };
    DecodeCost {
        backend,
        hardware,
        intra_only,
        max_gop,
        mean_gop,
        worst_seek_samples,
        pixels_per_frame: pixels,
        class,
    }
}

/// The nominal rate of a probed video stream as an engine rate.
pub fn nominal(probe: &MediaProbe) -> Option<Rate> {
    probe.video.as_ref()?.nominal_rate.as_ref().map(|n| n.rate)
}

impl MediaProbe {
    pub fn to_json(&self) -> Result<String> {
        serde_json::to_string_pretty(self).map_err(|e| MediaError::invalid(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_fingerprint_keeps_probe_and_index_identical() {
        // Large enough to take the parallel route.
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../apps/desktop/e2e/fixtures/tone.wav");
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size >= 128 << 10);
        let (parallel, _) = probe_with_input(&path).unwrap();
        let (serial, _) =
            probe_with_input_fingerprint(&path, size, || crate::hash::sampled_fingerprint(&path))
                .unwrap();
        assert!(!parallel.probe.sampled_fingerprint.is_empty());
        assert_eq!(parallel.probe, serial.probe);
        assert_eq!(parallel.index, serial.index);
    }

    #[test]
    fn clean_aperture_rectangle() {
        // 1920×1080 frame, 1888×1062 aperture centered: (16, 9, 1888, 1062).
        let mut c = vec![];
        for (n, d) in [(1888i32, 1i32), (1062, 1), (0, 1), (0, 1)] {
            c.extend_from_slice(&n.to_be_bytes());
            c.extend_from_slice(&d.to_be_bytes());
        }
        let a = clean_aperture(&c, 1920, 1080).unwrap();
        assert_eq!(a.rect, (16, 9, 1888, 1062));
        // A horizontal offset of −8.5 pixels.
        c[16..20].copy_from_slice(&(-17i32).to_be_bytes());
        c[20..24].copy_from_slice(&2i32.to_be_bytes());
        assert_eq!(
            clean_aperture(&c, 1920, 1080).unwrap().horizontal_offset,
            -8.5
        );
    }

    #[test]
    fn fiel_atom_orders() {
        let info = |fo| CodecInfo {
            codec: "x".into(),
            codec_id: 0,
            codec_tag: None,
            profile: 0,
            profile_name: None,
            width: 0,
            height: 0,
            pix_fmt: String::new(),
            bit_depth: 8,
            log2_chroma_w: 1,
            log2_chroma_h: 1,
            pix_fmt_alpha: false,
            full_range: false,
            alpha_mode: false,
            color_primaries: 2,
            color_trc: 2,
            color_space: 2,
            color_range: 0,
            chroma_location: 0,
            field_order: fo,
            sample_aspect: (0, 1),
        };
        assert_eq!(
            field_info(&info(0), Some(&[2, 9])).order,
            FieldOrder::TopFirst
        );
        assert_eq!(
            field_info(&info(0), Some(&[2, 14])).order,
            FieldOrder::BottomFirst
        );
        assert_eq!(
            field_info(&info(0), Some(&[1, 0])).order,
            FieldOrder::Progressive
        );
        assert_eq!(
            field_info(&info(ff::AV_FIELD_BB), None).order,
            FieldOrder::BottomFirst
        );
        assert_eq!(field_info(&info(0), None).order, FieldOrder::Unknown);
    }
}
