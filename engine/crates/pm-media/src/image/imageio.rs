//! ImageIO: HEIF/HEIC, AVIF, animation frames (GIF, APNG, animated WebP and
//! AVIF, HEIF sequences), and any other format macOS reads.
//!
//! Pixels come from the image's data provider as stored (no color
//! matching) for RGB and gray images at 8, 16, or 32-bit (integer, half, or
//! float). Images in other color models (CMYK, Lab, indexed) are drawn into
//! a 16-bit sRGB context, which color-converts them; such images report an
//! `Other` profile class.

use super::{
    Animation, ImageBackend, ImageColor, ImageFormat, Rgba, StillImage, icc, orient, pool_rgba,
};
use crate::apple::cf::*;
use crate::error::{MediaError, Result};
use crate::pool::FramePool;
use pm_protocol::color::{AlphaMode, Transfer};
use std::ffi::c_void;
use std::path::Path;

type CGImageSourceRef = *mut c_void;
type CGImageRef = *mut c_void;
type CGColorSpaceRef = *mut c_void;
type CGContextRef = *mut c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[link(name = "ImageIO", kind = "framework")]
unsafe extern "C" {
    fn CGImageSourceCreateWithURL(url: CFTypeRef, options: CFDictionaryRef) -> CGImageSourceRef;
    fn CGImageSourceGetCount(src: CGImageSourceRef) -> usize;
    fn CGImageSourceCopyPropertiesAtIndex(
        src: CGImageSourceRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> CFDictionaryRef;
    fn CGImageSourceCopyProperties(
        src: CGImageSourceRef,
        options: CFDictionaryRef,
    ) -> CFDictionaryRef;
    fn CGImageSourceCreateImageAtIndex(
        src: CGImageSourceRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> CGImageRef;
    fn CGImageSourceGetType(src: CGImageSourceRef) -> CFStringRef;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    static kCGColorSpaceSRGB: CFStringRef;
    fn CGImageGetWidth(img: CGImageRef) -> usize;
    fn CGImageGetHeight(img: CGImageRef) -> usize;
    fn CGImageGetBitsPerComponent(img: CGImageRef) -> usize;
    fn CGImageGetBitsPerPixel(img: CGImageRef) -> usize;
    fn CGImageGetBytesPerRow(img: CGImageRef) -> usize;
    fn CGImageGetBitmapInfo(img: CGImageRef) -> u32;
    fn CGImageGetColorSpace(img: CGImageRef) -> CGColorSpaceRef;
    fn CGImageGetDataProvider(img: CGImageRef) -> *mut c_void;
    fn CGDataProviderCopyData(provider: *mut c_void) -> CFTypeRef;
    fn CGColorSpaceGetModel(space: CGColorSpaceRef) -> i32;
    fn CGColorSpaceCopyICCData(space: CGColorSpaceRef) -> CFTypeRef;
    fn CGColorSpaceCreateWithName(name: CFStringRef) -> CGColorSpaceRef;
    fn CGColorSpaceRelease(space: CGColorSpaceRef);
    fn CGBitmapContextCreate(
        data: *mut c_void,
        width: usize,
        height: usize,
        bits_per_component: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
    ) -> CGContextRef;
    fn CGContextDrawImage(ctx: CGContextRef, rect: CGRect, img: CGImageRef);
    fn CGContextRelease(ctx: CGContextRef);
}

const MODEL_MONOCHROME: i32 = 0;
const MODEL_RGB: i32 = 1;
const ALPHA_MASK: u32 = 0x1F;
const FLOAT: u32 = 1 << 8;
const ORDER_MASK: u32 = 0x7000;
const ORDER_16_LITTLE: u32 = 1 << 12;
const ORDER_32_LITTLE: u32 = 2 << 12;
const PREMULTIPLIED_LAST: u32 = 1;

fn source(path: &Path) -> Result<Cf> {
    let url = cf_file_url(path).ok_or_else(|| MediaError::invalid("path"))?;
    let src =
        unsafe { Cf::from_create(CGImageSourceCreateWithURL(url.as_ptr(), std::ptr::null())) };
    src.ok_or_else(|| MediaError::Image(format!("ImageIO cannot open {}", path.display())))
}

/// Frame count and delays; None for a single image.
pub fn animation(path: &Path) -> Result<Option<Animation>> {
    let src = source(path)?;
    let n = unsafe { CGImageSourceGetCount(src.as_mut_ptr()) };
    if n <= 1 {
        return Ok(None);
    }
    let mut delays = Vec::with_capacity(n);
    for i in 0..n {
        let props = unsafe {
            Cf::from_create(CGImageSourceCopyPropertiesAtIndex(
                src.as_mut_ptr(),
                i,
                std::ptr::null(),
            ))
        };
        let delay = props
            .as_ref()
            .and_then(|p| frame_delay(p.as_ptr()))
            .unwrap_or(0.1);
        delays.push(delay);
    }
    let props = unsafe {
        Cf::from_create(CGImageSourceCopyProperties(
            src.as_mut_ptr(),
            std::ptr::null(),
        ))
    };
    let loop_count = props.as_ref().and_then(|p| {
        ["{GIF}", "{PNG}", "{WebP}", "{HEICS}", "{AVIS}"]
            .iter()
            .find_map(|k| unsafe {
                let d = cf_dict_get(p.as_ptr(), k);
                let v = cf_dict_get(d, "LoopCount");
                cf_i64(v).map(|v| v as u32)
            })
    });
    Ok(Some(Animation {
        frames: n,
        delays,
        loop_count,
    }))
}

/// Unclamped delay where the format has one, else the clamped delay.
fn frame_delay(props: CFTypeRef) -> Option<f64> {
    for (dict, keys) in [
        ("{GIF}", ["UnclampedDelayTime", "DelayTime"]),
        ("{PNG}", ["UnclampedDelayTime", "DelayTime"]),
        ("{WebP}", ["UnclampedDelayTime", "DelayTime"]),
        ("{HEICS}", ["UnclampedDelayTime", "DelayTime"]),
        ("{AVIS}", ["UnclampedDelayTime", "DelayTime"]),
    ] {
        let d = unsafe { cf_dict_get(props, dict) };
        if d.is_null() {
            continue;
        }
        for k in keys {
            let v = unsafe { cf_dict_get(d, k) };
            if !v.is_null() {
                let mut f = 0f64;
                if unsafe {
                    CFNumberGetValue(
                        v,
                        13, /* kCFNumberDoubleType */
                        &mut f as *mut f64 as *mut c_void,
                    )
                } != 0
                    && f > 0.0
                {
                    return Some(f);
                }
            }
        }
    }
    None
}

pub fn read_frame(
    path: &Path,
    index: usize,
    pool: &FramePool,
    format: ImageFormat,
) -> Result<StillImage> {
    let src = source(path)?;
    let count = unsafe { CGImageSourceGetCount(src.as_mut_ptr()) };
    if index >= count {
        return Err(MediaError::invalid(format!("frame {index} of {count}")));
    }
    let props = unsafe {
        Cf::from_create(CGImageSourceCopyPropertiesAtIndex(
            src.as_mut_ptr(),
            index,
            std::ptr::null(),
        ))
    };
    let orientation = props
        .as_ref()
        .and_then(|p| unsafe { cf_i64(cf_dict_get(p.as_ptr(), "Orientation")) })
        .filter(|o| (1..=8).contains(o))
        .unwrap_or(1) as u8;
    let img = unsafe {
        Cf::from_create(CGImageSourceCreateImageAtIndex(
            src.as_mut_ptr(),
            index,
            std::ptr::null(),
        ))
    }
    .ok_or_else(|| MediaError::Image(format!("ImageIO could not decode frame {index}")))?;
    let uti =
        unsafe { cf_string_value(CGImageSourceGetType(src.as_mut_ptr())) }.unwrap_or_default();
    let p = img.as_mut_ptr();
    let (w, h) = unsafe { (CGImageGetWidth(p), CGImageGetHeight(p)) };
    let bpc = unsafe { CGImageGetBitsPerComponent(p) };
    let bpp = unsafe { CGImageGetBitsPerPixel(p) };
    let row = unsafe { CGImageGetBytesPerRow(p) };
    let info = unsafe { CGImageGetBitmapInfo(p) };
    let space = unsafe { CGImageGetColorSpace(p) };
    let model = if space.is_null() {
        -1
    } else {
        unsafe { CGColorSpaceGetModel(space) }
    };
    let icc_data = if space.is_null() {
        None
    } else {
        unsafe { Cf::from_create(CGColorSpaceCopyICCData(space)) }
    };
    let icc_bytes = icc_data
        .as_ref()
        .and_then(|d| unsafe { cf_data_bytes(d.as_ptr()) });
    let alpha_info = info & ALPHA_MASK;
    let comps = bpp / bpc.max(1);
    let direct = (model == MODEL_RGB && (comps == 3 || comps == 4))
        || (model == MODEL_MONOCHROME && (comps == 1 || comps == 2));
    let label = format!("ImageIO {uti} {bpc}-bit");
    let (rgba, bits, float, premultiplied, icc_class, icc) = if direct && matches!(bpc, 8 | 16 | 32)
    {
        let data = unsafe { Cf::from_create(CGDataProviderCopyData(CGImageGetDataProvider(p))) }
            .ok_or_else(|| MediaError::Image("no pixel data".into()))?;
        let bytes = unsafe { cf_data_bytes(data.as_ptr()) }
            .ok_or_else(|| MediaError::Image("no pixel data".into()))?;
        let unpacked = unpack(&bytes, w, h, row, bpc, comps, info)?;
        let premul = matches!(alpha_info, 1 | 2);
        let class = icc_bytes.as_deref().map(icc::classify);
        (
            unpacked,
            if bpc == 32 { 32 } else { bpc as u8 },
            info & FLOAT != 0,
            premul,
            class,
            icc_bytes,
        )
    } else {
        // Other color models: draw into 16-bit premultiplied sRGB.
        let stride = w * 8;
        let mut buf = vec![0u16; w * h * 4];
        unsafe {
            let srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
            let ctx = CGBitmapContextCreate(
                buf.as_mut_ptr() as *mut c_void,
                w,
                h,
                16,
                stride,
                srgb,
                PREMULTIPLIED_LAST | ORDER_16_LITTLE,
            );
            CGColorSpaceRelease(srgb);
            if ctx.is_null() {
                return Err(MediaError::Image(
                    "could not create a bitmap context".into(),
                ));
            }
            CGContextDrawImage(
                ctx,
                CGRect {
                    x: 0.0,
                    y: 0.0,
                    w: w as f64,
                    h: h as f64,
                },
                p,
            );
            CGContextRelease(ctx);
        }
        (
            Unpacked::U16(buf),
            16,
            false,
            true,
            Some(icc::IccClass::Other),
            icc_bytes,
        )
    };
    let frame = match rgba {
        Unpacked::U8(v) => {
            let (v, ow, oh) = orient(&v, w, h, 4, orientation);
            pool_rgba(pool, ow, oh, Rgba::U8(&v), premultiplied, 8, label)?
        }
        Unpacked::U16(v) => {
            let (v, ow, oh) = orient(&v, w, h, 4, orientation);
            pool_rgba(
                pool,
                ow,
                oh,
                if float { Rgba::F16(&v) } else { Rgba::U16(&v) },
                premultiplied,
                16,
                label,
            )?
        }
        Unpacked::F32(v) => {
            let (v, ow, oh) = orient(&v, w, h, 4, orientation);
            pool_rgba(pool, ow, oh, Rgba::F32(&v), premultiplied, 32, label)?
        }
    };
    let has_alpha = !matches!(alpha_info, 0 | 5 | 6);
    Ok(StillImage {
        frame,
        format,
        backend: ImageBackend::ImageIo,
        bit_depth: bits,
        float,
        color: ImageColor {
            icc_bytes: icc.as_ref().map(|d| d.len()),
            icc,
            icc_class,
            transfer: if float {
                Transfer::Linear
            } else {
                Transfer::DisplayE
            },
            alpha: match (has_alpha, premultiplied) {
                (false, _) => AlphaMode::Opaque,
                (true, true) => AlphaMode::Premultiplied,
                (true, false) => AlphaMode::Straight,
            },
            chromaticities: None,
        },
        orientation,
    })
}

enum Unpacked {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

/// Raw provider bytes → packed RGBA in R, G, B, A order.
fn unpack(
    b: &[u8],
    w: usize,
    h: usize,
    row: usize,
    bpc: usize,
    comps: usize,
    info: u32,
) -> Result<Unpacked> {
    let alpha = info & ALPHA_MASK;
    // Where alpha (or padding) sits: first (ARGB/XRGB) or last (RGBA/RGBX).
    let alpha_first = matches!(alpha, 2 | 4 | 6);
    let has_slot = comps == 4 || comps == 2;
    let little = matches!(info & ORDER_MASK, ORDER_16_LITTLE | ORDER_32_LITTLE);
    let order32_little = info & ORDER_MASK == ORDER_32_LITTLE;
    let opaque = matches!(alpha, 0 | 5 | 6);
    if b.len() < row * (h.saturating_sub(1)) + w * comps * bpc / 8 {
        return Err(MediaError::Image("short pixel data".into()));
    }
    let read = |x: usize, y: usize, c: usize| -> f64 {
        let o = y * row + (x * comps + c) * bpc / 8;
        match bpc {
            8 => {
                // 32-bit little-endian byte order reverses the four bytes of a pixel (BGRA).
                let c = if order32_little && comps == 4 {
                    3 - c
                } else {
                    c
                };
                b[y * row + x * comps + c] as f64
            }
            16 => {
                let v = [b[o], b[o + 1]];
                (if little {
                    u16::from_le_bytes(v)
                } else {
                    u16::from_be_bytes(v)
                }) as f64
            }
            _ => {
                let v = [b[o], b[o + 1], b[o + 2], b[o + 3]];
                (if little {
                    f32::from_le_bytes(v)
                } else {
                    f32::from_be_bytes(v)
                }) as f64
            }
        }
    };
    // Opaque alpha: the maximum code, or 1.0 (as a half float's bit pattern
    // for 16-bit float images, whose samples are kept as bit patterns).
    let max = match (bpc, info & FLOAT != 0) {
        (8, _) => 255.0,
        (16, false) => 65535.0,
        (16, true) => 15360.0,
        _ => 1.0,
    };
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (color_off, alpha_c) = match (has_slot, alpha_first) {
                (true, true) => (1, Some(0)),
                (true, false) => (0, Some(comps - 1)),
                _ => (0, None),
            };
            let color: [f64; 3] = if comps <= 2 {
                let g = read(x, y, color_off);
                [g, g, g]
            } else {
                [
                    read(x, y, color_off),
                    read(x, y, color_off + 1),
                    read(x, y, color_off + 2),
                ]
            };
            let a = match alpha_c {
                Some(c) if !opaque => read(x, y, c),
                _ => max,
            };
            out.extend_from_slice(&[color[0], color[1], color[2], a]);
        }
    }
    Ok(match (bpc, info & FLOAT != 0) {
        (8, _) => Unpacked::U8(out.into_iter().map(|v| v as u8).collect()),
        // Half floats keep their bit patterns.
        (16, _) => Unpacked::U16(out.into_iter().map(|v| v as u16).collect()),
        _ => Unpacked::F32(out.into_iter().map(|v| v as f32).collect()),
    })
}
