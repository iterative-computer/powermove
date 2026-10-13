//! FFmpeg through Powermove's LGPL build (`PM_FFMPEG_DIR`): demuxing and
//! probing (`Input`), packet reads for containers without file offsets, and
//! software decoding into Metal-shareable pool memory (`decoder`).

pub mod alpha;
pub mod decoder;

use crate::error::{MediaError, Result};
use crate::sys::ffmpeg as ff;
use pm_time::Rational;
use std::ffi::{CStr, CString, c_int};
use std::path::Path;
use std::ptr;
use std::sync::Once;

pub fn err_str(e: c_int) -> String {
    let mut buf = [0 as std::ffi::c_char; 256];
    unsafe {
        ff::av_strerror(e, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

pub(crate) fn check(e: c_int, what: &str) -> Result<c_int> {
    if e < 0 { Err(MediaError::Ffmpeg { what: what.into(), code: e, message: err_str(e) }) } else { Ok(e) }
}

/// FFmpeg logs warnings for recoverable stream damage to stderr; the engine
/// reports errors through its own channels, so only errors are logged.
pub(crate) fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe { ff::av_log_set_level(ff::AV_LOG_ERROR as c_int) });
}

pub(crate) unsafe fn cstr(p: *const std::ffi::c_char) -> Option<String> {
    if p.is_null() { None } else { Some(unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()) }
}

/// An open input.
pub struct Input {
    pub(crate) fmt: *mut ff::AVFormatContext,
}

// The context is used from one thread at a time.
unsafe impl Send for Input {}

impl Input {
    /// Opens and probes a file (`avformat_find_stream_info`).
    pub fn open(path: &Path) -> Result<Input> {
        let inp = Self::open_raw(path)?;
        unsafe { check(ff::avformat_find_stream_info(inp.fmt, ptr::null_mut()), "stream info")? };
        Ok(inp)
    }

    /// Opens without probing stream info (packet reads only).
    pub fn open_raw(path: &Path) -> Result<Input> {
        init();
        let c = CString::new(path.to_str().ok_or_else(|| MediaError::invalid("path is not UTF-8"))?)
            .map_err(|_| MediaError::invalid("path contains NUL"))?;
        let mut fmt: *mut ff::AVFormatContext = ptr::null_mut();
        unsafe {
            check(ff::avformat_open_input(&mut fmt, c.as_ptr(), ptr::null(), ptr::null_mut()), "open")?;
        }
        Ok(Input { fmt })
    }

    pub fn format_name(&self) -> String {
        unsafe { cstr((*(*self.fmt).iformat).name).unwrap_or_default() }
    }

    pub fn streams(&self) -> usize {
        unsafe { (*self.fmt).nb_streams as usize }
    }

    pub(crate) fn st(&self, i: usize) -> *mut ff::AVStream {
        assert!(i < self.streams(), "stream {i} out of range");
        unsafe { *(*self.fmt).streams.add(i) }
    }

    pub(crate) fn par(&self, i: usize) -> *mut ff::AVCodecParameters {
        unsafe { (*self.st(i)).codecpar }
    }

    pub fn media_type(&self, i: usize) -> ff::AVMediaType {
        unsafe { (*self.par(i)).codec_type }
    }

    pub fn codec_id(&self, i: usize) -> ff::AVCodecID {
        unsafe { (*self.par(i)).codec_id }
    }

    pub fn codec_name(&self, i: usize) -> String {
        unsafe { cstr(ff::avcodec_get_name(self.codec_id(i))).unwrap_or_default() }
    }

    /// The container's id for the stream (MP4 track_ID, Matroska
    /// TrackNumber, MXF track id), as FFmpeg reports it.
    pub fn stream_id(&self, i: usize) -> u64 {
        unsafe { (*self.st(i)).id as u32 as u64 }
    }

    pub fn timebase(&self, i: usize) -> Result<Rational> {
        let tb = unsafe { (*self.st(i)).time_base };
        Ok(Rational::new(tb.num as i64, tb.den as i64)?)
    }

    pub fn codec_is_intra_only(&self, i: usize) -> bool {
        unsafe {
            let d = ff::avcodec_descriptor_get(self.codec_id(i));
            !d.is_null() && (*d).props & ff::AV_CODEC_PROP_INTRA_ONLY as c_int != 0
        }
    }

    /// The best video stream (FFmpeg's choice, skipping attached pictures).
    pub fn best_video(&self) -> Option<usize> {
        let s = unsafe { ff::av_find_best_stream(self.fmt, ff::AVMEDIA_TYPE_VIDEO, -1, -1, ptr::null_mut(), 0) };
        (s >= 0).then_some(s as usize)
    }

    pub fn best_audio(&self) -> Option<usize> {
        let s = unsafe { ff::av_find_best_stream(self.fmt, ff::AVMEDIA_TYPE_AUDIO, -1, -1, ptr::null_mut(), 0) };
        (s >= 0).then_some(s as usize)
    }

    /// How many video streams precede `i`, when `i` is a video stream.
    pub fn video_ordinal(&self, i: usize) -> Option<usize> {
        (self.media_type(i) == ff::AVMEDIA_TYPE_VIDEO)
            .then(|| (0..i).filter(|&k| self.media_type(k) == ff::AVMEDIA_TYPE_VIDEO).count())
    }

    /// Stream metadata value.
    pub fn stream_metadata(&self, i: usize, key: &str) -> Option<String> {
        let k = CString::new(key).ok()?;
        unsafe {
            let e = ff::av_dict_get((*self.st(i)).metadata, k.as_ptr(), ptr::null(), 0);
            if e.is_null() { None } else { cstr((*e).value) }
        }
    }

    /// Container metadata value.
    pub fn format_metadata(&self, key: &str) -> Option<String> {
        let k = CString::new(key).ok()?;
        unsafe {
            let e = ff::av_dict_get((*self.fmt).metadata, k.as_ptr(), ptr::null(), 0);
            if e.is_null() { None } else { cstr((*e).value) }
        }
    }

    /// Every metadata entry of the container and of stream `i`.
    pub fn all_metadata(&self, stream: Option<usize>) -> Vec<(String, String)> {
        let mut out = Vec::new();
        unsafe {
            let dict = match stream {
                Some(i) => (*self.st(i)).metadata,
                None => (*self.fmt).metadata,
            };
            let mut e: *const ff::AVDictionaryEntry = ptr::null();
            loop {
                e = ff::av_dict_iterate(dict, e);
                if e.is_null() {
                    break;
                }
                out.push((cstr((*e).key).unwrap_or_default(), cstr((*e).value).unwrap_or_default()));
            }
        }
        out
    }

    pub fn extradata(&self, i: usize) -> Vec<u8> {
        unsafe {
            let par = self.par(i);
            if (*par).extradata.is_null() || (*par).extradata_size <= 0 {
                vec![]
            } else {
                std::slice::from_raw_parts((*par).extradata, (*par).extradata_size as usize).to_vec()
            }
        }
    }

    /// Byte position and size of the stream's first packet.
    pub fn first_packet(&self, i: usize) -> Result<(i64, usize)> {
        self.with_first_packet(i, |p| unsafe { ((*p).pos, (*p).size as usize) })
    }

    /// Duration of the stream's first packet, in the stream timebase.
    pub fn packet_duration(&self, i: usize) -> Option<i64> {
        self.with_first_packet(i, |p| unsafe { (*p).duration }).ok().filter(|&d| d > 0)
    }

    fn with_first_packet<R>(&self, i: usize, f: impl FnOnce(*const ff::AVPacket) -> R) -> Result<R> {
        unsafe {
            check(ff::av_seek_frame(self.fmt, -1, 0, ff::AVSEEK_FLAG_BYTE as c_int), "seek to start")?;
            let mut pkt = ff::av_packet_alloc();
            let result = loop {
                let r = ff::av_read_frame(self.fmt, pkt);
                if r < 0 {
                    break Err(MediaError::invalid(format!("no packet for stream {i}: {}", err_str(r))));
                }
                if (*pkt).stream_index as usize == i {
                    break Ok(f(pkt));
                }
                ff::av_packet_unref(pkt);
            };
            ff::av_packet_free(&mut pkt);
            result
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.fmt) }
    }
}

/// Codec facts from FFmpeg's probe of one stream.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodecInfo {
    pub codec: String,
    #[serde(skip)]
    pub codec_id: ff::AVCodecID,
    /// Codec tag (MOV/AVI four-character code) when the container has one.
    pub codec_tag: Option<String>,
    pub profile: i32,
    pub profile_name: Option<String>,
    pub width: i32,
    pub height: i32,
    pub pix_fmt: String,
    pub bit_depth: u8,
    pub log2_chroma_w: u8,
    pub log2_chroma_h: u8,
    /// The pixel format has an alpha component.
    pub pix_fmt_alpha: bool,
    pub full_range: bool,
    /// Matroska `alpha_mode` (VP8/VP9 alpha in BlockAdditions).
    pub alpha_mode: bool,
    #[serde(skip)]
    pub color_primaries: ff::AVColorPrimaries,
    #[serde(skip)]
    pub color_trc: ff::AVColorTransferCharacteristic,
    #[serde(skip)]
    pub color_space: ff::AVColorSpace,
    #[serde(skip)]
    pub color_range: ff::AVColorRange,
    #[serde(skip)]
    pub chroma_location: ff::AVChromaLocation,
    #[serde(skip)]
    pub field_order: ff::AVFieldOrder,
    /// Sample aspect ratio (0/1 when unknown).
    pub sample_aspect: (i32, i32),
}

impl Input {
    pub fn codec_info(&self, i: usize) -> CodecInfo {
        unsafe {
            let par = self.par(i);
            let fmt = (*par).format;
            let desc = if (*par).codec_type == ff::AVMEDIA_TYPE_VIDEO { ff::av_pix_fmt_desc_get(fmt) } else { ptr::null() };
            let (depth, cw, ch, name, alpha) = if desc.is_null() {
                (8, 1, 1, "none".to_string(), false)
            } else {
                (
                    (*desc).comp[0].depth as u8,
                    (*desc).log2_chroma_w,
                    (*desc).log2_chroma_h,
                    cstr((*desc).name).unwrap_or_default(),
                    (*desc).flags as u32 & ff::AV_PIX_FMT_FLAG_ALPHA != 0,
                )
            };
            let range = (*par).color_range;
            let tag = (*par).codec_tag;
            let profile_name = cstr(ff::avcodec_profile_name((*par).codec_id, (*par).profile));
            CodecInfo {
                codec: self.codec_name(i),
                codec_id: (*par).codec_id,
                codec_tag: (tag != 0).then(|| {
                    let b = tag.to_le_bytes();
                    if b.iter().all(|c| c.is_ascii_graphic() || *c == b' ') {
                        String::from_utf8_lossy(&b).into_owned()
                    } else {
                        format!("0x{tag:08x}")
                    }
                }),
                profile: (*par).profile,
                profile_name,
                width: (*par).width,
                height: (*par).height,
                // yuvj formats are full range whatever the tag (section 9).
                full_range: range == ff::AVCOL_RANGE_JPEG || name.starts_with("yuvj"),
                pix_fmt: name,
                bit_depth: depth,
                log2_chroma_w: cw,
                log2_chroma_h: ch,
                pix_fmt_alpha: alpha,
                alpha_mode: self.stream_metadata(i, "alpha_mode").as_deref() == Some("1"),
                color_primaries: (*par).color_primaries,
                color_trc: (*par).color_trc,
                color_space: (*par).color_space,
                color_range: range,
                chroma_location: (*par).chroma_location,
                field_order: (*par).field_order,
                sample_aspect: ((*par).sample_aspect_ratio.num, (*par).sample_aspect_ratio.den),
            }
        }
    }
}

/// A compressed sample ready for a decoder.
#[derive(Clone, Debug, Default)]
pub struct Packet {
    pub data: Vec<u8>,
    /// Matroska BlockAdditional id 1 (VP8/VP9 alpha).
    pub additional: Option<Vec<u8>>,
    /// Composition timestamp (media timebase).
    pub pts: i64,
    pub dts: Option<i64>,
    pub duration: i64,
    pub key: bool,
}

/// One demuxed packet's facts (demux scans and cross-checks).
#[derive(Clone, Debug)]
pub struct DemuxPacket {
    pub pts: Option<i64>,
    pub dts: Option<i64>,
    pub duration: i64,
    pub key: bool,
    pub discard: bool,
    pub pos: i64,
    pub size: usize,
    /// First VCL NAL type of an Annex B H.264 or HEVC packet.
    pub nal_type: Option<u8>,
}

/// Reads every packet of stream `i` (payloads are read by the demuxer but
/// nothing is decoded).
pub fn demux_scan(path: &Path, i: usize) -> Result<(Vec<DemuxPacket>, Rational, ff::AVCodecID)> {
    let inp = Input::open(path)?;
    let nal = crate::index::scan::nal_codec(inp.codec_id(i));
    let mut out = Vec::new();
    unsafe {
        let mut pkt = ff::av_packet_alloc();
        let result = loop {
            let r = ff::av_read_frame(inp.fmt, pkt);
            if r == ff::AVERROR_EOF {
                break Ok(());
            }
            if r < 0 {
                break Err(MediaError::Ffmpeg { what: "read".into(), code: r, message: err_str(r) });
            }
            if (*pkt).stream_index as usize == i {
                let data = if (*pkt).data.is_null() {
                    &[][..]
                } else {
                    std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize)
                };
                out.push(DemuxPacket {
                    pts: ((*pkt).pts != ff::AV_NOPTS_VALUE).then_some((*pkt).pts),
                    dts: ((*pkt).dts != ff::AV_NOPTS_VALUE).then_some((*pkt).dts),
                    duration: (*pkt).duration,
                    key: (*pkt).flags & ff::AV_PKT_FLAG_KEY as c_int != 0,
                    discard: (*pkt).flags & ff::AV_PKT_FLAG_DISCARD as c_int != 0,
                    pos: (*pkt).pos,
                    size: (*pkt).size as usize,
                    nal_type: nal.and_then(|c| {
                        (data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]))
                            .then(|| c.first_vcl(crate::vt::annexb_nals(data)))
                            .flatten()
                    }),
                });
            }
            ff::av_packet_unref(pkt);
        };
        ff::av_packet_free(&mut pkt);
        result?;
    }
    Ok((out, inp.timebase(i)?, inp.codec_id(i)))
}

/// Reads indexed samples: file reads where the container gives offsets,
/// otherwise a byte seek in FFmpeg's demuxer to the first needed packet.
pub struct PacketReader {
    file: std::fs::File,
    path: std::path::PathBuf,
    stream: usize,
    demux: Option<Input>,
    pub bytes_read: u64,
}

impl PacketReader {
    pub fn new(path: &Path, stream: usize) -> Result<Self> {
        Ok(PacketReader { file: std::fs::File::open(path)?, path: path.into(), stream, demux: None, bytes_read: 0 })
    }

    /// Reads the given samples (ascending decode-order indices).
    pub fn read(&mut self, idx: &crate::index::FrameIndex, which: &[usize]) -> Result<Vec<Packet>> {
        use crate::index::Loc;
        use std::os::unix::fs::FileExt;
        if !which.windows(2).all(|w| w[0] < w[1]) {
            return Err(MediaError::invalid("sample list not ascending"));
        }
        let Some(&first) = which.first() else { return Ok(vec![]) };
        let strip = idx.config.strip_prefix.as_deref().unwrap_or(&[]);
        match idx.samples[first].loc {
            Loc::File { .. } => which
                .iter()
                .map(|&i| {
                    let s = &idx.samples[i];
                    let Loc::File { offset, size, additional } = s.loc else {
                        return Err(MediaError::invalid("mixed sample locations"));
                    };
                    let mut data = vec![0u8; strip.len() + size as usize];
                    data[..strip.len()].copy_from_slice(strip);
                    self.file.read_exact_at(&mut data[strip.len()..], offset)?;
                    self.bytes_read += size as u64;
                    let additional = match additional {
                        Some((o, n)) => {
                            let mut b = vec![0u8; n as usize];
                            self.file.read_exact_at(&mut b, o)?;
                            Some(b)
                        }
                        None => None,
                    };
                    Ok(Packet { data, additional, pts: s.cts, dts: s.dts, duration: s.duration, key: s.key })
                })
                .collect(),
            Loc::Demux { .. } => self.read_demuxed(idx, which),
        }
    }

    fn read_demuxed(&mut self, idx: &crate::index::FrameIndex, which: &[usize]) -> Result<Vec<Packet>> {
        use crate::index::Loc;
        if self.demux.is_none() {
            self.demux = Some(Input::open(&self.path)?);
        }
        let inp = self.demux.as_ref().unwrap();
        let want: Vec<(i64, usize)> = which
            .iter()
            .map(|&i| match idx.samples[i].loc {
                Loc::Demux { pos } => Ok((pos, i)),
                _ => Err(MediaError::invalid("mixed sample locations")),
            })
            .collect::<Result<_>>()?;
        let mut out = Vec::with_capacity(want.len());
        unsafe {
            check(ff::av_seek_frame(inp.fmt, self.stream as c_int, want[0].0, ff::AVSEEK_FLAG_BYTE as c_int), "byte seek")?;
            let mut pkt = ff::av_packet_alloc();
            let mut k = 0;
            let mut err = None;
            while k < want.len() {
                let r = ff::av_read_frame(inp.fmt, pkt);
                if r == ff::AVERROR_EOF {
                    break;
                }
                if r < 0 {
                    err = Some(MediaError::Ffmpeg { what: "read".into(), code: r, message: err_str(r) });
                    break;
                }
                if (*pkt).stream_index as usize == self.stream {
                    let p = (*pkt).pos;
                    if p == want[k].0 {
                        let s = &idx.samples[want[k].1];
                        let data = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize).to_vec();
                        self.bytes_read += data.len() as u64;
                        out.push(Packet { data, additional: None, pts: s.cts, dts: s.dts, duration: s.duration, key: s.key });
                        k += 1;
                    } else if p > want[k].0 {
                        err = Some(MediaError::invalid(format!(
                            "the demuxer passed byte {} without returning the indexed packet",
                            want[k].0
                        )));
                        break;
                    }
                }
                ff::av_packet_unref(pkt);
            }
            ff::av_packet_free(&mut pkt);
            if let Some(e) = err {
                return Err(e);
            }
            if k != want.len() {
                return Err(MediaError::invalid(format!("demux seek delivered {k} of {} packets", want.len())));
            }
        }
        Ok(out)
    }
}
