//! Chromium 152's SDR AVIF decode: CrabbyAvif + dav1d + libyuv into a
//! premultiplied 8-bit bitmap, with Blink `AVIFImageDecoder`'s container
//! handling: a valid `clap` anchored at (0, 0) crops the decoded planes
//! before conversion (any other `clap` shows the full image), and `irot` /
//! `imir` become the image orientation, applied as the ImageBitmap applies
//! it. Native-depth ImageIO reading is unchanged.
use super::{ImageColor, ImageFormat, Rgba, StillImage, icc::IccClass, orient, pool_rgba};
use crate::{MediaError, Result, pool::FramePool, sys::avif as a};
use pm_protocol::color::{AlphaMode, Transfer};
use std::path::Path;

/// Whether the decoded facts are in the class this module reproduces
/// (8-bit, sRGB or untagged). [`structure`] adds the container facts.
pub fn applies(still: &StillImage) -> bool {
    still.format == ImageFormat::Avif
        && still.bit_depth == 8
        && matches!(still.color.icc_class, None | Some(IccClass::Srgb))
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
        // Blink validates `clap` itself and ignores invalid ones.
        (*decoder.0).strictFlags &= !a::AVIF_STRICT_CLAP_VALID;
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

/// Why an AVIF's structure is outside the class compared with Chromium
/// (8-bit 4:2:0, full or limited range, opaque or with alpha, any
/// `irot`/`imir`/`clap`; `tests/stills.rs` and
/// `e2e/native-still-orientation-parity.spec.ts`). None inside it.
pub fn unmeasured(path: &Path) -> Result<Option<String>> {
    let bytes = std::fs::read(path)?;
    unsafe {
        let decoder = parse(&bytes)?;
        let image = &*(*decoder.0).image;
        Ok(match image.yuvFormat {
            a::AVIF_PIXEL_FORMAT_YUV420 if image.depth == 8 => None,
            a::AVIF_PIXEL_FORMAT_YUV420 => Some(format!("{}-bit AVIF stills", image.depth)),
            a::AVIF_PIXEL_FORMAT_YUV444 => Some("AVIF stills with 4:4:4 chroma".into()),
            a::AVIF_PIXEL_FORMAT_YUV422 => Some("AVIF stills with 4:2:2 chroma".into()),
            a::AVIF_PIXEL_FORMAT_YUV400 => Some("monochrome AVIF stills".into()),
            other => Some(format!("AVIF stills of pixel format {other}")),
        })
    }
}

pub fn read(path: &Path, original: &StillImage, pool: &FramePool) -> Result<StillImage> {
    let bytes = std::fs::read(path)?;
    // Decoder and its image own all allocations until after the pool copy.
    unsafe {
        let decoder = parse(&bytes)?;
        let ok = a::crabby_avifDecoderNthImage(decoder.0, 0);
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF first frame {ok}")));
        }
        let decoded = &*(*decoder.0).image;
        if !matches!(decoded.colorPrimaries, 1 | 2)
            || !matches!(decoded.transferCharacteristics, 2 | 13)
        {
            return Err(MediaError::invalid(
                "AVIF compatibility bitmap needs sRGB colorimetry",
            ));
        }
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
        Ok(StillImage {
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
            ..original.clone()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
