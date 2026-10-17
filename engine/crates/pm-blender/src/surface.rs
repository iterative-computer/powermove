//! Blender surfaces (plan sections 9 and 17, contract 11 invariants 1, 4,
//! 5; amendment 11).
//!
//! A surface is one 3D layer's pixels for one frame: a 16-bit RGBA PNG whose
//! color codes are AgX through the piecewise sRGB curve, which is the
//! display encoding E, with straight alpha. The engine decodes it exactly
//! (`code / 65535`; 8-bit surfaces, today's worker output, as `code / 255`),
//! never applies the PNG `gAMA`, `sRGB`, `iCCP`, or `cHRM` chunks, and
//! premultiplies in E at f32 — section 9's rule for straight sources in
//! Standard color, where the input transform is the identity.
//!
//! Like today's `cropSurface`, only the occupied rectangle is kept: the
//! bounding box of pixels with alpha above 0, grown by 2 pixels and clamped;
//! a surface with nothing in it keeps one transparent pixel at the origin.
//! Pixels outside the rectangle are transparent by construction.

use std::sync::Arc;

use pm_protocol::common::{PixelRect, Size};

/// A decoded surface, ready to upload.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedSurface {
    /// The full render size.
    pub size: Size,
    /// The occupied rectangle inside `size`.
    pub crop: PixelRect,
    /// Premultiplied, E-encoded RGBA of the crop rectangle, top row first.
    pub pixels: Arc<[f32]>,
    /// 8 or 16.
    pub bit_depth: u8,
}

impl DecodedSurface {
    /// Bytes this surface holds in memory.
    pub fn bytes(&self) -> u64 {
        (self.pixels.len() * std::mem::size_of::<f32>()) as u64
    }

    /// The crop rectangle as fractions of the full size (today's `rect`:
    /// x, y, width, height), the quad the compositor draws it into.
    pub fn rect(&self) -> [f64; 4] {
        let (w, h) = (f64::from(self.size.width), f64::from(self.size.height));
        [
            f64::from(self.crop.x) / w,
            f64::from(self.crop.y) / h,
            f64::from(self.crop.width) / w,
            f64::from(self.crop.height) / h,
        ]
    }

    /// The premultiplied pixel at full-surface coordinates (transparent
    /// outside the crop).
    pub fn pixel(&self, x: u32, y: u32) -> [f32; 4] {
        let c = self.crop;
        if x < c.x || y < c.y || x >= c.x + c.width || y >= c.y + c.height {
            return [0.0; 4];
        }
        let i = (((y - c.y) * c.width + (x - c.x)) * 4) as usize;
        [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SurfaceError {
    #[error("the Blender surface is not a PNG: {0}")]
    Decode(String),
    #[error("the Blender surface is {0}, not RGBA")]
    ColorType(String),
    #[error("the Blender surface has {0}-bit samples, not 8 or 16")]
    Depth(u8),
    #[error("the Blender surface is {got_w}×{got_h}, the frame needs {want_w}×{want_h}")]
    Size {
        got_w: u32,
        got_h: u32,
        want_w: u32,
        want_h: u32,
    },
}

/// Decodes a surface PNG (8- or 16-bit RGBA) without any color transform.
pub fn decode_surface(bytes: &[u8]) -> Result<DecodedSurface, SurfaceError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // No gamma, no sRGB or ICC handling, no expansion: the codes as written.
    decoder.set_transformations(png::Transformations::IDENTITY);
    decoder.ignore_checksums(false);
    let mut reader = decoder
        .read_info()
        .map_err(|e| SurfaceError::Decode(e.to_string()))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| SurfaceError::Decode("the surface is too large".into()))?;
    let mut buf = vec![0u8; size];
    let info = reader
        .next_frame(&mut buf)
        .map_err(|e| SurfaceError::Decode(e.to_string()))?;
    if info.color_type != png::ColorType::Rgba {
        return Err(SurfaceError::ColorType(format!("{:?}", info.color_type)));
    }
    let (w, h) = (info.width, info.height);
    let n = w as usize * h as usize;
    let depth = match info.bit_depth {
        png::BitDepth::Eight => 8u8,
        png::BitDepth::Sixteen => 16u8,
        other => return Err(SurfaceError::Depth(other as u8)),
    };
    let code = |i: usize| -> u16 {
        if depth == 16 {
            u16::from_be_bytes([buf[i * 2], buf[i * 2 + 1]])
        } else {
            u16::from(buf[i])
        }
    };
    let max = if depth == 16 { 65535.0f32 } else { 255.0f32 };
    // Today's crop: alpha above 0, grown by 2, clamped.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0u32, 0u32);
    let mut any = false;
    for y in 0..h {
        for x in 0..w {
            if code((y as usize * w as usize + x as usize) * 4 + 3) > 0 {
                any = true;
                x0 = x0.min(x);
                x1 = x1.max(x);
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
    }
    let crop = if any {
        let (x0, y0) = (x0.saturating_sub(2), y0.saturating_sub(2));
        let (x1, y1) = ((x1 + 2).min(w - 1), (y1 + 2).min(h - 1));
        PixelRect {
            x: x0,
            y: y0,
            width: x1 - x0 + 1,
            height: y1 - y0 + 1,
        }
    } else {
        PixelRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        }
    };
    debug_assert!(n > 0);
    let mut pixels = Vec::with_capacity(crop.width as usize * crop.height as usize * 4);
    for y in crop.y..crop.y + crop.height {
        for x in crop.x..crop.x + crop.width {
            let i = (y as usize * w as usize + x as usize) * 4;
            let a = f32::from(code(i + 3)) / max;
            for k in 0..3 {
                pixels.push(f32::from(code(i + k)) / max * a);
            }
            pixels.push(a);
        }
    }
    Ok(DecodedSurface {
        size: Size::new(w, h),
        crop,
        pixels: pixels.into(),
        bit_depth: depth,
    })
}

/// Checks a decoded surface has the size the frame asked for.
pub fn check_size(surface: &DecodedSurface, width: u32, height: u32) -> Result<(), SurfaceError> {
    if surface.size.width == width && surface.size.height == height {
        Ok(())
    } else {
        Err(SurfaceError::Size {
            got_w: surface.size.width,
            got_h: surface.size.height,
            want_w: width,
            want_h: height,
        })
    }
}

/// Writes straight RGBA16 samples as a PNG with the given ancillary chunks
/// before `IDAT` (tests and tools).
pub fn encode_rgba16(width: u32, height: u32, rgba: &[u16], chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut e = png::Encoder::new(&mut out, width, height);
        e.set_color(png::ColorType::Rgba);
        e.set_depth(png::BitDepth::Sixteen);
        let mut w = e.write_header().expect("header");
        for (kind, data) in chunks {
            w.write_chunk(png::chunk::ChunkType(**kind), data)
                .expect("chunk");
        }
        let bytes: Vec<u8> = rgba.iter().flat_map(|v| v.to_be_bytes()).collect();
        w.write_image_data(&bytes).expect("data");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_decode_exactly_and_premultiply_in_e() {
        // 8 × 6 surface, two opaque-ish pixels, with a gAMA and an sRGB chunk
        // that must be ignored.
        let (w, h) = (8u32, 6u32);
        let mut rgba = vec![0u16; (w * h * 4) as usize];
        let put = |rgba: &mut Vec<u16>, x: u32, y: u32, p: [u16; 4]| {
            let i = ((y * w + x) * 4) as usize;
            rgba[i..i + 4].copy_from_slice(&p);
        };
        put(&mut rgba, 3, 2, [65535, 32768, 1, 65535]);
        put(&mut rgba, 4, 3, [40000, 20000, 10000, 16384]);
        // Straight color under zero alpha stays out of the crop and the output.
        put(&mut rgba, 7, 5, [65535, 65535, 65535, 0]);
        let png = encode_rgba16(
            w,
            h,
            &rgba,
            &[(b"gAMA", &45455u32.to_be_bytes()), (b"sRGB", &[0])],
        );
        let s = decode_surface(&png).unwrap();
        assert_eq!(s.bit_depth, 16);
        assert_eq!(s.size, Size::new(8, 6));
        assert_eq!(
            s.crop,
            PixelRect {
                x: 1,
                y: 0,
                width: 6,
                height: 6
            }
        );
        let p = s.pixel(3, 2);
        assert_eq!(p, [1.0, 32768.0 / 65535.0, 1.0 / 65535.0, 1.0]);
        let a = 16384.0f32 / 65535.0;
        assert_eq!(
            s.pixel(4, 3),
            [
                40000.0 / 65535.0 * a,
                20000.0 / 65535.0 * a,
                10000.0 / 65535.0 * a,
                a
            ]
        );
        assert_eq!(s.pixel(7, 5), [0.0; 4]);
        assert_eq!(s.rect(), [1.0 / 8.0, 0.0, 6.0 / 8.0, 1.0]);
        assert!(check_size(&s, 8, 6).is_ok());
        assert!(check_size(&s, 16, 12).is_err());
    }

    #[test]
    fn empty_surfaces_keep_one_transparent_pixel() {
        let png = encode_rgba16(4, 4, &[0; 64], &[]);
        let s = decode_surface(&png).unwrap();
        assert_eq!(
            s.crop,
            PixelRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        );
        assert_eq!(&*s.pixels, &[0.0; 4]);
    }

    #[test]
    fn eight_bit_surfaces_decode_as_code_over_255() {
        let mut out = Vec::new();
        {
            let mut e = png::Encoder::new(&mut out, 1, 1);
            e.set_color(png::ColorType::Rgba);
            e.set_depth(png::BitDepth::Eight);
            e.write_header()
                .unwrap()
                .write_image_data(&[255, 128, 0, 255])
                .unwrap();
        }
        let s = decode_surface(&out).unwrap();
        assert_eq!(s.bit_depth, 8);
        assert_eq!(s.pixel(0, 0), [1.0, 128.0 / 255.0, 0.0, 1.0]);
        assert!(decode_surface(b"not a png").is_err());
    }
}
