//! Chromium 152's SDR AVIF decode: CrabbyAvif + dav1d + libyuv into a
//! premultiplied 8-bit bitmap. Native-depth ImageIO reading is unchanged.
use super::{ImageColor, ImageFormat, Rgba, StillImage, icc::IccClass, orient, pool_rgba};
use crate::{MediaError, Result, pool::FramePool, sys::avif as a};
use pm_protocol::color::{AlphaMode, Transfer};
use std::path::Path;

pub fn applies(still: &StillImage) -> bool {
    still.format == ImageFormat::Avif
        && still.bit_depth == 8
        && still.orientation == 1
        && matches!(still.color.icc_class, None | Some(IccClass::Srgb))
}

struct Decoder(*mut a::avifDecoder);
impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe { a::crabby_avifDecoderDestroy(self.0) };
    }
}

pub fn read(path: &Path, original: &StillImage, pool: &FramePool) -> Result<StillImage> {
    let bytes = std::fs::read(path)?;
    // Decoder and its image own all allocations until after the pool copy.
    unsafe {
        let decoder = Decoder(a::crabby_avifDecoderCreate());
        if decoder.0.is_null() {
            return Err(MediaError::decode("AVIF decoder allocation"));
        }
        (*decoder.0).maxThreads = 4;
        (*decoder.0).ignoreXMP = 1;
        let ok = a::crabby_avifDecoderSetIOMemory(decoder.0, bytes.as_ptr(), bytes.len());
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF input {ok}")));
        }
        let ok = a::crabby_avifDecoderParse(decoder.0);
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF parse {ok}")));
        }
        let ok = a::crabby_avifDecoderNthImage(decoder.0, 0);
        if ok != a::AVIF_RESULT_OK {
            return Err(MediaError::decode(format!("AVIF first frame {ok}")));
        }
        let image = &*(*decoder.0).image;
        if !matches!(image.colorPrimaries, 1 | 2)
            || !matches!(image.transferCharacteristics, 2 | 13)
        {
            return Err(MediaError::invalid(
                "AVIF compatibility bitmap needs sRGB colorimetry",
            ));
        }
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
        let (rgba, w, h) = orient(&bgra, w, h, 4, original.orientation);
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
            ..original.clone()
        })
    }
}
