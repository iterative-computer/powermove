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
use pm_protocol::color::{AlphaMode, Transfer, may_unpremultiply};
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
    fn CGImageSourceCreateThumbnailAtIndex(
        src: CGImageSourceRef,
        index: usize,
        options: CFDictionaryRef,
    ) -> CGImageRef;
    static kCGImageSourceCreateThumbnailFromImageAlways: CFStringRef;
    static kCGImageSourceThumbnailMaxPixelSize: CFStringRef;
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
    static kCGColorSpaceExtendedLinearSRGB: CFStringRef;
    fn CGColorSpaceCreateWithICCData(data: CFTypeRef) -> CGColorSpaceRef;
    fn CGColorSpaceGetNumberOfComponents(space: CGColorSpaceRef) -> usize;
    fn CGDataProviderCreateWithCFData(data: CFTypeRef) -> *mut c_void;
    fn CGDataProviderRelease(provider: *mut c_void);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: CGColorSpaceRef,
        bitmap_info: u32,
        provider: *mut c_void,
        decode: *const f64,
        should_interpolate: bool,
        intent: i32,
    ) -> CGImageRef;
    fn CGImageRelease(img: CGImageRef);
    fn CGImageGetRenderingIntent(img: CGImageRef) -> i32;
    fn CGContextSetRenderingIntent(ctx: CGContextRef, intent: i32);
    fn CGContextSetInterpolationQuality(ctx: CGContextRef, quality: i32);
}

/// `kCGRenderingIntentRelativeColorimetric`.
const RELATIVE_COLORIMETRIC: i32 = 2;
/// `kCGInterpolationNone`.
const INTERPOLATION_NONE: i32 = 1;

/// Converts a still tagged with an ICC profile that is neither equivalent to
/// sRGB nor Display P3 (Adobe RGB, ProPhoto, camera and printer RGB
/// profiles, gray gamma 2.2, ...) through ColorSync, relative colorimetric,
/// into extended linear sRGB in float: values outside 0..1 are kept (no
/// clip until output, section 9).
///
/// `alpha` is how the stored samples are read: the file's alpha mode, or an
/// Interpret Footage override replacing it. The profile transforms colors,
/// never alpha, so the color is taken out of its alpha reading first, in the
/// stored encoding as the source pass does (a matte removed, then
/// unpremultiplied above
/// [`UNPREMULTIPLY_ALPHA_FLOOR`](pm_protocol::color::UNPREMULTIPLY_ALPHA_FLOOR); pixels below it are
/// converted as stored, the way the source pass keeps them), converted as
/// opaque color, and premultiplied again in linear light. The result is
/// premultiplied linear light with Rec.709 primaries and no profile;
/// `Opaque` reads every alpha as 1.
pub fn convert_to_linear_srgb(
    still: &StillImage,
    pool: &FramePool,
    alpha: AlphaMode,
) -> Result<StillImage> {
    let Some(profile) = still.color.icc.as_ref() else {
        return Err(MediaError::invalid("the still has no ICC profile"));
    };
    let f = &still.frame;
    if f.format.layout != crate::frame::Layout::Rgba || f.format.is_float() {
        return Err(MediaError::unsupported(
            "ColorSync conversion of float or planar stills",
        ));
    }
    let (w, h) = (f.format.width, f.format.height);
    let sixteen = f.format.planes[0].format == crate::frame::PlaneFormat::Rgba16;
    let max = if sixteen {
        ((1u32 << f.format.bits.clamp(1, 16)) - 1) as f32
    } else {
        255.0
    };
    // Stored samples as 0..1 RGBA.
    let stored: Vec<f32> = f.with_planes(|pl| {
        let p = &pl[0];
        let mut out = Vec::with_capacity(w * h * 4);
        for y in 0..h {
            let row = unsafe {
                std::slice::from_raw_parts(p.ptr.add(y * p.stride), w * if sixteen { 8 } else { 4 })
            };
            if sixteen {
                out.extend(
                    row.chunks_exact(2)
                        .map(|c| f32::from(u16::from_le_bytes([c[0], c[1]])) / max),
                );
            } else {
                out.extend(row.iter().map(|&v| f32::from(v) / max));
            }
        }
        out
    })?;
    // The color to convert (straight, stored encoding), the alpha to keep,
    // and whether the pixel is premultiplied again after conversion.
    let mut color = Vec::with_capacity(w * h * 4);
    let mut alphas = Vec::with_capacity(w * h);
    let mut kept = Vec::with_capacity(w * h);
    for px in stored.chunks_exact(4) {
        let a = px[3];
        let (rgb, a, premultiply) = match alpha {
            AlphaMode::Opaque => ([px[0], px[1], px[2]], 1.0, false),
            AlphaMode::Straight => ([px[0], px[1], px[2]], a, true),
            AlphaMode::Premultiplied | AlphaMode::PremultipliedWithColor { .. } => {
                let matte = match alpha {
                    AlphaMode::PremultipliedWithColor { matte } => matte.map(|m| m as f32),
                    _ => [0.0; 3],
                };
                let removed = [0, 1, 2].map(|c| px[c] - (1.0 - a) * matte[c]);
                if may_unpremultiply(f64::from(a)) {
                    (removed.map(|v| v / a), a, true)
                } else {
                    (removed, a, false)
                }
            }
        };
        color.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 1.0]);
        alphas.push(a);
        kept.push(!premultiply);
    }
    unsafe {
        let icc = Cf::from_create(CFDataCreate(
            std::ptr::null(),
            profile.as_ptr(),
            profile.len() as isize,
        ))
        .ok_or_else(|| MediaError::Image("ICC data".into()))?;
        let space = CGColorSpaceCreateWithICCData(icc.as_ptr());
        if space.is_null() {
            return Err(MediaError::Image(
                "ColorSync cannot use this ICC profile".into(),
            ));
        }
        let comps = CGColorSpaceGetNumberOfComponents(space);
        // Gray profiles take gray samples; RGB profiles RGB and a skipped
        // component. Float samples, so unpremultiplied colors keep their
        // precision.
        let (data, spp, alpha_info): (Vec<f32>, usize, u32) = match comps {
            1 => (color.chunks_exact(4).map(|p| p[0]).collect(), 1, 0),
            3 => (color, 4, NONE_SKIP_LAST),
            n => {
                CGColorSpaceRelease(space);
                return Err(MediaError::unsupported(format!(
                    "an ICC profile with {n} components on an RGB still"
                )));
            }
        };
        let bytes = Cf::from_create(CFDataCreate(
            std::ptr::null(),
            data.as_ptr().cast(),
            (data.len() * 4) as isize,
        ))
        .ok_or_else(|| MediaError::Image("pixel data".into()))?;
        let provider = CGDataProviderCreateWithCFData(bytes.as_ptr());
        let img = CGImageCreate(
            w,
            h,
            32,
            32 * spp,
            w * spp * 4,
            space,
            alpha_info | FLOAT | ORDER_32_LITTLE,
            provider,
            std::ptr::null(),
            false,
            RELATIVE_COLORIMETRIC,
        );
        CGDataProviderRelease(provider);
        CGColorSpaceRelease(space);
        if img.is_null() {
            return Err(MediaError::Image(
                "could not describe the still to ColorSync".into(),
            ));
        }
        let mut out = vec![0f32; w * h * 4];
        let dst = CGColorSpaceCreateWithName(kCGColorSpaceExtendedLinearSRGB);
        let ctx = CGBitmapContextCreate(
            out.as_mut_ptr().cast(),
            w,
            h,
            32,
            w * 16,
            dst,
            PREMULTIPLIED_LAST | FLOAT | ORDER_32_LITTLE,
        );
        CGColorSpaceRelease(dst);
        if ctx.is_null() {
            CGImageRelease(img);
            return Err(MediaError::Image("could not create a float context".into()));
        }
        CGContextSetRenderingIntent(ctx, RELATIVE_COLORIMETRIC);
        CGContextSetInterpolationQuality(ctx, INTERPOLATION_NONE);
        CGContextDrawImage(
            ctx,
            CGRect {
                x: 0.0,
                y: 0.0,
                w: w as f64,
                h: h as f64,
            },
            img,
        );
        CGContextRelease(ctx);
        CGImageRelease(img);
        // The source was opaque: `out` holds straight linear color.
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let a = alphas[i];
            if !kept[i] {
                for v in &mut px[..3] {
                    *v *= a;
                }
            }
            px[3] = a;
        }
        let opaque = matches!(alpha, AlphaMode::Opaque);
        let frame = pool_rgba(
            pool,
            w,
            h,
            Rgba::F32(&out),
            !opaque,
            32,
            format!("ColorSync extended linear sRGB from {}", f.format.label),
        )?;
        Ok(StillImage {
            frame,
            format: still.format,
            backend: still.backend,
            bit_depth: 32,
            float: true,
            color: ImageColor {
                icc: None,
                icc_bytes: None,
                icc_class: None,
                transfer: Transfer::Linear,
                alpha: if opaque {
                    AlphaMode::Opaque
                } else {
                    AlphaMode::Premultiplied
                },
                chromaticities: None,
            },
            orientation: still.orientation,
        })
    }
}

const MODEL_MONOCHROME: i32 = 0;
const MODEL_RGB: i32 = 1;
const ALPHA_MASK: u32 = 0x1F;
const FLOAT: u32 = 1 << 8;
const ORDER_MASK: u32 = 0x7000;
const ORDER_16_LITTLE: u32 = 1 << 12;
const ORDER_32_LITTLE: u32 = 2 << 12;
const PREMULTIPLIED_LAST: u32 = 1;
const ALPHA_LAST: u32 = 3;
const ALPHA_FIRST: u32 = 4;
/// `kCGImageAlphaNoneSkipLast`.
const NONE_SKIP_LAST: u32 = 5;

fn source(path: &Path) -> Result<Cf> {
    let url = cf_file_url(path).ok_or_else(|| MediaError::invalid("path"))?;
    let src =
        unsafe { Cf::from_create(CGImageSourceCreateWithURL(url.as_ptr(), std::ptr::null())) };
    src.ok_or_else(|| MediaError::Image(format!("ImageIO cannot open {}", path.display())))
}

/// Header dimensions without decoding the image pixels.
pub fn dimensions(path: &Path) -> Result<(u32, u32)> {
    let src = source(path)?;
    let props = unsafe {
        Cf::from_create(CGImageSourceCopyPropertiesAtIndex(
            src.as_mut_ptr(),
            0,
            std::ptr::null(),
        ))
    }
    .ok_or_else(|| MediaError::Image("ImageIO has no image properties".into()))?;
    let width = unsafe { cf_i64(cf_dict_get(props.as_ptr(), "PixelWidth")) }
        .ok_or_else(|| MediaError::Image("ImageIO has no width".into()))?;
    let height = unsafe { cf_i64(cf_dict_get(props.as_ptr(), "PixelHeight")) }
        .ok_or_else(|| MediaError::Image("ImageIO has no height".into()))?;
    if width <= 0 || height <= 0 || width > u32::MAX as i64 || height > u32::MAX as i64 {
        return Err(MediaError::invalid("invalid ImageIO dimensions"));
    }
    Ok((width as u32, height as u32))
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
    read_frame_impl(path, index, pool, format, None)
}

/// ImageIO decodes a thumbnail at or below `max_dimension` without a
/// generated file. The original reader remains the only export path.
pub fn read_reduced(
    path: &Path,
    index: usize,
    pool: &FramePool,
    format: ImageFormat,
    max_dimension: u32,
) -> Result<StillImage> {
    if max_dimension == 0 || max_dimension > i32::MAX as u32 {
        return Err(MediaError::invalid("invalid ImageIO thumbnail dimension"));
    }
    read_frame_impl(path, index, pool, format, Some(max_dimension))
}

fn read_frame_impl(
    path: &Path,
    index: usize,
    pool: &FramePool,
    format: ImageFormat,
    max_dimension: Option<u32>,
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
    let max = max_dimension.map(|n| cf_i32(n as i32));
    let options = max.as_ref().map(|n| unsafe {
        cf_dict(&[
            (kCGImageSourceCreateThumbnailFromImageAlways, cf_bool(true)),
            (kCGImageSourceThumbnailMaxPixelSize, n.as_ptr()),
        ])
    });
    let img = unsafe {
        Cf::from_create(match options.as_ref() {
            Some(o) => CGImageSourceCreateThumbnailAtIndex(src.as_mut_ptr(), index, o.as_ptr()),
            None => CGImageSourceCreateImageAtIndex(src.as_mut_ptr(), index, std::ptr::null()),
        })
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
        // Other color models (CMYK, Lab, indexed) and packings: ColorSync
        // draws them through the file's profile into 16-bit sRGB, so the
        // samples are sRGB from here on and the profile is kept for the
        // facts only (`IccClass::ConvertedToSrgb`).
        //
        // A bitmap context only holds premultiplied alpha, and ColorSync
        // converts a straight-alpha image premultiplied, so the color under
        // partial alpha would not be the file's color converted. Straight
        // alpha is therefore split off: the color converts as an opaque
        // image and the file's alpha is attached unchanged, keeping the
        // file's (straight) reading for alpha overrides.
        let straight = matches!(alpha_info, ALPHA_LAST | ALPHA_FIRST);
        let split = if straight {
            unsafe { split_straight_alpha(p, space, w, h, bpc, comps, row, info) }
        } else {
            None
        };
        let (buf, straight) = match split {
            Some((opaque, alpha)) => {
                let mut buf = unsafe { draw_srgb16(opaque.as_mut_ptr(), w, h)? };
                for (px, a) in buf.chunks_exact_mut(4).zip(alpha) {
                    px[3] = a;
                }
                (buf, true)
            }
            None => {
                let mut buf = unsafe { draw_srgb16(p, w, h)? };
                // A packing the split does not read: divide the drawn color
                // back out (approximate under partial alpha).
                if straight {
                    unpremultiply_u16(&mut buf);
                }
                (buf, straight)
            }
        };
        (
            Unpacked::U16(buf),
            16,
            false,
            !straight,
            Some(icc::IccClass::ConvertedToSrgb),
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

/// Draws `img` into 16-bit premultiplied sRGB (little-endian RGBA)
/// through ColorSync.
unsafe fn draw_srgb16(img: CGImageRef, w: usize, h: usize) -> Result<Vec<u16>> {
    let mut buf = vec![0u16; w * h * 4];
    unsafe {
        let srgb = CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        let ctx = CGBitmapContextCreate(
            buf.as_mut_ptr() as *mut c_void,
            w,
            h,
            16,
            w * 8,
            srgb,
            PREMULTIPLIED_LAST | ORDER_16_LITTLE,
        );
        CGColorSpaceRelease(srgb);
        if ctx.is_null() {
            return Err(MediaError::Image(
                "could not create a bitmap context".into(),
            ));
        }
        CGContextSetInterpolationQuality(ctx, INTERPOLATION_NONE);
        CGContextDrawImage(
            ctx,
            CGRect {
                x: 0.0,
                y: 0.0,
                w: w as f64,
                h: h as f64,
            },
            img,
        );
        CGContextRelease(ctx);
    }
    Ok(buf)
}

/// An owned `CGImage`, released on drop.
struct OwnedImage(CGImageRef);

impl OwnedImage {
    fn as_mut_ptr(&self) -> CGImageRef {
        self.0
    }
}

impl Drop for OwnedImage {
    fn drop(&mut self) {
        unsafe { CGImageRelease(self.0) }
    }
}

/// Splits an integer image with straight alpha (8 or 16 bits, alpha first
/// or last, big-endian or 16-bit little-endian components) into an opaque
/// image of its color in the same color space and its alpha as 16-bit
/// codes. None for packings it does not read.
#[allow(clippy::too_many_arguments)]
unsafe fn split_straight_alpha(
    img: CGImageRef,
    space: CGColorSpaceRef,
    w: usize,
    h: usize,
    bpc: usize,
    comps: usize,
    row: usize,
    info: u32,
) -> Option<(OwnedImage, Vec<u16>)> {
    const ORDER_16_BIG: u32 = 3 << 12;
    const ORDER_32_BIG: u32 = 4 << 12;
    if space.is_null() || info & FLOAT != 0 {
        return None;
    }
    let n = unsafe { CGColorSpaceGetNumberOfComponents(space) };
    let order = info & ORDER_MASK;
    let little = match (bpc, order) {
        (8, 0) | (8, ORDER_32_BIG) | (16, 0) | (16, ORDER_16_BIG) => false,
        (16, ORDER_16_LITTLE) => true,
        _ => return None,
    };
    if comps != n + 1 {
        return None;
    }
    let first = info & ALPHA_MASK == ALPHA_FIRST;
    let bytes_per = bpc / 8;
    let data = unsafe { Cf::from_create(CGDataProviderCopyData(CGImageGetDataProvider(img))) }?;
    let bytes = unsafe { cf_data_bytes(data.as_ptr()) }?;
    if bytes.len() < row * (h - 1) + w * comps * bytes_per {
        return None;
    }
    let mut color = Vec::with_capacity(w * h * n * bytes_per);
    let mut alpha = Vec::with_capacity(w * h);
    for y in 0..h {
        for px in bytes[y * row..y * row + w * comps * bytes_per].chunks_exact(comps * bytes_per) {
            let (a, c) = if first {
                px.split_at(bytes_per)
            } else {
                let (c, a) = px.split_at(n * bytes_per);
                (a, c)
            };
            color.extend_from_slice(c);
            alpha.push(match (bytes_per, little) {
                (1, _) => u16::from(a[0]) * 257,
                (_, false) => u16::from_be_bytes([a[0], a[1]]),
                (_, true) => u16::from_le_bytes([a[0], a[1]]),
            });
        }
    }
    let cf = unsafe {
        Cf::from_create(CFDataCreate(
            std::ptr::null(),
            color.as_ptr(),
            color.len() as isize,
        ))
    }?;
    unsafe {
        let provider = CGDataProviderCreateWithCFData(cf.as_ptr());
        let opaque = CGImageCreate(
            w,
            h,
            bpc,
            bpc * n,
            w * n * bytes_per,
            space,
            order,
            provider,
            std::ptr::null(),
            false,
            // The file's intent, as drawing the image itself would use.
            CGImageGetRenderingIntent(img),
        );
        CGDataProviderRelease(provider);
        (!opaque.is_null()).then(|| (OwnedImage(opaque), alpha))
    }
}

/// Premultiplied 16-bit RGBA to straight, rounded; fully transparent
/// pixels keep color 0.
fn unpremultiply_u16(px: &mut [u16]) {
    for p in px.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        if a == 0 || a == 65535 {
            continue;
        }
        for c in &mut p[..3] {
            *c = ((u32::from(*c) * 65535 + a / 2) / a).min(65535) as u16;
        }
    }
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
