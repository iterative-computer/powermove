//! Color bitmap glyphs (Apple Color Emoji's `sbix`, `CBDT`), placed as
//! Canvas2D places them.
//!
//! Canvas2D draws Apple Color Emoji with the strike chosen for the device
//! size, scaled to it, with Apple's fixed downward shift of size / 8 (the
//! shift CoreText applies to that font; HarfBuzz issue 2679). Skrifa picks
//! the strike: the smallest one at least as large as the device size, else
//! the largest. S4 measured this placement against Canvas2D (ink centroid
//! 26.67 px for both at 48 px), and CoreText's own `CTFontDrawGlyphs`
//! 6 px higher, so Canvas2D placement is the rule for both levels
//! (amendment 10).
//!
//! The strike image is resampled once to the device scale with an area
//! filter (premultiplied), so the paint pass samples it close to 1:1.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use kurbo::{Affine, Rect, Vec2};
use skrifa::bitmap::{BitmapData, BitmapFormat, Origin};
use skrifa::instance::Size;
use skrifa::raw::TableProvider;
use skrifa::{GlyphId, MetadataProvider};

use super::outline::FaceRef;

/// A premultiplied RGBA8 image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba8 {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// A color glyph image and where it goes, relative to the glyph origin in
/// device px (y down).
#[derive(Clone, Debug)]
pub struct BitmapGlyph {
    pub image: Arc<Rgba8>,
    /// Image pixels to device px relative to the glyph origin.
    pub transform: Affine,
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    font: usize,
    index: u32,
    glyph: u32,
    size: u32,
}

/// Whether a face has color bitmap strikes.
pub fn has_bitmaps(face: FaceRef<'_>) -> bool {
    face.font().is_some_and(|f| !f.bitmap_strikes().is_empty())
}

/// Whether a face has color vector glyphs (`COLR` with `CPAL`), which this
/// renderer does not draw.
pub fn has_colr(face: FaceRef<'_>) -> bool {
    face.font()
        .is_some_and(|f| f.colr().is_ok() && f.cpal().is_ok())
}

/// The bitmap glyph cache.
#[derive(Default)]
pub struct Bitmaps {
    map: HashMap<Key, Option<Arc<BitmapGlyph>>>,
}

impl Bitmaps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.map.clear();
    }

    /// The color bitmap of `glyph` at `size` device px, or `None` when the
    /// glyph has no bitmap at any strike.
    pub fn get(&mut self, face: FaceRef<'_>, glyph: u32, size: f32) -> Option<Arc<BitmapGlyph>> {
        let key = Key {
            font: face.id,
            index: face.index,
            glyph,
            size: size.to_bits(),
        };
        if let Some(hit) = self.map.get(&key) {
            return hit.clone();
        }
        let built = build(face, glyph, size).map(Arc::new);
        if self.map.len() > 4096 {
            self.map.clear();
        }
        self.map.insert(key, built.clone());
        built
    }
}

fn decode(data: &BitmapData<'_>, width: u32, height: u32) -> Option<Rgba8> {
    let rgba: Vec<u8> = match data {
        BitmapData::Png(bytes) => {
            let mut decoder = png::Decoder::new(Cursor::new(*bytes));
            decoder.set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().ok()?;
            if reader.output_color_type() != (png::ColorType::Rgba, png::BitDepth::Eight) {
                // Gray with alpha expands to gray-alpha; widen it.
                let (ct, bd) = reader.output_color_type();
                if bd != png::BitDepth::Eight {
                    return None;
                }
                let mut buf = vec![0; reader.output_buffer_size()?];
                let info = reader.next_frame(&mut buf).ok()?;
                let px = info.width as usize * info.height as usize;
                return Some(premultiply(Rgba8 {
                    width: info.width,
                    height: info.height,
                    data: match ct {
                        png::ColorType::GrayscaleAlpha => (0..px)
                            .flat_map(|i| [buf[i * 2], buf[i * 2], buf[i * 2], buf[i * 2 + 1]])
                            .collect(),
                        png::ColorType::Rgb => (0..px)
                            .flat_map(|i| [buf[i * 3], buf[i * 3 + 1], buf[i * 3 + 2], 255])
                            .collect(),
                        png::ColorType::Grayscale => {
                            (0..px).flat_map(|i| [buf[i], buf[i], buf[i], 255]).collect()
                        }
                        _ => return None,
                    },
                }));
            }
            let mut buf = vec![0; reader.output_buffer_size()?];
            let info = reader.next_frame(&mut buf).ok()?;
            if info.width != width || info.height != height {
                return None;
            }
            buf.truncate(width as usize * height as usize * 4);
            buf
        }
        BitmapData::Bgra(bytes) => {
            if bytes.len() != width as usize * height as usize * 4 {
                return None;
            }
            // `sbix` and `CBDT` BGRA data is premultiplied already.
            return Some(Rgba8 {
                width,
                height,
                data: bytes
                    .chunks_exact(4)
                    .flat_map(|p| [p[2], p[1], p[0], p[3]])
                    .collect(),
            });
        }
        BitmapData::Mask(_) => return None,
    };
    Some(premultiply(Rgba8 {
        width,
        height,
        data: rgba,
    }))
}

fn premultiply(mut img: Rgba8) -> Rgba8 {
    for p in img.data.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for c in &mut p[..3] {
            *c = ((u32::from(*c) * a + 127) / 255) as u8;
        }
    }
    img
}

/// Area-filter resampling of a premultiplied image by `scale` < 1 (larger
/// scales are left to the paint pass's bilinear filter and return the image
/// as it is). Output pixel `o` covers source pixels `[o, o + 1) / scale`;
/// the part of the last row and column beyond the source is transparent.
pub fn resample(img: &Rgba8, scale: f64) -> Rgba8 {
    if !(scale < 1.0) || img.width == 0 || img.height == 0 {
        return img.clone();
    }
    let w = ((f64::from(img.width) * scale).ceil() as u32).max(1);
    let h = ((f64::from(img.height) * scale).ceil() as u32).max(1);
    let (sw, sh) = (img.width as usize, img.height as usize);
    let inv = 1.0 / scale;
    let mut out = vec![0u8; w as usize * h as usize * 4];
    let weights = |o: usize, n: usize| -> Vec<(usize, f64)> {
        let x0 = o as f64 * inv;
        let x1 = (x0 + inv).min(n as f64);
        let mut v = Vec::new();
        let mut i = x0.floor() as usize;
        while (i as f64) < x1 && i < n {
            let a = (i as f64).max(x0);
            let b = ((i + 1) as f64).min(x1);
            if b > a {
                v.push((i, b - a));
            }
            i += 1;
        }
        v
    };
    let wx: Vec<_> = (0..w as usize).map(|x| weights(x, sw)).collect();
    let wy: Vec<_> = (0..h as usize).map(|y| weights(y, sh)).collect();
    for (y, ry) in wy.iter().enumerate() {
        for (x, rx) in wx.iter().enumerate() {
            let mut acc = [0.0f64; 4];
            for &(sy, fy) in ry {
                for &(sx, fx) in rx {
                    let p = &img.data[(sy * sw + sx) * 4..(sy * sw + sx) * 4 + 4];
                    for c in 0..4 {
                        acc[c] += f64::from(p[c]) * fx * fy;
                    }
                }
            }
            let area = inv * inv;
            let o = (y * w as usize + x) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / area).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    Rgba8 {
        width: w,
        height: h,
        data: out,
    }
}

fn build(face: FaceRef<'_>, glyph: u32, size: f32) -> Option<BitmapGlyph> {
    let font = face.font()?;
    let strikes = font.bitmap_strikes();
    if strikes.is_empty() {
        return None;
    }
    let bitmap = strikes.glyph_for_size(Size::new(size), GlyphId::new(glyph))?;
    let image = decode(&bitmap.data, bitmap.width, bitmap.height)?;
    let upem = f32::from(font.head().ok()?.units_per_em());
    let is_apple_color_emoji = strikes.format() == Some(BitmapFormat::Sbix)
        && font
            .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
            .any(|name| name.chars().eq("AppleColorEmoji".chars()));
    let scale = f64::from(size / bitmap.ppem_y);
    let units = f64::from(size / upem);
    let outer = if is_apple_color_emoji {
        Vec2::new(0.0, f64::from(size) / 8.0)
    } else {
        Vec2::new(
            f64::from(bitmap.bearing_x) * units,
            -f64::from(bitmap.bearing_y) * units,
        )
    };
    let mut transform = Affine::translate(outer)
        * Affine::scale(scale)
        * Affine::translate((
            f64::from(bitmap.inner_bearing_x),
            -f64::from(bitmap.inner_bearing_y),
        ));
    if bitmap.placement_origin == Origin::BottomLeft {
        transform *= Affine::translate((0.0, -f64::from(image.height)));
    }
    // Resample to the device scale; a resampled pixel spans 1 / scale
    // source pixels.
    let transform = if scale < 1.0 {
        transform * Affine::scale(1.0 / scale)
    } else {
        transform
    };
    let image = resample(&image, scale);
    let bounds = transform.transform_rect_bbox(Rect::new(
        0.0,
        0.0,
        f64::from(image.width),
        f64::from(image.height),
    ));
    Some(BitmapGlyph {
        image: Arc::new(image),
        transform,
        bounds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn area_resampling_preserves_mean_and_extent() {
        let img = Rgba8 {
            width: 4,
            height: 2,
            data: (0..8).flat_map(|i| [i * 30, 0, 0, 255]).collect(),
        };
        let out = resample(&img, 0.5);
        assert_eq!((out.width, out.height), (2, 1));
        // (0 + 30 + 120 + 150) / 4 = 75
        assert_eq!(out.data[0], 75);
        assert_eq!(out.data[3], 255);
        let same = resample(&img, 1.5);
        assert_eq!(same, img);
    }
}
