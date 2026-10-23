//! FFmpeg software decoding straight into pool memory.
//!
//! A custom `get_buffer2` gives decoders with direct rendering (`DR1`) their
//! output planes inside pool slots, so the GPU importer can wrap them without
//! a copy. Every plane gets its own `AVBufferRef` (section 26 amendment 6):
//! FFmpeg's HEVC alpha path replaces one plane's buffer reference of a picture
//! it is still decoding (`replace_alpha_plane` in `hevc/refs.c`), and with a
//! single reference for all planes that freed the whole slot under a live
//! decode (S6 finding 2).
//!
//! Decoders that ignore `get_buffer2` (wrappers around libraries with their
//! own allocators) and pixel formats the importer cannot sample are copied
//! into a pool frame once; such frames report `copied`.

use super::{Input, Packet, check, cstr};
use crate::error::{MediaError, Result};
use crate::frame::{
    DecodedFrame, Layout, PixelFormat, Pixels, PlaneDesc, PlaneFormat, PoolFrame, PoolPlane,
};
use crate::pool::{FramePool, PAGE, ROW_ALIGN, SlotRef, ceil_rshift};
use crate::sys::ffmpeg as ff;
use pm_protocol::color::{Range, SampleAlignment};
use std::ffi::{CString, c_int, c_void};
use std::ptr;
use std::sync::Arc;

/// Planar layout of a pixel format the importer samples directly: every
/// component on its own plane, LSB-aligned, little-endian, one depth.
#[derive(Clone, Debug)]
pub struct PlanarLayout {
    pub rgb: bool,
    pub depth: u8,
    pub alpha: bool,
    pub log2_chroma_w: u8,
    pub log2_chroma_h: u8,
    pub planes: Vec<PlanePlacement>,
    pub total: usize,
}

#[derive(Clone, Debug)]
pub struct PlanePlacement {
    pub offset: usize,
    pub stride: usize,
    /// Visible size in samples.
    pub width: usize,
    pub height: usize,
    pub bytes_per_sample: usize,
    pub log2_w: u8,
    pub log2_h: u8,
}

impl PlanarLayout {
    /// The layout for `fmt`, or None when the importer cannot sample it as is.
    pub fn for_format(fmt: ff::AVPixelFormat, width: i32, height: i32) -> Option<PlanarLayout> {
        unsafe {
            let d = ff::av_pix_fmt_desc_get(fmt);
            if d.is_null() {
                return None;
            }
            let d = &*d;
            let flags = d.flags as u32;
            if flags
                & (ff::AV_PIX_FMT_FLAG_HWACCEL
                    | ff::AV_PIX_FMT_FLAG_PAL
                    | ff::AV_PIX_FMT_FLAG_BITSTREAM
                    | ff::AV_PIX_FMT_FLAG_FLOAT)
                != 0
            {
                return None;
            }
            let n = d.nb_components as usize;
            if n < 3 || flags & ff::AV_PIX_FMT_FLAG_PLANAR == 0 {
                return None;
            }
            let depth = d.comp[0].depth;
            if depth > 16 || (depth > 8 && flags & ff::AV_PIX_FMT_FLAG_BE != 0) {
                return None;
            }
            let rgb = flags & ff::AV_PIX_FMT_FLAG_RGB != 0;
            let mut planes = Vec::with_capacity(n);
            for i in 0..n {
                // Y′CbCr(A): component i on plane i. RGB: planes hold G, B, R,
                // then A (gbrp order), and the descriptor lists R, G, B, A.
                let comp = if rgb { [1, 2, 0, 3][i] } else { i };
                let c = d.comp[comp];
                if c.plane as usize != i || c.offset != 0 || c.shift != 0 || c.depth != depth {
                    return None;
                }
                let chroma = !rgb && (i == 1 || i == 2);
                let (lw, lh) = if chroma {
                    (d.log2_chroma_w, d.log2_chroma_h)
                } else {
                    (0, 0)
                };
                planes.push(PlanePlacement {
                    offset: 0,
                    stride: 0,
                    width: ceil_rshift(width as usize, lw),
                    height: ceil_rshift(height as usize, lh),
                    bytes_per_sample: c.step as usize,
                    log2_w: lw,
                    log2_h: lh,
                });
            }
            let mut l = PlanarLayout {
                rgb,
                depth: depth as u8,
                alpha: flags & ff::AV_PIX_FMT_FLAG_ALPHA != 0,
                log2_chroma_w: if rgb { 0 } else { d.log2_chroma_w },
                log2_chroma_h: if rgb { 0 } else { d.log2_chroma_h },
                planes,
                total: 0,
            };
            l.pad_to(width as usize, height as usize, &[0; 8]);
            Some(l)
        }
    }

    /// Places planes for a coded size of `w` × `h` with FFmpeg's per-plane
    /// linesize alignment; every offset and stride stays Metal-aligned.
    pub fn pad_to(&mut self, w: usize, h: usize, linesize_align: &[i32; 8]) {
        let mut off = 0;
        for (i, p) in self.planes.iter_mut().enumerate() {
            let a =
                (ROW_ALIGN.max(linesize_align[i].max(1) as usize)).div_ceil(ROW_ALIGN) * ROW_ALIGN;
            p.stride = (ceil_rshift(w, p.log2_w) * p.bytes_per_sample).div_ceil(a) * a;
            p.offset = off;
            off += p.stride * ceil_rshift(h, p.log2_h);
            off = off.div_ceil(ROW_ALIGN) * ROW_ALIGN;
        }
        // Tail padding for decoders that over-read the last row.
        self.total = (off + ROW_ALIGN * 4).div_ceil(PAGE) * PAGE;
    }

    pub fn pixel_format(
        &self,
        width: usize,
        height: usize,
        range: Range,
        label: String,
    ) -> PixelFormat {
        let pf = if self.depth > 8 {
            PlaneFormat::R16
        } else {
            PlaneFormat::R8
        };
        PixelFormat {
            layout: if self.rgb {
                Layout::PlanarRgb
            } else {
                Layout::Planar
            },
            bits: self.depth,
            alignment: SampleAlignment::Lsb,
            range: if self.rgb { Range::Full } else { range },
            log2_chroma_w: self.log2_chroma_w,
            log2_chroma_h: self.log2_chroma_h,
            alpha_bits: if self.alpha { self.depth } else { 0 },
            planes: self
                .planes
                .iter()
                .map(|p| PlaneDesc {
                    format: pf,
                    width: p.width,
                    height: p.height,
                })
                .collect(),
            width,
            height,
            label,
            rgba_premultiplied: false,
        }
    }
}

struct Opaque {
    pool: FramePool,
    chromium_h264_padding: bool,
}

unsafe extern "C" fn get_buffer2(
    ctx: *mut ff::AVCodecContext,
    frame: *mut ff::AVFrame,
    flags: c_int,
) -> c_int {
    unsafe {
        let op = &*((*ctx).opaque as *const Opaque);
        let Some(mut layout) =
            PlanarLayout::for_format((*frame).format, (*frame).width, (*frame).height)
        else {
            return ff::avcodec_default_get_buffer2(ctx, frame, flags);
        };
        // Decoders may write past the visible size (macroblock and edge padding).
        let (mut w, mut h) = ((*frame).width, (*frame).height);
        let mut align = [0 as c_int; 8];
        ff::avcodec_align_dimensions2(ctx, &mut w, &mut h, align.as_mut_ptr());
        layout.pad_to(w as usize, h as usize + 2, &align);
        let Ok(slot) = op.pool.acquire(layout.total) else {
            return ff::averror(libc::ENOMEM);
        };
        let base = slot.ptr();
        if op.chromium_h264_padding {
            // Chromium's software H.264 buffer starts cleared. Reused pool
            // slots must have the same untouched macroblock-edge pixels.
            ptr::write_bytes(base, 0, slot.len());
        }
        let n = layout.planes.len();
        for p in 0..n {
            let start = layout.planes[p].offset;
            let end = if p + 1 < n {
                layout.planes[p + 1].offset
            } else {
                layout.total
            };
            let token = Box::into_raw(Box::new(slot.clone()));
            let buf = ff::av_buffer_create(
                base.add(start),
                end - start,
                Some(release),
                token as *mut c_void,
                0,
            );
            if buf.is_null() {
                drop(Box::from_raw(token));
                for q in 0..p {
                    ff::av_buffer_unref(&mut (*frame).buf[q]);
                }
                return ff::averror(libc::ENOMEM);
            }
            (*frame).buf[p] = buf;
            (*frame).data[p] = base.add(start);
            (*frame).linesize[p] = layout.planes[p].stride as c_int;
        }
        (*frame).extended_data = (*frame).data.as_mut_ptr();
        0
    }
}

unsafe extern "C" fn release(opaque: *mut c_void, _data: *mut u8) {
    unsafe { drop(Box::from_raw(opaque as *mut SlotRef)) }
}

/// Picks the first software format the decoder offers.
unsafe extern "C" fn get_format(
    _ctx: *mut ff::AVCodecContext,
    fmt: *const ff::AVPixelFormat,
) -> ff::AVPixelFormat {
    unsafe {
        let mut p = fmt;
        while *p != ff::AV_PIX_FMT_NONE {
            let d = ff::av_pix_fmt_desc_get(*p);
            if !d.is_null() && (*d).flags as u32 & ff::AV_PIX_FMT_FLAG_HWACCEL == 0 {
                return *p;
            }
            p = p.add(1);
        }
        ff::AV_PIX_FMT_NONE
    }
}

/// Owns one decoded AVFrame while its planes are in use.
struct FfFrame(*mut ff::AVFrame);

unsafe impl Send for FfFrame {}
unsafe impl Sync for FfFrame {}

impl Drop for FfFrame {
    fn drop(&mut self) {
        unsafe { ff::av_frame_free(&mut self.0) }
    }
}

/// FFmpeg decoders that advertise DR1 but copy from the library's own frame
/// into the `get_buffer2` frame (FFmpeg's VP8 libvpx wrapper calls
/// `ff_get_buffer` then `av_image_copy2`; libvpx's VP8 decoder has no
/// external frame buffers).
const INTERNAL_COPY: [&str; 1] = ["libvpx"];

/// Interlacing reported on decoded frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FieldFlags {
    pub interlaced: bool,
    pub top_field_first: bool,
}

pub struct FfDecoder {
    ctx: *mut ff::AVCodecContext,
    pool: FramePool,
    pkt: *mut ff::AVPacket,
    frame: *mut ff::AVFrame,
    name: String,
    dr1: bool,
    internal_copy: bool,
    chromium_h264_padding: bool,
    draining: bool,
    /// Frames copied into the pool (decoders without direct rendering, or
    /// formats converted for the importer).
    pub copies: usize,
    /// Frames FFmpeg flagged corrupt (dropped, never returned).
    pub corrupt: usize,
    /// Interlacing of the last decoded frame.
    pub last_fields: FieldFlags,
}

unsafe impl Send for FfDecoder {}

impl FfDecoder {
    /// Opens a decoder for stream `stream` of `input`: FFmpeg's native
    /// decoder for the codec, or `name` (for example `libdav1d`).
    pub fn new(
        input: &Input,
        stream: usize,
        name: Option<&str>,
        pool: FramePool,
        threads: i32,
    ) -> Result<Self> {
        unsafe {
            let par = input.par(stream);
            let codec = match name {
                Some(n) => {
                    let c = CString::new(n).map_err(|_| MediaError::invalid("decoder name"))?;
                    ff::avcodec_find_decoder_by_name(c.as_ptr())
                }
                None => ff::avcodec_find_decoder((*par).codec_id),
            };
            if codec.is_null() {
                return Err(MediaError::unsupported(format!(
                    "no FFmpeg decoder {} for {}",
                    name.unwrap_or("(native)"),
                    input.codec_name(stream)
                )));
            }
            let mut ctx = ff::avcodec_alloc_context3(codec);
            if ctx.is_null() {
                return Err(MediaError::OutOfMemory(0));
            }
            if let Err(e) = check(
                ff::avcodec_parameters_to_context(ctx, par),
                "codec parameters",
            ) {
                ff::avcodec_free_context(&mut ctx);
                return Err(e);
            }
            (*ctx).pkt_timebase = (*input.st(stream)).time_base;
            (*ctx).thread_count = threads;
            let op = Box::into_raw(Box::new(Opaque {
                pool: pool.clone(),
                chromium_h264_padding: false,
            }));
            (*ctx).opaque = op as *mut c_void;
            (*ctx).get_format = Some(get_format);
            (*ctx).get_buffer2 = Some(get_buffer2);
            if let Err(e) = check(
                ff::avcodec_open2(ctx, codec, ptr::null_mut()),
                "open decoder",
            ) {
                ff::avcodec_free_context(&mut ctx);
                drop(Box::from_raw(op));
                return Err(e);
            }
            let name = cstr((*codec).name).unwrap_or_default();
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
                chromium_h264_padding: false,
                draining: false,
                copies: 0,
                corrupt: 0,
                last_fields: FieldFlags::default(),
            })
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The decoder writes its output into pool frames itself.
    pub fn direct(&self) -> bool {
        self.dr1
    }

    /// Chromium's full-range 8-bit H.264 software decoder exposes its
    /// cleared, aligned coded YUV planes while retaining visible geometry.
    /// The option is set before the first packet and never affects native
    /// decoding or other codecs.
    pub fn with_chromium_h264_padding(mut self) -> Result<Self> {
        if self.name != "h264" || !self.dr1 {
            return Err(MediaError::unsupported(
                "Chromium H.264 coded padding requires FFmpeg's direct h264 decoder",
            ));
        }
        self.chromium_h264_padding = true;
        unsafe {
            (*(self.ctx.as_ref().unwrap().opaque as *mut Opaque)).chromium_h264_padding = true;
        }
        Ok(self)
    }

    /// Sends one packet (`None` drains) and appends every frame now available.
    pub fn send(&mut self, p: Option<&Packet>, out: &mut Vec<DecodedFrame>) -> Result<()> {
        unsafe {
            match p {
                Some(p) => {
                    if self.draining {
                        self.flush();
                    }
                    ff::av_packet_unref(self.pkt);
                    check(
                        ff::av_new_packet(self.pkt, p.data.len() as c_int),
                        "allocate packet",
                    )?;
                    ptr::copy_nonoverlapping(p.data.as_ptr(), (*self.pkt).data, p.data.len());
                    (*self.pkt).pts = p.pts;
                    (*self.pkt).dts = p.dts.unwrap_or(ff::AV_NOPTS_VALUE);
                    (*self.pkt).duration = p.duration;
                    if p.key {
                        (*self.pkt).flags |= ff::AV_PKT_FLAG_KEY as c_int;
                    }
                    if let Some(a) = &p.additional {
                        let sd = ff::av_packet_new_side_data(
                            self.pkt,
                            ff::AV_PKT_DATA_MATROSKA_BLOCKADDITIONAL,
                            8 + a.len(),
                        );
                        if sd.is_null() {
                            return Err(MediaError::OutOfMemory(a.len()));
                        }
                        ptr::copy_nonoverlapping(1u64.to_be_bytes().as_ptr(), sd, 8);
                        ptr::copy_nonoverlapping(a.as_ptr(), sd.add(8), a.len());
                    }
                    let r = ff::avcodec_send_packet(self.ctx, self.pkt);
                    // Damaged packets are reported per frame; a decoder that
                    // rejects one keeps going with the next.
                    if r < 0 && r != ff::AVERROR_INVALIDDATA {
                        check(r, "send packet")?;
                    }
                }
                None => {
                    if !self.draining {
                        let r = ff::avcodec_send_packet(self.ctx, ptr::null());
                        if r != ff::AVERROR_EOF {
                            check(r, "drain")?;
                        }
                        self.draining = true;
                    }
                }
            }
            loop {
                let r = ff::avcodec_receive_frame(self.ctx, self.frame);
                if r == ff::averror(libc::EAGAIN) || r == ff::AVERROR_EOF {
                    break;
                }
                if r == ff::AVERROR_INVALIDDATA {
                    self.corrupt += 1;
                    continue;
                }
                check(r, "receive frame")?;
                let flags = (*self.frame).flags as u32;
                if flags & ff::AV_FRAME_FLAG_CORRUPT != 0 || (*self.frame).decode_error_flags != 0 {
                    if std::env::var_os("PM_MEDIA_TRACE").is_some() {
                        eprintln!(
                            "{}: corrupt frame pts {} (flags {flags:#x}, decode_error_flags {:#x})",
                            self.name,
                            (*self.frame).pts,
                            (*self.frame).decode_error_flags
                        );
                    }
                    self.corrupt += 1;
                    ff::av_frame_unref(self.frame);
                    continue;
                }
                self.last_fields = FieldFlags {
                    interlaced: flags & ff::AV_FRAME_FLAG_INTERLACED != 0,
                    top_field_first: flags & ff::AV_FRAME_FLAG_TOP_FIELD_FIRST != 0,
                };
                let pts = if (*self.frame).pts != ff::AV_NOPTS_VALUE {
                    (*self.frame).pts
                } else {
                    (*self.frame).best_effort_timestamp
                };
                let f = ff::av_frame_alloc();
                ff::av_frame_move_ref(f, self.frame);
                out.push(self.adopt(pts, f)?);
            }
        }
        Ok(())
    }

    /// Wraps a decoded frame. Frames already in pool slots are used as they
    /// are; others are copied into a pool frame once.
    fn adopt(&mut self, pts: i64, f: *mut ff::AVFrame) -> Result<DecodedFrame> {
        let owner = FfFrame(f);
        unsafe {
            let fmt = (*f).format;
            let (w, h) = ((*f).width as usize, (*f).height as usize);
            let label = cstr(ff::av_get_pix_fmt_name(fmt)).unwrap_or_default();
            let full = (*f).color_range == ff::AVCOL_RANGE_JPEG || label.starts_with("yuvj");
            let range = if full { Range::Full } else { Range::Limited };
            if let Some(mut layout) = PlanarLayout::for_format(fmt, (*f).width, (*f).height) {
                if self.chromium_h264_padding {
                    if layout.rgb
                        || layout.depth != 8
                        || layout.log2_chroma_w != 1
                        || layout.log2_chroma_h != 1
                    {
                        return Err(MediaError::unsupported(
                            "Chromium H.264 coded padding requires planar 8-bit 4:2:0",
                        ));
                    }
                    let (mut coded_w, mut coded_h) = ((*f).width, (*f).height);
                    let mut align = [0 as c_int; 8];
                    ff::avcodec_align_dimensions2(
                        self.ctx,
                        &mut coded_w,
                        &mut coded_h,
                        align.as_mut_ptr(),
                    );
                    for plane in &mut layout.planes {
                        plane.width = ceil_rshift(coded_w as usize, plane.log2_w);
                        plane.height = ceil_rshift(coded_h as usize, plane.log2_h);
                    }
                }
                let slots: Option<Vec<SlotRef>> = (0..layout.planes.len())
                    .map(|p| self.pool.find_containing((*f).data[p]))
                    .collect();
                if let Some(slots) = slots {
                    if self.chromium_h264_padding {
                        for (p, slot) in slots.iter().enumerate() {
                            let offset = (*f).data[p].offset_from(slot.ptr()) as usize;
                            let stride = (*f).linesize[p] as usize;
                            let plane = &layout.planes[p];
                            let end = offset
                                .checked_add(
                                    (plane.height - 1).checked_mul(stride).ok_or_else(|| {
                                        MediaError::invalid("coded plane overflow")
                                    })?,
                                )
                                .and_then(|n| {
                                    plane
                                        .width
                                        .checked_mul(plane.bytes_per_sample)
                                        .and_then(|width| n.checked_add(width))
                                })
                                .ok_or_else(|| MediaError::invalid("coded plane overflow"))?;
                            let next = if p + 1 < slots.len() && slots[p + 1].ptr() == slot.ptr() {
                                (*f).data[p + 1].offset_from(slot.ptr()) as usize
                            } else {
                                slot.len()
                            };
                            if end > next || next > slot.len() {
                                return Err(MediaError::decode(
                                    "Chromium H.264 coded plane exceeds pool slot",
                                ));
                            }
                        }
                    }
                    // Rendered into our slots by the decoder (or copied there by
                    // a wrapper that advertises DR1 but copies internally).
                    if self.internal_copy {
                        self.copies += 1;
                    }
                    let planes = slots
                        .into_iter()
                        .enumerate()
                        .map(|(p, slot)| PoolPlane {
                            offset: (*f).data[p].offset_from(slot.ptr()) as usize,
                            stride: (*f).linesize[p] as usize,
                            decoder_ptr: (*f).data[p] as usize,
                            slot,
                        })
                        .collect();
                    return Ok(DecodedFrame {
                        pts,
                        format: Arc::new(layout.pixel_format(
                            w,
                            h,
                            range,
                            if self.chromium_h264_padding {
                                "Chromium I420 coded".into()
                            } else {
                                label
                            },
                        )),
                        pixels: Pixels::Pool(Arc::new(PoolFrame {
                            planes,
                            owner: Some(Box::new(owner)),
                            copied: self.internal_copy,
                        })),
                        color: None,
                    });
                }
                if self.chromium_h264_padding {
                    return Err(MediaError::unsupported(
                        "Chromium H.264 coded padding needs direct pool planes",
                    ));
                }
                // The decoder used its own memory: one copy of the planes.
                let mut l = layout.clone();
                l.pad_to(w, h, &[0; 8]);
                let slot = self.pool.acquire(l.total)?;
                for (p, pl) in l.planes.iter().enumerate() {
                    let row = pl.width * pl.bytes_per_sample;
                    for y in 0..pl.height {
                        ptr::copy_nonoverlapping(
                            (*f).data[p].offset(y as isize * (*f).linesize[p] as isize),
                            slot.ptr().add(pl.offset + y * pl.stride),
                            row,
                        );
                    }
                }
                self.copies += 1;
                let planes = l
                    .planes
                    .iter()
                    .map(|pl| PoolPlane {
                        slot: slot.clone(),
                        offset: pl.offset,
                        stride: pl.stride,
                        decoder_ptr: slot.ptr() as usize + pl.offset,
                    })
                    .collect();
                return Ok(DecodedFrame {
                    pts,
                    format: Arc::new(l.pixel_format(w, h, range, label)),
                    pixels: Pixels::Pool(Arc::new(PoolFrame {
                        planes,
                        owner: None,
                        copied: true,
                    })),
                    color: None,
                });
            }
            let converted = convert_to_planar(&self.pool, f, range, label)?;
            self.copies += 1;
            Ok(DecodedFrame { pts, ..converted })
        }
    }

    /// Drops all decoder state so decoding can restart at any random-access
    /// point (a seek within one session).
    pub fn flush(&mut self) {
        unsafe { ff::avcodec_flush_buffers(self.ctx) };
        self.draining = false;
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

/// Converts any FFmpeg pixel format the importer cannot sample (packed,
/// semi-planar, paletted, big-endian, gray) into planar Y′CbCr(A) or
/// G, B, R(A) at the format's own depth, with one copy.
unsafe fn convert_to_planar(
    pool: &FramePool,
    f: *const ff::AVFrame,
    range: Range,
    label: String,
) -> Result<DecodedFrame> {
    unsafe {
        let fmt = (*f).format;
        let d = ff::av_pix_fmt_desc_get(fmt);
        if d.is_null() {
            return Err(MediaError::unsupported(format!("pixel format {fmt}")));
        }
        let d = &*d;
        let flags = d.flags as u32;
        if flags
            & (ff::AV_PIX_FMT_FLAG_HWACCEL
                | ff::AV_PIX_FMT_FLAG_BITSTREAM
                | ff::AV_PIX_FMT_FLAG_FLOAT)
            != 0
        {
            return Err(MediaError::unsupported(format!("pixel format {label}")));
        }
        let (w, h) = ((*f).width as usize, (*f).height as usize);
        let pal = flags & ff::AV_PIX_FMT_FLAG_PAL != 0;
        let rgb = pal || flags & ff::AV_PIX_FMT_FLAG_RGB != 0;
        let gray = !rgb && d.nb_components <= 2;
        let alpha = flags & ff::AV_PIX_FMT_FLAG_ALPHA != 0;
        let depth = if pal {
            8
        } else {
            (0..d.nb_components as usize)
                .map(|c| d.comp[c].depth)
                .max()
                .unwrap_or(8) as u8
        };
        let (lw, lh) = if rgb || gray {
            (0, 0)
        } else {
            (d.log2_chroma_w, d.log2_chroma_h)
        };
        let bps = if depth > 8 { 2 } else { 1 };
        let n = 3 + alpha as usize;
        let mut layout = PlanarLayout {
            rgb,
            depth,
            alpha,
            log2_chroma_w: lw,
            log2_chroma_h: lh,
            planes: (0..n)
                .map(|i| {
                    let (pw, ph) = if !rgb && (i == 1 || i == 2) {
                        (lw, lh)
                    } else {
                        (0, 0)
                    };
                    PlanePlacement {
                        offset: 0,
                        stride: 0,
                        width: ceil_rshift(w, pw),
                        height: ceil_rshift(h, ph),
                        bytes_per_sample: bps,
                        log2_w: pw,
                        log2_h: ph,
                    }
                })
                .collect(),
            total: 0,
        };
        layout.pad_to(w, h, &[0; 8]);
        let slot = pool.acquire(layout.total)?;
        let mut line = vec![0u16; w + 1];
        let put = |plane: &PlanePlacement, y: usize, vals: &[u16]| {
            let row = slot.ptr().add(plane.offset + y * plane.stride);
            for (x, &v) in vals.iter().take(plane.width).enumerate() {
                if bps == 2 {
                    *(row as *mut u16).add(x) = v;
                } else {
                    *row.add(x) = v as u8;
                }
            }
        };
        // Planar output order: Y′ Cb Cr A, or G B R A (gbrp order).
        // Component indices in the descriptor: YUV(A) = 0,1,2,(3); RGB(A) = R,G,B,(A).
        let src_comp = |out_plane: usize| -> usize {
            if rgb {
                [1, 2, 0, 3][out_plane]
            } else {
                out_plane
            }
        };
        let neutral = if depth > 8 { 1u16 << (depth - 1) } else { 128 };
        for (op, plane) in layout.planes.iter().enumerate() {
            for y in 0..plane.height {
                if pal {
                    // Palette entries are native-endian 0xAARRGGBB.
                    let palette = (*f).data[1] as *const u32;
                    let idx = (*f).data[0].offset(y as isize * (*f).linesize[0] as isize);
                    for (x, slot) in line.iter_mut().enumerate().take(w) {
                        let argb = *palette.add(*idx.add(x) as usize);
                        *slot = match op {
                            0 => (argb >> 8) & 0xFF,
                            1 => argb & 0xFF,
                            2 => (argb >> 16) & 0xFF,
                            _ => argb >> 24,
                        } as u16;
                    }
                } else if gray && (op == 1 || op == 2) {
                    line[..w].fill(neutral);
                } else {
                    let c = if gray && op == 3 { 1 } else { src_comp(op) };
                    let mut data: [*const u8; 4] =
                        [(*f).data[0], (*f).data[1], (*f).data[2], (*f).data[3]];
                    let linesize: [c_int; 4] = [
                        (*f).linesize[0],
                        (*f).linesize[1],
                        (*f).linesize[2],
                        (*f).linesize[3],
                    ];
                    ff::av_read_image_line2(
                        line.as_mut_ptr() as *mut c_void,
                        data.as_mut_ptr(),
                        linesize.as_ptr(),
                        d,
                        0,
                        y as c_int,
                        c as c_int,
                        plane.width as c_int,
                        0,
                        2,
                    );
                    // Components shallower than the output depth are scaled up.
                    let cd = d.comp[c].depth as u8;
                    if cd < depth {
                        for v in &mut line[..plane.width] {
                            *v <<= depth - cd;
                        }
                    }
                }
                put(plane, y, &line[..plane.width]);
            }
        }
        let planes = layout
            .planes
            .iter()
            .map(|pl| PoolPlane {
                slot: slot.clone(),
                offset: pl.offset,
                stride: pl.stride,
                decoder_ptr: slot.ptr() as usize + pl.offset,
            })
            .collect();
        Ok(DecodedFrame {
            pts: 0,
            format: Arc::new(layout.pixel_format(w, h, range, format!("{label} (converted)"))),
            pixels: Pixels::Pool(Arc::new(PoolFrame {
                planes,
                owner: None,
                copied: true,
            })),
            color: None,
        })
    }
}
