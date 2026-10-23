//! Reproduce the shipped TIFF → FFmpeg PNG → Blink bitmap at level 1.
//! Decode and negotiate integer RGB through the same linked LGPL FFmpeg;
//! the PNG round trip itself does not alter those stored sample values.
use super::{ImageBackend, ImageColor, Rgba, StillImage, icc, orient, pool_rgba};
use crate::{MediaError, Result, ff::check, pool::FramePool, sys::ffmpeg as f};
use pm_protocol::color::{AlphaMode, Transfer};
use std::{path::Path, ptr};

struct Decode {
    codec: *mut f::AVCodecContext,
    frame: *mut f::AVFrame,
    packet: *mut f::AVPacket,
    scale: *mut f::SwsContext,
}

impl Drop for Decode {
    fn drop(&mut self) {
        unsafe {
            f::sws_freeContext(self.scale);
            f::av_packet_free(&mut self.packet);
            f::av_frame_free(&mut self.frame);
            f::avcodec_free_context(&mut self.codec);
        }
    }
}

pub(super) fn read(path: &Path, orientation: u8, pool: &FramePool) -> Result<StillImage> {
    read_source(path, orientation, pool, false)
}

/// Samples of the PNG rendition itself, before Blink reduces it to its
/// 8-bit bitmap. Blender receives this PNG directly in today's app.
pub(super) fn read_source(
    path: &Path,
    orientation: u8,
    pool: &FramePool,
    full_depth: bool,
) -> Result<StillImage> {
    crate::ff::init();
    let bytes = std::fs::read(path)?;
    let size = i32::try_from(bytes.len()).map_err(|_| MediaError::invalid("TIFF packet size"))?;
    unsafe {
        let codec = f::avcodec_find_decoder(f::AV_CODEC_ID_TIFF);
        if codec.is_null() {
            return Err(MediaError::decode("FFmpeg TIFF decoder unavailable"));
        }
        let mut d = Decode {
            codec: f::avcodec_alloc_context3(codec),
            frame: f::av_frame_alloc(),
            packet: f::av_packet_alloc(),
            scale: ptr::null_mut(),
        };
        if d.codec.is_null() || d.frame.is_null() || d.packet.is_null() {
            return Err(MediaError::decode("TIFF decoder allocation"));
        }
        (*d.codec).thread_count = 1;
        check(
            f::avcodec_open2(d.codec, codec, ptr::null_mut()),
            "open TIFF",
        )?;
        check(f::av_new_packet(d.packet, size), "TIFF packet")?;
        ptr::copy_nonoverlapping(bytes.as_ptr(), (*d.packet).data, bytes.len());
        check(f::avcodec_send_packet(d.codec, d.packet), "decode TIFF")?;
        check(f::avcodec_receive_frame(d.codec, d.frame), "TIFF frame")?;
        let frame = &*d.frame;
        let (w, h) = (frame.width, frame.height);
        let descriptor = f::av_pix_fmt_desc_get(frame.format);
        if descriptor.is_null() || w <= 0 || h <= 0 {
            return Err(MediaError::invalid("TIFF decoded format"));
        }
        let alpha = (*descriptor).flags & u64::from(f::AV_PIX_FMT_FLAG_ALPHA) != 0;
        let descriptor = &*descriptor;
        let wide = descriptor.comp[..descriptor.nb_components as usize]
            .iter()
            .any(|c| c.depth > 8);
        let bpp = if wide { 8 } else { 4 };
        let length = (w as usize)
            .checked_mul(h as usize)
            .and_then(|n| n.checked_mul(bpp))
            .filter(|n| *n <= 1 << 30)
            .ok_or_else(|| MediaError::invalid("TIFF bitmap size"))?;
        let format = if wide {
            f::AV_PIX_FMT_RGBA64BE
        } else {
            f::AV_PIX_FMT_RGBA
        };
        d.scale = f::sws_getContext(
            w,
            h,
            frame.format,
            w,
            h,
            format,
            f::SWS_BICUBIC as i32,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null(),
        );
        if d.scale.is_null() {
            return Err(MediaError::decode("TIFF PNG pixel conversion"));
        }
        let mut stored = vec![0u8; length];
        let mut dst = [ptr::null_mut(); 4];
        dst[0] = stored.as_mut_ptr();
        let stride = [
            w.checked_mul(bpp as i32)
                .ok_or_else(|| MediaError::invalid("TIFF stride"))?,
            0,
            0,
            0,
        ];
        let src: Vec<*const u8> = frame.data.iter().map(|p| p.cast_const()).collect();
        check(
            f::sws_scale(
                d.scale,
                src.as_ptr(),
                frame.linesize.as_ptr(),
                0,
                h,
                dst.as_ptr(),
                stride.as_ptr(),
            ),
            "TIFF RGB",
        )?;
        let (frame, depth) = if wide && full_depth {
            let samples: Vec<u16> = stored
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            let (rgba, w, h) = orient(&samples, w as usize, h as usize, 4, orientation);
            (
                pool_rgba(
                    pool,
                    w,
                    h,
                    Rgba::U16(&rgba),
                    false,
                    16,
                    "FFmpeg TIFF PNG samples".into(),
                )?,
                16,
            )
        } else {
            let rgba: Vec<u8> = if wide {
                stored.chunks_exact(2).map(|c| c[0]).collect()
            } else {
                stored
            };
            let (rgba, w, h) = orient(&rgba, w as usize, h as usize, 4, orientation);
            (
                pool_rgba(
                    pool,
                    w,
                    h,
                    Rgba::U8(&rgba),
                    false,
                    8,
                    "FFmpeg TIFF PNG samples".into(),
                )?,
                8,
            )
        };
        let side = f::av_frame_get_side_data(d.frame, f::AV_FRAME_DATA_ICC_PROFILE);
        let profile = if side.is_null() {
            None
        } else {
            let bytes = std::slice::from_raw_parts((*side).data, (*side).size);
            // Blink rejects a PNG ICC profile whose model disagrees with RGB.
            matches!(bytes.get(16..20), Some(b"RGB " | b"GRAY")).then(|| bytes.to_vec())
        };
        Ok(StillImage {
            frame,
            backend: ImageBackend::Ffmpeg,
            bit_depth: depth,
            float: false,
            color: ImageColor {
                icc_class: profile.as_deref().map(icc::classify),
                icc_bytes: profile.as_ref().map(Vec::len),
                icc: profile,
                transfer: Transfer::DisplayE,
                alpha: if alpha {
                    AlphaMode::Straight
                } else {
                    AlphaMode::Opaque
                },
                chromaticities: None,
            },
            format: super::ImageFormat::Tiff,
            orientation,
        })
    }
}
