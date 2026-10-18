//! Chromium 152's AVIF decode: CrabbyAvif + dav1d + libyuv into a
//! premultiplied 8-bit bitmap, with Blink `AVIFImageDecoder`'s container
//! handling: a valid `clap` anchored at (0, 0) crops the decoded planes
//! before conversion (any other `clap` shows the full image), and `irot` /
//! `imir` become the image orientation, applied as the ImageBitmap applies
//! it. Native-depth ImageIO reading is unchanged.
use super::{ImageColor, ImageFormat, Rgba, StillImage, orient, pool_rgba};
use crate::{MediaError, Result, pool::FramePool, sys::avif as a};
use pm_protocol::color::{AlphaMode, Transfer};
use std::path::Path;

/// Whether the decoded facts are in the class this module reproduces
/// (supported ICC or CICP; ImageIO exposes deep AVIF as 16-bit).
/// [`unmeasured`] adds the container facts.
pub fn applies(still: &StillImage) -> bool {
    still.format == ImageFormat::Avif && matches!(still.bit_depth, 8 | 16) && !still.float
}

struct Decoder(*mut a::avifDecoder);
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { a::crabby_avifDecoderDestroy(self.0) };
    }
}

struct View(*mut a::avifImage);
impl Drop for View {
    fn drop(&mut self) {
        unsafe { a::crabby_avifImageDestroy(self.0) };
    }
}

/// A parsed decoder over `bytes`, configured as Blink configures it.
/// `bytes` must outlive the decoder.
unsafe fn parse(bytes: &[u8]) -> Result<Decoder> {
    unsafe {
        let decoder = Decoder(a::crabby_avifDecoderCreate());
        if decoder.0.is_null() {
            return Err(MediaError::decode("AVIF decoder allocation"));
        }
        (*decoder.0).maxThreads = 4;
        (*decoder.0).ignoreXMP = 1;
        (*decoder.0).ignoreExif = 1;
        // Blink validates `clap` itself and ignores invalid ones, and
        // accepts AV1 items without `pixi` (libheif 1.11 and older).
        (*decoder.0).strictFlags &= !(a::AVIF_STRICT_CLAP_VALID | a::AVIF_STRICT_PIXI_REQUIRED);
        let ok = a::crabby_avifDecoderSetIOMemory(decoder.0, bytes.as_ptr(), bytes.len());
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF input {ok}")));
        }
        let ok = a::crabby_avifDecoderParse(decoder.0);
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF parse {ok}")));
        }
        Ok(decoder)
    }
}

/// Blink's orientation for `irot`/`imir` (`kAxisAngleToOrientation`), as
/// an EXIF orientation number.
fn orientation(flags: u32, angle: u8, axis: u8) -> u8 {
    let angle = if flags & a::AVIF_TRANSFORM_IROT != 0 {
        (angle & 3) as usize
    } else {
        0
    };
    let row = if flags & a::AVIF_TRANSFORM_IMIR != 0 {
        1 + (axis & 1) as usize
    } else {
        0
    };
    // Rows: no mirror, top-bottom mirror, left-right mirror; columns:
    // anticlockwise quarter turns. TopLeft 1, TopRight 2, BottomRight 3,
    // BottomLeft 4, LeftTop 5, RightTop 6, RightBottom 7, LeftBottom 8.
    const TABLE: [[u8; 4]; 3] = [[1, 8, 3, 6], [4, 5, 2, 7], [2, 7, 4, 5]];
    TABLE[row][angle]
}

/// The crop Blink applies: a valid `clap` anchored at (0, 0); None shows
/// the full image.
unsafe fn clean_aperture(image: &a::avifImage) -> Option<a::avifCropRect> {
    if image.transformFlags & a::AVIF_TRANSFORM_CLAP == 0 {
        return None;
    }
    let mut rect = a::avifCropRect {
        x: 0,
        y: 0,
        width: 0,
        height: 0,
    };
    let mut diag = a::avifDiagnostics::default();
    let valid = unsafe {
        a::crabby_avifCropRectConvertCleanApertureBox(
            &mut rect,
            &image.clap,
            image.width,
            image.height,
            image.yuvFormat,
            &mut diag,
        )
    };
    (valid != 0 && rect.x == 0 && rect.y == 0).then_some(rect)
}

/// Whether Blink gives an AVIF without an ICC profile the sRGB color
/// profile. `AVIFImageDecoder::GetColorSpace` reads unspecified primaries
/// as BT.709 and an unspecified transfer as sRGB, and
/// `gfx::ColorSpace::GetTransferFunction` gives BT.709, SMPTE 170M (BT.601)
/// and both BT.2020 transfers the sRGB curve, so with BT.709 primaries
/// `SkColorSpace::MakeRGB` returns sRGB itself and no conversion runs.
/// Matrix and range only steer the YUV conversion, which is CrabbyAvif's
/// in both.
pub fn srgb_cicp(primaries: u16, transfer: u16) -> bool {
    matches!(primaries, 1 | 2) && matches!(transfer, 1 | 2 | 6 | 13 | 14 | 15)
}

/// Profile pointers borrow the decoder-owned ICC bytes. Blink rejects an
/// invalid AVIF profile rather than silently substituting the CICP profile.
unsafe fn profile(image: &a::avifImage) -> Result<crate::sys::skcms::skcms_ICCProfile> {
    use crate::sys::skcms as s;
    unsafe {
        let mut source = std::mem::zeroed();
        if image.icc.data.is_null()
            || !s::skcms_ParseWithA2BPriority(
                image.icc.data.cast(),
                image.icc.size,
                [0, 1].as_ptr(),
                2,
                &mut source,
            )
        {
            return Err(MediaError::decode("invalid AVIF ICC profile"));
        }
        let signature = source.data_color_space;
        if signature != s::skcms_Signature_RGB
            && !(image.yuvFormat == a::AVIF_PIXEL_FORMAT_YUV400
                && signature == s::skcms_Signature_Gray)
        {
            return Err(MediaError::decode(
                "AVIF ICC color model does not match its planes",
            ));
        }
        Ok(source)
    }
}

/// Why the container's color description (what Blink's
/// `AVIFImageDecoder` reads after `avifDecoderParse`) is outside the class
/// this module reproduces.
/// With `wide`, the CICP pairs of [`super::compat::cicp_supported`] (BT.2020,
/// P3 and PQ/HLG, measured for Chromium's bitmap); otherwise SDR sRGB only,
/// the class the native (ImageIO) reader was compared in.
fn colorimetry(image: &a::avifImage, wide: bool) -> Option<String> {
    if image.icc.size != 0 {
        // ICC overrides CICP in Blink. Validate the same RGB/gray model
        // here; the ImageBitmap conversion applies it in `read`.
        return unsafe { profile(image).err().map(|e| e.to_string()) };
    }
    let (p, t) = (image.colorPrimaries, image.transferCharacteristics);
    let drawn = srgb_cicp(p, t) || (wide && super::compat::cicp_supported(p, t));
    (!drawn).then(|| format!("AVIF stills with color primaries {p} and transfer {t}"))
}

/// Why an AVIF is outside the class compared with Chromium: SDR 8/10/12-bit
/// 4:4:4, 4:2:2, 4:2:0 or monochrome, full/limited range, alpha, container
/// transforms and ICC, plus the measured 16-bit enhanced 4:4:4 containers.
/// Gain maps keep the SDR base image, as the shipped
/// ImageBitmap-to-WebGL upload does, including HDR base images.
pub fn unmeasured(path: &Path) -> Result<Option<String>> {
    unmeasured_in(path, true)
}

/// [`unmeasured`] for the native-depth (ImageIO) reader of level 2 layers
/// and 3D maps, which has no PQ/HLG or wide-primaries reading: only SDR
/// sRGB color descriptions (or a supported ICC profile) are in its class.
pub fn unmeasured_sdr(path: &Path) -> Result<Option<String>> {
    unmeasured_in(path, false)
}

fn unmeasured_in(path: &Path, wide: bool) -> Result<Option<String>> {
    let bytes = std::fs::read(path)?;
    unsafe {
        let decoder = parse(&bytes)?;
        let image = &*(*decoder.0).image;
        if let Some(what) = colorimetry(image, wide) {
            return Ok(Some(what));
        }
        Ok(match image.yuvFormat {
            a::AVIF_PIXEL_FORMAT_YUV420
            | a::AVIF_PIXEL_FORMAT_YUV444
            | a::AVIF_PIXEL_FORMAT_YUV422
            | a::AVIF_PIXEL_FORMAT_YUV400
                if matches!(image.depth, 8 | 10 | 12)
                    || image.depth == 16 && image.yuvFormat == a::AVIF_PIXEL_FORMAT_YUV444 =>
            {
                None
            }
            _ => Some(format!(
                "{}-bit AVIF stills of pixel format {}",
                image.depth, image.yuvFormat
            )),
        })
    }
}

pub fn read(path: &Path, _original: &StillImage, pool: &FramePool) -> Result<StillImage> {
    read_bitmap(path, pool)
}

/// Decode the compatibility bitmap independently of ImageIO. Some valid AVIFs
/// (including premultiplied alpha) are accepted by Chromium but not ImageIO.
/// Native-depth reading continues to use the ordinary source reader.
pub fn read_bitmap(path: &Path, pool: &FramePool) -> Result<StillImage> {
    let bytes = std::fs::read(path)?;
    // Decoder and its image own all allocations until after the pool copy.
    unsafe {
        let decoder = parse(&bytes)?;
        // Blink reads the color description from the container, before any
        // frame is decoded. Callers gate on `unmeasured`; this is an
        // internal invariant, not a property of the file.
        if let Some(what) = colorimetry(&*(*decoder.0).image, true) {
            return Err(MediaError::decode(format!(
                "AVIF compatibility bitmap outside its class: {what}"
            )));
        }
        let ok = a::crabby_avifDecoderNthImage(decoder.0, 0);
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF first frame {ok}")));
        }
        let decoded = &*(*decoder.0).image;
        let o = orientation(
            decoded.transformFlags,
            decoded.irot.angle,
            decoded.imir.axis,
        );
        let view;
        let image = match clean_aperture(decoded) {
            Some(rect) => {
                view = View(a::crabby_avifImageCreateEmpty());
                if view.0.is_null() {
                    return Err(MediaError::decode("AVIF view allocation"));
                }
                let ok = a::crabby_avifImageSetViewRect(view.0, decoded, &rect);
                if ok != a::AVIF_RESULT_OK {
                    return Err(MediaError::decode(format!("AVIF clean aperture {ok}")));
                }
                &*view.0
            }
            None => decoded,
        };
        let (w, h) = (image.width as usize, image.height as usize);
        let count = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .filter(|n| *n <= 1 << 30)
            .ok_or_else(|| MediaError::invalid("AVIF bitmap exceeds limit"))?;
        let mut bgra = vec![0u8; count];
        let mut rgb: a::avifRGBImage = std::mem::zeroed();
        a::crabby_avifRGBImageSetDefaults(&mut rgb, image);
        rgb.depth = 8;
        rgb.format = a::AVIF_RGB_FORMAT_BGRA;
        rgb.alphaPremultiplied = true;
        rgb.maxThreads = 4;
        rgb.pixels = bgra.as_mut_ptr();
        rgb.rowBytes = image.width * 4;
        let ok = a::crabby_avifImageYUVToRGB(image, &mut rgb);
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF RGB conversion {ok}")));
        }
        for p in bgra.chunks_exact_mut(4) {
            p.swap(0, 2)
        }
        // The ImageBitmap's orientation transform is a pixel permutation of
        // the premultiplied bitmap (measured exact against Chromium's
        // texture, docs/engine/media.md).
        let (rgba, w, h) = orient(&bgra, w, h, 4, o);
        let alpha = if image.alphaPlane.is_null() {
            AlphaMode::Opaque
        } else {
            AlphaMode::Premultiplied
        };
        let bitmap = StillImage {
            frame: pool_rgba(
                pool,
                w,
                h,
                Rgba::U8(&rgba),
                true,
                8,
                "Chromium AVIF rgba8".into(),
            )?,
            color: ImageColor {
                icc: None,
                icc_bytes: None,
                icc_class: None,
                transfer: Transfer::DisplayE,
                alpha,
                chromaticities: None,
            },
            orientation: o,
            bit_depth: 8,
            float: false,
            format: ImageFormat::Avif,
            backend: super::ImageBackend::CrabbyAvif,
        };
        if decoded.icc.size != 0 {
            let bytes = std::slice::from_raw_parts(decoded.icc.data, decoded.icc.size).to_vec();
            let class = super::icc::classify(&bytes);
            // ImageBitmap tags its decoded source-space bitmap. WebGL's
            // readPixels conversion uses Skia's float color pipeline, which
            // can retain out-of-gamut premultiplied RGB above alpha.
            let mut tagged = bitmap;
            tagged.color.icc_bytes = Some(bytes.len());
            tagged.color.icc_class = Some(class);
            tagged.color.icc = Some(bytes);
            super::compat::bitmap(&tagged, pool)
        } else if !srgb_cicp(decoded.colorPrimaries, decoded.transferCharacteristics) {
            super::compat::bitmap_cicp(
                &bitmap,
                pool,
                decoded.colorPrimaries,
                decoded.transferCharacteristics,
            )
        } else {
            Ok(bitmap)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_descriptions_blink_reads_as_srgb() {
        // `GetColorSpace` defaults (2 reads as BT.709 primaries and the sRGB
        // transfer); `gfx::ColorSpace::GetTransferFunction` gives BT.709,
        // SMPTE 170M and both BT.2020 transfers the sRGB curve.
        for p in [1, 2] {
            for t in [1, 2, 6, 13, 14, 15] {
                assert!(srgb_cicp(p, t), "{p}/{t}");
            }
            for t in [0, 4, 5, 7, 8, 9, 10, 11, 12, 16, 17, 18] {
                assert!(!srgb_cicp(p, t), "{p}/{t}");
            }
        }
        for p in [0, 4, 5, 6, 7, 8, 9, 10, 11, 12, 22] {
            assert!(!srgb_cicp(p, 13), "{p}");
        }
    }

    #[test]
    fn blink_orientation_table() {
        let irot = a::AVIF_TRANSFORM_IROT;
        let imir = a::AVIF_TRANSFORM_IMIR;
        assert_eq!(orientation(0, 3, 1), 1, "flags gate the properties");
        assert_eq!(orientation(irot, 1, 0), 8);
        assert_eq!(orientation(irot, 2, 0), 3);
        assert_eq!(orientation(irot, 3, 0), 6);
        assert_eq!(orientation(imir, 0, 0), 4);
        assert_eq!(orientation(imir, 0, 1), 2);
        assert_eq!(orientation(irot | imir, 1, 0), 5);
        assert_eq!(orientation(irot | imir, 3, 1), 5);
        assert_eq!(orientation(irot | imir, 1, 1), 7);
    }
}
