//! JPEG through libjpeg-turbo (the TurboJPEG API), the library today's viewer
//! (Chromium) decodes JPEG with, with the same settings as its decoder and
//! `djpeg`: accurate integer IDCT, fancy (smooth) chroma upsampling, block
//! smoothing for progressive scans. Gray, Y′CbCr, and RGB JPEGs decode to
//! RGBA at their sample precision (8 or 12 bits; lossless JPEG at 2 to 16).
//! CMYK and YCCK JPEGs need their profile and go to ImageIO.
//!
//! EXIF orientation is read from the APP1 segment and applied, as the
//! browser applies it.

use super::{ImageBackend, ImageColor, ImageFormat, Rgba, StillImage, icc, orient, pool_rgba};
use crate::error::{MediaError, Result};
use crate::pool::FramePool;
use crate::sys::turbojpeg as tj;
use pm_protocol::color::{AlphaMode, Transfer};
use std::ffi::CStr;
use std::path::Path;

/// A TurboJPEG decompressor, destroyed on drop.
struct Handle(tj::tjhandle);

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe { tj::tj3Destroy(self.0) };
    }
}

impl Handle {
    fn new() -> Result<Handle> {
        // tj3Init(TJINIT_DECOMPRESS) is a macro over tj3InitVersion.
        // The API version is TURBOJPEG_VERSION_NUMBER of the pinned 3.2.0.
        let h = unsafe { tj::tj3InitVersion(tj::TJINIT_DECOMPRESS as i32, 3_002_000) };
        if h.is_null() {
            return Err(MediaError::Image("libjpeg-turbo could not start".into()));
        }
        Ok(Handle(h))
    }

    fn error(&self, what: &str) -> MediaError {
        let msg = unsafe { CStr::from_ptr(tj::tj3GetErrorStr(self.0)) }
            .to_string_lossy()
            .into_owned();
        MediaError::Image(format!("JPEG {what}: {msg}"))
    }

    fn set(&self, param: tj::TJPARAM, value: i32) -> Result<()> {
        if unsafe { tj::tj3Set(self.0, param as i32, value) } != 0 {
            return Err(self.error("parameter"));
        }
        Ok(())
    }

    fn get(&self, param: tj::TJPARAM) -> i32 {
        unsafe { tj::tj3Get(self.0, param as i32) }
    }
}

/// The JPEG's header facts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JpegInfo {
    pub width: usize,
    pub height: usize,
    /// Sample precision in bits (8 or 12; 2 to 16 for lossless JPEG).
    pub precision: u8,
    pub lossless: bool,
    /// TurboJPEG colorspace (`TJCS_*`).
    pub colorspace: i32,
    /// TurboJPEG subsampling (`TJSAMP_*`), -1 when unknown.
    pub subsampling: i32,
}

/// EXIF orientation (1 to 8) from a JPEG's APP1 segments, 1 when absent.
pub fn exif_orientation(bytes: &[u8]) -> u8 {
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            break;
        }
        let marker = bytes[i + 1];
        if marker == 0xD8 || (0xD0..=0xD7).contains(&marker) {
            i += 2;
            continue;
        }
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        let seg = &bytes[(i + 4).min(bytes.len())..(i + 2 + len).min(bytes.len())];
        if marker == 0xE1 && seg.starts_with(b"Exif\0\0") {
            if let Some(o) = tiff_orientation(&seg[6..]) {
                return o;
            }
        }
        i += 2 + len;
    }
    1
}

/// Orientation (tag 274) from IFD0 of a TIFF structure.
fn tiff_orientation(t: &[u8]) -> Option<u8> {
    let le = match t.get(..2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |o: usize| -> Option<u16> {
        let b = t.get(o..o + 2)?;
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |o: usize| -> Option<u32> {
        let b = t.get(o..o + 4)?;
        Some(if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd = u32_at(4)? as usize;
    let n = u16_at(ifd)? as usize;
    for k in 0..n.min(512) {
        let e = ifd + 2 + 12 * k;
        if u16_at(e)? == 274 {
            let v = u16_at(e + 8)?;
            return (1..=8).contains(&v).then_some(v as u8);
        }
    }
    None
}

/// Reads the header.
pub fn info(bytes: &[u8]) -> Result<JpegInfo> {
    let h = Handle::new()?;
    header(&h, bytes)
}

fn header(h: &Handle, bytes: &[u8]) -> Result<JpegInfo> {
    // Keep ICC profiles (APP2) for tj3GetICCProfile.
    h.set(tj::TJPARAM_SAVEMARKERS, 2)?;
    if unsafe { tj::tj3DecompressHeader(h.0, bytes.as_ptr(), bytes.len()) } != 0 {
        return Err(h.error("header"));
    }
    Ok(JpegInfo {
        width: h.get(tj::TJPARAM_JPEGWIDTH).max(0) as usize,
        height: h.get(tj::TJPARAM_JPEGHEIGHT).max(0) as usize,
        precision: h.get(tj::TJPARAM_PRECISION).clamp(0, 16) as u8,
        lossless: h.get(tj::TJPARAM_LOSSLESS) == 1,
        colorspace: h.get(tj::TJPARAM_COLORSPACE),
        subsampling: h.get(tj::TJPARAM_SUBSAMP),
    })
}

/// Whether libjpeg-turbo decodes this JPEG here (CMYK and YCCK go to
/// ImageIO, which applies their profile).
pub fn handles(info: &JpegInfo) -> bool {
    [tj::TJCS_RGB, tj::TJCS_YCbCr, tj::TJCS_GRAY].contains(&info.colorspace)
}

pub fn read(path: &Path, pool: &FramePool) -> Result<StillImage> {
    let bytes = std::fs::read(path)?;
    let h = Handle::new()?;
    let info = header(&h, &bytes)?;
    if !handles(&info) {
        return super::imageio::read_frame(path, 0, pool, ImageFormat::Jpeg);
    }
    let (w, ht) = (info.width, info.height);
    if w == 0 || ht == 0 || w > 1 << 16 || ht > 1 << 16 {
        return Err(MediaError::Image(format!("JPEG of {w}×{ht}")));
    }
    // Accurate IDCT and smooth upsampling (TurboJPEG's defaults, set
    // explicitly: they are what the viewer shows).
    h.set(tj::TJPARAM_FASTDCT, 0)?;
    h.set(tj::TJPARAM_FASTUPSAMPLE, 0)?;
    let icc = unsafe {
        let mut buf: *mut u8 = std::ptr::null_mut();
        let mut size = 0usize;
        if tj::tj3GetICCProfile(h.0, &mut buf, &mut size) == 0 && !buf.is_null() && size > 0 {
            let v = std::slice::from_raw_parts(buf, size).to_vec();
            tj::tj3Free(buf.cast());
            Some(v)
        } else {
            None
        }
    };
    let orientation = exif_orientation(&bytes);
    let pitch = (w * 4) as i32;
    let label = format!("libjpeg-turbo {}-bit", info.precision);
    let (frame, bits) = match info.precision {
        8 if !info.lossless => {
            let mut px = vec![0u8; w * ht * 4];
            let r = unsafe {
                tj::tj3Decompress8(
                    h.0,
                    bytes.as_ptr(),
                    bytes.len(),
                    px.as_mut_ptr(),
                    pitch,
                    tj::TJPF_RGBA as i32,
                )
            };
            if r != 0 && unsafe { tj::tj3GetErrorCode(h.0) } == tj::TJERR_FATAL as i32 {
                return Err(h.error("decode"));
            }
            let (px, ow, oh) = orient(&px, w, ht, 4, orientation);
            (pool_rgba(pool, ow, oh, Rgba::U8(&px), false, 8, label)?, 8)
        }
        p => {
            // 12-bit lossy, or lossless at any precision: samples 0 to 2^p − 1.
            let mut px = vec![0u16; w * ht * 4];
            let r = unsafe {
                if p == 12 && !info.lossless {
                    tj::tj3Decompress12(
                        h.0,
                        bytes.as_ptr(),
                        bytes.len(),
                        px.as_mut_ptr().cast(),
                        pitch,
                        tj::TJPF_RGBA as i32,
                    )
                } else {
                    tj::tj3Decompress16(
                        h.0,
                        bytes.as_ptr(),
                        bytes.len(),
                        px.as_mut_ptr(),
                        pitch,
                        tj::TJPF_RGBA as i32,
                    )
                }
            };
            if r != 0 && unsafe { tj::tj3GetErrorCode(h.0) } == tj::TJERR_FATAL as i32 {
                return Err(h.error("decode"));
            }
            // TurboJPEG fills alpha with the maximum of the precision.
            let (px, ow, oh) = orient(&px, w, ht, 4, orientation);
            (pool_rgba(pool, ow, oh, Rgba::U16(&px), false, p, label)?, p)
        }
    };
    let icc_class = icc.as_deref().map(icc::classify);
    Ok(StillImage {
        frame,
        format: ImageFormat::Jpeg,
        backend: ImageBackend::LibjpegTurbo,
        bit_depth: bits,
        float: false,
        color: ImageColor {
            icc_bytes: icc.as_ref().map(Vec::len),
            icc,
            icc_class,
            transfer: Transfer::DisplayE,
            alpha: AlphaMode::Opaque,
            chromaticities: None,
        },
        orientation,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_exif_orientation_from_app1() {
        // SOI, APP1 "Exif\0\0" with a big-endian TIFF holding tag 274 = 6.
        let mut tiff = b"MM\0\x2a\0\0\0\x08".to_vec();
        tiff.extend([0, 1]); // one entry
        tiff.extend([0x01, 0x12, 0, 3, 0, 0, 0, 1, 0, 6, 0, 0]);
        tiff.extend([0, 0, 0, 0]);
        let mut app1 = b"Exif\0\0".to_vec();
        app1.extend(&tiff);
        let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE1];
        jpeg.extend(((app1.len() + 2) as u16).to_be_bytes());
        jpeg.extend(&app1);
        jpeg.extend([0xFF, 0xDA, 0, 2]);
        assert_eq!(exif_orientation(&jpeg), 6);
        assert_eq!(exif_orientation(&[0xFF, 0xD8, 0xFF, 0xDA]), 1);
    }
}
