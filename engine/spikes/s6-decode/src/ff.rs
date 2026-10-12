//! FFmpeg (Homebrew 8.1 dylibs through ffmpeg-sys-the-third): probe, the TS
//! demux scan, demuxed packet reads, and software decode straight into
//! Metal-shared memory through a custom `get_buffer2`.

use crate::index::{FrameIndex, IndexSource, IndexStats, Loc, Sample};
use crate::pool::{FrameLayout, MetalPool, PoolFrame, PoolPlane, SlotRef};
use crate::time::Rational;
use anyhow::{Context, Result, anyhow, bail, ensure};
use ffmpeg_sys_the_third as ff;
use std::ffi::{CStr, CString, c_int, c_void};
use std::path::Path;
use std::ptr;
use std::sync::Arc;

pub fn err_str(e: c_int) -> String {
    let mut buf = [0 as std::ffi::c_char; 256];
    unsafe {
        ff::av_strerror(e, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

fn check(e: c_int, what: &str) -> Result<c_int> {
    if e < 0 { Err(anyhow!("{what}: {}", err_str(e))) } else { Ok(e) }
}

/// An open input with its best video stream.
pub struct Input {
    pub fmt: *mut ff::AVFormatContext,
    pub stream: c_int,
}

unsafe impl Send for Input {}

impl Input {
    pub fn open(path: &Path) -> Result<Input> {
        let c = CString::new(path.to_str().context("path")?)?;
        let mut fmt: *mut ff::AVFormatContext = ptr::null_mut();
        unsafe {
            check(ff::avformat_open_input(&mut fmt, c.as_ptr(), ptr::null(), ptr::null_mut()), "open")?;
            if let Err(e) = check(ff::avformat_find_stream_info(fmt, ptr::null_mut()), "stream info") {
                ff::avformat_close_input(&mut fmt);
                return Err(e);
            }
            let s = ff::av_find_best_stream(fmt, ff::AVMediaType::VIDEO, -1, -1, ptr::null_mut(), 0);
            if s < 0 {
                ff::avformat_close_input(&mut fmt);
                bail!("no video stream");
            }
            Ok(Input { fmt, stream: s })
        }
    }
    pub fn st(&self) -> *mut ff::AVStream {
        unsafe { *(*self.fmt).streams.add(self.stream as usize) }
    }
    pub fn par(&self) -> *mut ff::AVCodecParameters {
        unsafe { (*self.st()).codecpar }
    }
    pub fn format_name(&self) -> String {
        unsafe { CStr::from_ptr((*(*self.fmt).iformat).name).to_string_lossy().into_owned() }
    }
    pub fn timebase(&self) -> Rational {
        let tb = unsafe { (*self.st()).time_base };
        Rational::new(tb.num as i64, tb.den as i64)
    }
    pub fn metadata(&self, key: &str) -> Option<String> {
        let k = CString::new(key).ok()?;
        unsafe {
            let e = ff::av_dict_get((*self.st()).metadata, k.as_ptr(), ptr::null(), 0);
            if e.is_null() { None } else { Some(CStr::from_ptr((*e).value).to_string_lossy().into_owned()) }
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe { ff::avformat_close_input(&mut self.fmt) }
    }
}

/// Codec facts the spike needs, from FFmpeg's probe.
#[derive(Clone, Debug, serde::Serialize)]
pub struct StreamInfo {
    pub container: String,
    pub codec: String,
    pub codec_id: u32,
    pub profile: i32,
    pub width: i32,
    pub height: i32,
    pub pix_fmt: String,
    pub bit_depth: u8,
    pub log2_chroma_w: u8,
    pub log2_chroma_h: u8,
    pub full_range: bool,
    pub color_range: String,
    /// Matrix coefficients tag ("unknown" when untagged).
    pub color_space: String,
    pub timebase: Rational,
    /// Matroska `alpha_mode` tag (VP8/VP9 alpha in BlockAdditions).
    pub alpha_mode: bool,
}

pub fn stream_info(inp: &Input) -> StreamInfo {
    unsafe {
        let par = inp.par();
        let fmt = ff::AVPixelFormat((*par).format);
        let desc = ff::av_pix_fmt_desc_get(fmt);
        let (depth, cw, ch, name) = if desc.is_null() {
            (8, 1, 1, "none".to_string())
        } else {
            (
                (*desc).comp[0].depth as u8,
                (*desc).log2_chroma_w,
                (*desc).log2_chroma_h,
                CStr::from_ptr((*desc).name).to_string_lossy().into_owned(),
            )
        };
        let range = (*par).color_range;
        // yuvj formats are full range regardless of the tag (section 9).
        let full = range == ff::AVColorRange::JPEG || name.starts_with("yuvj");
        StreamInfo {
            container: inp.format_name(),
            codec: CStr::from_ptr(ff::avcodec_get_name((*par).codec_id)).to_string_lossy().into_owned(),
            codec_id: (*par).codec_id.0,
            profile: (*par).profile,
            width: (*par).width,
            height: (*par).height,
            pix_fmt: name,
            bit_depth: depth,
            log2_chroma_w: cw,
            log2_chroma_h: ch,
            full_range: full,
            color_range: CStr::from_ptr(ff::av_color_range_name(range)).to_string_lossy().into_owned(),
            color_space: {
                let n = ff::av_color_space_name((*par).color_space);
                if n.is_null() { "unknown".into() } else { CStr::from_ptr(n).to_string_lossy().into_owned() }
            },
            timebase: inp.timebase(),
            alpha_mode: inp.metadata("alpha_mode").as_deref() == Some("1"),
        }
    }
}

pub fn extradata(inp: &Input) -> Vec<u8> {
    unsafe {
        let par = inp.par();
        if (*par).extradata.is_null() {
            vec![]
        } else {
            std::slice::from_raw_parts((*par).extradata, (*par).extradata_size as usize).to_vec()
        }
    }
}

/// Packet list from FFmpeg's demuxer, used as the index for containers without
/// tables (MPEG-TS) and as a cross-check for the others.
pub struct DemuxPacket {
    pub pts: i64,
    pub dts: Option<i64>,
    pub duration: i64,
    pub key: bool,
    pub pos: i64,
    pub size: i32,
    pub discard: bool,
    /// First VCL NAL type of an Annex B H.264 or HEVC packet.
    pub nal_type: Option<u8>,
}

/// H.264 or HEVC, for NAL-type marking.
pub fn nal_codec(inp: &Input) -> Option<crate::index::NalCodec> {
    match unsafe { (*inp.par()).codec_id } {
        ff::AVCodecID::H264 => Some(crate::index::NalCodec::H264),
        ff::AVCodecID::HEVC => Some(crate::index::NalCodec::Hevc),
        _ => None,
    }
}

pub fn demux_scan(path: &Path) -> Result<(Vec<DemuxPacket>, Rational)> {
    let inp = Input::open(path)?;
    let codec = nal_codec(&inp);
    let mut out = Vec::new();
    unsafe {
        let pkt = ff::av_packet_alloc();
        loop {
            let r = ff::av_read_frame(inp.fmt, pkt);
            if r == ff::AVERROR_EOF {
                break;
            }
            check(r, "read")?;
            if (*pkt).stream_index == inp.stream {
                let pts = if (*pkt).pts == ff::AV_NOPTS_VALUE { (*pkt).dts } else { (*pkt).pts };
                out.push(DemuxPacket {
                    pts,
                    dts: ((*pkt).dts != ff::AV_NOPTS_VALUE).then_some((*pkt).dts),
                    duration: (*pkt).duration,
                    key: (*pkt).flags & ff::AV_PKT_FLAG_KEY != 0,
                    pos: (*pkt).pos,
                    size: (*pkt).size,
                    discard: (*pkt).flags & ff::AV_PKT_FLAG_DISCARD != 0,
                    nal_type: codec.and_then(|c| {
                        let data = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize);
                        // Annex B (MPEG-TS); length-prefixed containers are marked from their tables.
                        (data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1]))
                            .then(|| c.first_vcl(crate::vt::annexb_nals(data)))
                            .flatten()
                    }),
                });
            }
            ff::av_packet_unref(pkt);
        }
        let mut p = pkt;
        ff::av_packet_free(&mut p);
    }
    Ok((out, inp.timebase()))
}

/// Index for containers without frame tables (section 13: "scanning only where
/// none exist").
pub fn scan_index(path: &Path) -> Result<FrameIndex> {
    let t0 = std::time::Instant::now();
    let (pkts, tb) = demux_scan(path)?;
    ensure!(!pkts.is_empty(), "no packets");
    let codec = if pkts.iter().any(|p| p.nal_type.is_some()) { nal_codec(&Input::open(path)?) } else { None };
    let mut seen = std::collections::HashSet::new();
    for p in &pkts {
        ensure!(p.pos >= 0 && seen.insert(p.pos), "packet positions are not unique; demux index unusable");
        ensure!(p.pts != ff::AV_NOPTS_VALUE, "packet without timestamps");
    }
    let samples = pkts
        .iter()
        .map(|p| Sample {
            dts: p.dts,
            pts: p.pts,
            duration: p.duration,
            key: p.key,
            loc: Loc::Demux { pos: p.pos },
            presented: !p.discard,
            nal_type: p.nal_type,
        })
        .collect();
    let mut idx = FrameIndex {
        source: IndexSource::DemuxScan,
        timebase: tb,
        samples,
        order: vec![],
        sample_entry: None,
        codec_private: None,
        codec_id: None,
        nal_codec: codec,
        stats: IndexStats { scan_reads: pkts.len(), ..Default::default() },
    };
    idx.finish();
    idx.summarize_open_gop();
    idx.stats.build_ms = t0.elapsed().as_secs_f64() * 1e3;
    Ok(idx)
}

/// A compressed sample ready for either decoder.
#[derive(Clone)]
pub struct Packet {
    pub data: Vec<u8>,
    /// Matroska BlockAdditional id 1 (VP8/VP9 alpha).
    pub additional: Option<Vec<u8>>,
    pub pts: i64,
    pub dts: Option<i64>,
    pub duration: i64,
    pub key: bool,
}

/// Reads samples by index: file offsets where the container allows, otherwise
/// a byte seek in FFmpeg's demuxer to the first sample's packet.
pub struct PacketReader {
    file: std::fs::File,
    path: std::path::PathBuf,
    demux: Option<Input>,
    pub bytes_read: u64,
}

impl PacketReader {
    pub fn new(path: &Path) -> Result<Self> {
        Ok(PacketReader { file: std::fs::File::open(path)?, path: path.into(), demux: None, bytes_read: 0 })
    }

    /// Reads the given samples (ascending decode-order indices).
    pub fn read(&mut self, idx: &FrameIndex, which: &[usize]) -> Result<Vec<Packet>> {
        use std::os::unix::fs::FileExt;
        ensure!(which.windows(2).all(|w| w[0] < w[1]), "sample list not ascending");
        let Some(&first) = which.first() else { return Ok(vec![]) };
        match idx.samples[first].loc {
            Loc::File { .. } => which
                .iter()
                .map(|&i| {
                    let s = &idx.samples[i];
                    let Loc::File { offset, size, additional } = s.loc else { bail!("mixed locations") };
                    let mut data = vec![0u8; size as usize];
                    self.file.read_exact_at(&mut data, offset)?;
                    self.bytes_read += size as u64;
                    let additional = match additional {
                        Some((o, n)) => {
                            let mut b = vec![0u8; n as usize];
                            self.file.read_exact_at(&mut b, o)?;
                            Some(b)
                        }
                        None => None,
                    };
                    Ok(Packet { data, additional, pts: s.pts, dts: s.dts, duration: s.duration, key: s.key })
                })
                .collect(),
            Loc::Demux { pos } => {
                if self.demux.is_none() {
                    self.demux = Some(Input::open(&self.path)?);
                }
                let inp = self.demux.as_ref().unwrap();
                let want: Vec<(i64, usize)> = which
                    .iter()
                    .map(|&i| match idx.samples[i].loc {
                        Loc::Demux { pos } => Ok((pos, i)),
                        _ => Err(anyhow!("mixed locations")),
                    })
                    .collect::<Result<_>>()?;
                let mut out = Vec::with_capacity(want.len());
                unsafe {
                    check(ff::av_seek_frame(inp.fmt, inp.stream, pos, ff::AVSEEK_FLAG_BYTE), "byte seek")?;
                    let pkt = ff::av_packet_alloc();
                    let mut k = 0;
                    let mut err = None;
                    while k < want.len() {
                        let r = ff::av_read_frame(inp.fmt, pkt);
                        if r == ff::AVERROR_EOF {
                            break;
                        }
                        if r < 0 {
                            err = Some(anyhow!("read: {}", err_str(r)));
                            break;
                        }
                        if (*pkt).stream_index == inp.stream {
                            let p = (*pkt).pos;
                            if p == want[k].0 {
                                let s = &idx.samples[want[k].1];
                                let data = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize).to_vec();
                                self.bytes_read += data.len() as u64;
                                out.push(Packet { data, additional: None, pts: s.pts, dts: s.dts, duration: s.duration, key: s.key });
                                k += 1;
                            } else if p > want[k].0 {
                                err = Some(anyhow!("demuxer passed pos {} without returning the indexed packet", want[k].0));
                                break;
                            }
                        }
                        ff::av_packet_unref(pkt);
                    }
                    let mut p = pkt;
                    ff::av_packet_free(&mut p);
                    if let Some(e) = err {
                        return Err(e);
                    }
                    ensure!(k == want.len(), "demux seek delivered {k} of {} packets", want.len());
                }
                Ok(out)
            }
        }
    }
}

/// Owns one decoded AVFrame (kept alive while its pool planes are in use).
pub struct FfFrame {
    pub frame: *mut ff::AVFrame,
}

unsafe impl Send for FfFrame {}

impl Drop for FfFrame {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.frame) }
    }
}

pub struct FfDecoder {
    ctx: *mut ff::AVCodecContext,
    pool: Arc<MetalPool>,
    pkt: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    pub name: String,
    /// The decoder writes its output into `get_buffer2` frames itself.
    pub dr1: bool,
    /// The wrapper decodes into the library's own memory and copies each frame
    /// into a `get_buffer2` frame (FFmpeg's VP8 libvpx wrapper:
    /// `libvpxdec.c` calls `ff_get_buffer` then `av_image_copy2`). The frame
    /// lands in a pool slot, so the copy is invisible to the slot lookup and is
    /// declared here instead.
    pub internal_copy: bool,
    /// Frames copied, by the spike (decoders that ignore `get_buffer2`) or
    /// inside the wrapper (`internal_copy`).
    pub copies: usize,
    pub frames: usize,
}

unsafe impl Send for FfDecoder {}

/// FFmpeg decoders that advertise DR1 but copy from the library's own frame
/// into the `get_buffer2` frame (checked against FFmpeg 8.1.2's
/// `libavcodec/libvpxdec.c`: VP8 has no external frame buffers in libvpx).
const INTERNAL_COPY: [&str; 1] = ["libvpx"];

struct Opaque {
    pool: Arc<MetalPool>,
}

unsafe extern "C" fn get_buffer2(ctx: *mut ff::AVCodecContext, frame: *mut ff::AVFrame, flags: c_int) -> c_int {
    unsafe {
        let op = &*((*ctx).opaque as *const Opaque);
        let fmt = ff::AVPixelFormat((*frame).format);
        let Some(mut layout) = FrameLayout::for_format(fmt, (*frame).width, (*frame).height) else {
            return ff::avcodec_default_get_buffer2(ctx, frame, flags);
        };
        // Decoders may write past the visible size (macroblock and edge padding).
        let (mut w, mut h) = ((*frame).width, (*frame).height);
        let mut align = [0 as c_int; 8];
        ff::avcodec_align_dimensions2(ctx, &mut w, &mut h, align.as_mut_ptr());
        layout.pad_to(w as usize, h as usize + 2, &align);
        let Some(slot) = op.pool.acquire(layout.total) else { return ff::AVERROR(libc::ENOMEM) };
        let base = slot.ptr();
        // One buffer reference per plane, each holding the slot. A single
        // reference for every plane is allowed by the API, but FFmpeg 8.1's
        // HEVC alpha path (`replace_alpha_plane` in hevc/refs.c) swaps the
        // alpha-layer picture's luma buffer for the base picture's alpha
        // plane and keeps writing the other planes: with one reference that
        // frees the whole slot under a live decode, and the stray writes land
        // in whichever frame reuses it (measured: 128-valued chroma blocks in
        // random-access frames of hevc-alpha.mov in 6 of 12 runs at 4
        // threads).
        let n = layout.planes.len();
        for p in 0..n {
            let start = layout.planes[p].offset;
            let end = if p + 1 < n { layout.planes[p + 1].offset } else { layout.total };
            let token = Box::into_raw(Box::new(slot.clone()));
            let buf = ff::av_buffer_create(base.add(start), end - start, Some(release), token as *mut c_void, 0);
            if buf.is_null() {
                drop(Box::from_raw(token));
                for q in 0..p {
                    ff::av_buffer_unref(&mut (*frame).buf[q]);
                }
                return ff::AVERROR(libc::ENOMEM);
            }
            (*frame).buf[p] = buf;
            (*frame).data[p] = base.add(start);
            (*frame).linesize[p] = layout.planes[p].stride as c_int;
        }
        (*frame).extended_data = (*frame).data.as_mut_ptr();
        0
    }
}

/// Picks the first software format, logging the offered list when
/// `S6_LOG_GET_FORMAT` is set.
unsafe extern "C" fn get_format(_ctx: *mut ff::AVCodecContext, fmt: *const ff::AVPixelFormat) -> ff::AVPixelFormat {
    unsafe {
        let mut list = vec![];
        let mut p = fmt;
        while *p != ff::AVPixelFormat::NONE {
            list.push(*p);
            p = p.add(1);
        }
        if std::env::var_os("S6_LOG_GET_FORMAT").is_some() {
            let names: Vec<String> = list
                .iter()
                .map(|f| CStr::from_ptr(ff::av_get_pix_fmt_name(*f)).to_string_lossy().into_owned())
                .collect();
            eprintln!("get_format offered {names:?}");
        }
        list.into_iter()
            .find(|f| {
                let d = ff::av_pix_fmt_desc_get(*f);
                !d.is_null() && (*d).flags as i32 & ff::AV_PIX_FMT_FLAG_HWACCEL == 0
            })
            .unwrap_or(ff::AVPixelFormat::NONE)
    }
}

unsafe extern "C" fn release(opaque: *mut c_void, _data: *mut u8) {
    unsafe { drop(Box::from_raw(opaque as *mut SlotRef)) }
}

impl FfDecoder {
    pub fn new(inp: &Input, decoder: Option<&str>, pool: Arc<MetalPool>, threads: i32) -> Result<Self> {
        unsafe {
            let par = inp.par();
            let codec = match decoder {
                Some(n) => {
                    let c = CString::new(n)?;
                    ff::avcodec_find_decoder_by_name(c.as_ptr())
                }
                None => ff::avcodec_find_decoder((*par).codec_id),
            };
            ensure!(!codec.is_null(), "no FFmpeg decoder {:?}", decoder);
            let ctx = ff::avcodec_alloc_context3(codec);
            check(ff::avcodec_parameters_to_context(ctx, par), "params")?;
            (*ctx).pkt_timebase = (*inp.st()).time_base;
            (*ctx).thread_count = threads;
            let op = Box::into_raw(Box::new(Opaque { pool: pool.clone() }));
            (*ctx).opaque = op as *mut c_void;
            (*ctx).get_format = Some(get_format);
            if std::env::var_os("S6_DEFAULT_GET_BUFFER").is_none() {
                (*ctx).get_buffer2 = Some(get_buffer2);
            }
            check(ff::avcodec_open2(ctx, codec, ptr::null_mut()), "open decoder")?;
            let name = CStr::from_ptr((*codec).name).to_string_lossy().into_owned();
            let internal_copy = INTERNAL_COPY.contains(&name.as_str());
            let dr1 = (*codec).capabilities & ff::AV_CODEC_CAP_DR1 as c_int != 0 && !internal_copy;
            Ok(FfDecoder {
                ctx,
                pool,
                pkt: ff::av_packet_alloc(),
                frame: ff::av_frame_alloc(),
                name,
                dr1,
                internal_copy,
                copies: 0,
                frames: 0,
            })
        }
    }

    pub fn send(&mut self, p: Option<&Packet>, out: &mut Vec<(i64, PoolFrame)>) -> Result<()> {
        unsafe {
            if let Some(p) = p {
                ff::av_packet_unref(self.pkt);
                check(ff::av_new_packet(self.pkt, p.data.len() as c_int), "alloc packet")?;
                ptr::copy_nonoverlapping(p.data.as_ptr(), (*self.pkt).data, p.data.len());
                (*self.pkt).pts = p.pts;
                (*self.pkt).dts = p.dts.unwrap_or(ff::AV_NOPTS_VALUE);
                (*self.pkt).duration = p.duration;
                if p.key {
                    (*self.pkt).flags |= ff::AV_PKT_FLAG_KEY;
                }
                if let Some(a) = &p.additional {
                    let sd = ff::av_packet_new_side_data(
                        self.pkt,
                        ff::AVPacketSideDataType::MATROSKA_BLOCKADDITIONAL,
                        8 + a.len(),
                    );
                    ensure!(!sd.is_null(), "side data");
                    ptr::copy_nonoverlapping(1u64.to_be_bytes().as_ptr(), sd, 8);
                    ptr::copy_nonoverlapping(a.as_ptr(), sd.add(8), a.len());
                }
                check(ff::avcodec_send_packet(self.ctx, self.pkt), "send packet")?;
            } else {
                let r = ff::avcodec_send_packet(self.ctx, ptr::null());
                if r != ff::AVERROR_EOF {
                    check(r, "send flush")?;
                }
            }
            loop {
                let r = ff::avcodec_receive_frame(self.ctx, self.frame);
                if r == ff::AVERROR(libc::EAGAIN) || r == ff::AVERROR_EOF {
                    break;
                }
                check(r, "receive frame")?;
                let pts = if (*self.frame).pts != ff::AV_NOPTS_VALUE {
                    (*self.frame).pts
                } else {
                    (*self.frame).best_effort_timestamp
                };
                let f = ff::av_frame_alloc();
                ff::av_frame_move_ref(f, self.frame);
                out.push((pts, self.adopt(f)?));
            }
        }
        Ok(())
    }

    /// Wraps a frame; copies it into a pool slot only if the decoder did not
    /// render into one (decoders that ignore `get_buffer2`: libdav1d,
    /// libvpx-vp9).
    fn adopt(&mut self, f: *mut ff::AVFrame) -> Result<PoolFrame> {
        unsafe {
            self.frames += 1;
            let fmt = ff::AVPixelFormat((*f).format);
            let mut layout = FrameLayout::for_format(fmt, (*f).width, (*f).height)
                .with_context(|| format!("unsupported decoded pixel format {}", fmt.0))?;
            let d = ff::av_pix_fmt_desc_get(fmt);
            let label = CStr::from_ptr(ff::av_get_pix_fmt_name(fmt)).to_string_lossy().into_owned();
            let full_range = (*f).color_range == ff::AVColorRange::JPEG || label.starts_with("yuvj");
            let decoder_ptrs: Vec<usize> = (0..layout.planes.len()).map(|p| (*f).data[p] as usize).collect();
            // Each plane in a live pool slot (normally one slot per frame).
            let found: Option<Vec<SlotRef>> =
                (0..layout.planes.len()).map(|p| self.pool.find_containing((*f).data[p])).collect();
            let (slots, copied) = if let Some(slots) = found {
                // The frame is in our slots: rendered there by the decoder, or
                // copied there by the wrapper (`internal_copy`).
                for (p, (pl, slot)) in layout.planes.iter_mut().zip(&slots).enumerate() {
                    pl.offset = (*f).data[p].offset_from(slot.ptr()) as usize;
                    pl.stride = (*f).linesize[p] as usize;
                }
                if self.internal_copy {
                    self.copies += 1;
                }
                (slots, self.internal_copy)
            } else {
                // Not ours: one CPU copy into Metal-shared memory.
                let align = [0 as c_int; 8];
                layout.pad_to((*f).width as usize, (*f).height as usize, &align);
                let slot = self.pool.acquire(layout.total).context("pool")?;
                for (p, pl) in layout.planes.iter().enumerate() {
                    let row = pl.width * pl.bytes_per_px;
                    for y in 0..pl.height {
                        ptr::copy_nonoverlapping(
                            (*f).data[p].add(y * (*f).linesize[p] as usize),
                            slot.ptr().add(pl.offset + y * pl.stride),
                            row,
                        );
                    }
                }
                self.copies += 1;
                (vec![slot; layout.planes.len()], true)
            };
            let planes = layout
                .planes
                .iter()
                .zip(decoder_ptrs)
                .zip(slots)
                .map(|((pl, dp), slot)| PoolPlane {
                    slot,
                    offset: pl.offset,
                    stride: pl.stride,
                    width: pl.width,
                    height: pl.height,
                    bytes_per_px: pl.bytes_per_px,
                    decoder_ptr: dp,
                })
                .collect();
            Ok(PoolFrame {
                planes,
                depth: layout.depth,
                log2_chroma_w: (*d).log2_chroma_w,
                log2_chroma_h: (*d).log2_chroma_h,
                full_range,
                label,
                copied,
                owner: Some(Box::new(FfFrame { frame: f })),
            })
        }
    }

    pub fn flush(&mut self) {
        unsafe { ff::avcodec_flush_buffers(self.ctx) }
    }
}

impl Drop for FfDecoder {
    fn drop(&mut self) {
        unsafe {
            let op = (*self.ctx).opaque as *mut Opaque;
            ff::avcodec_free_context(&mut self.ctx);
            drop(Box::from_raw(op));
            ff::av_packet_free(&mut self.pkt);
            ff::av_frame_free(&mut self.frame);
        }
    }
}
