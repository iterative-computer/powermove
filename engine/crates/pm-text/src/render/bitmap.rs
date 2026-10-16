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
    requested: u32,
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

    /// The color bitmap of `glyph` for a run at `requested` device px,
    /// drawn at `size` device px, or `None` when the glyph has no bitmap at
    /// any strike. `size` is the size CoreText draws the glyph at: for
    /// Apple Color Emoji, the face's full-em advance at the requested size
    /// (`FontResolver::em_advance`), which exceeds the requested size below
    /// 24 px.
    pub fn get(
        &mut self,
        face: FaceRef<'_>,
        glyph: u32,
        requested: f32,
        size: f32,
    ) -> Option<Arc<BitmapGlyph>> {
        let key = Key {
            font: face.id,
            index: face.index,
            glyph,
            size: size.to_bits(),
            requested: requested.to_bits(),
        };
        if let Some(hit) = self.map.get(&key) {
            return hit.clone();
        }
        let built = build(face, glyph, requested, size).map(Arc::new);
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

/// Lanczos-2 resampling of a premultiplied image by `scale` < 1 (the
/// kernel widened by 1 / scale); output pixel `o` is centered on source
/// position `(o + 0.5) / scale`. Larger scales return the image as it is.
pub fn lanczos2(img: &Rgba8, scale: f64) -> Rgba8 {
    if !(scale < 1.0) || img.width == 0 || img.height == 0 {
        return img.clone();
    }
    const SUPPORT: f64 = 2.0;
    let k = |x: f64| -> f64 {
        let x = x.abs();
        if x < 1e-9 {
            1.0
        } else if x >= SUPPORT {
            0.0
        } else {
            let p = std::f64::consts::PI * x;
            SUPPORT * p.sin() * (p / SUPPORT).sin() / (p * p)
        }
    };
    let w = ((f64::from(img.width) * scale).ceil() as u32).max(1);
    let h = ((f64::from(img.height) * scale).ceil() as u32).max(1);
    let weights = |o: u32, n: u32| -> Vec<(usize, f64)> {
        let center = (f64::from(o) + 0.5) / scale - 0.5;
        let r = SUPPORT / scale;
        let lo = (center - r).floor() as i64;
        let hi = (center + r).ceil() as i64;
        // Normalized over the whole kernel: taps outside the image are
        // transparent, so edges fade.
        let sum: f64 = (lo..=hi).map(|i| k((i as f64 - center) * scale)).sum();
        (lo..=hi)
            .filter(|i| *i >= 0 && *i < i64::from(n))
            .map(|i| (i as usize, k((i as f64 - center) * scale) / sum))
            .collect()
    };
    let wx: Vec<_> = (0..w).map(|x| weights(x, img.width)).collect();
    let wy: Vec<_> = (0..h).map(|y| weights(y, img.height)).collect();
    let sw = img.width as usize;
    let mut out = vec![0u8; w as usize * h as usize * 4];
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
            let o = (y * w as usize + x) * 4;
            // Premultiplied: color never exceeds alpha (the kernel rings).
            let a = acc[3].round().clamp(0.0, 255.0);
            out[o + 3] = a as u8;
            for c in 0..3 {
                out[o + c] = acc[c].round().clamp(0.0, a) as u8;
            }
        }
    }
    Rgba8 {
        width: w,
        height: h,
        data: out,
    }
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

/// Apple Color Emoji's downward shift in whole device pixels for a run at
/// `requested` px drawn at `size` px. From 20 px up it is `size / 8`
/// rounded, halves down; below, Canvas2D shifts the (enlarged) glyph
/// further, by the amounts measured at every size from 6 to 19 px
/// (`fit-emoji`, two emoji, every pixel equal after the shift).
pub fn apple_emoji_shift(requested: f32, size: f32) -> f64 {
    const SMALL: [f64; 14] = [1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 3.0, 4.0, 4.0, 4.0, 4.0, 4.0];
    let r = f64::from(requested).round();
    if r < 20.0 {
        if r >= 6.0 {
            return SMALL[(r - 6.0) as usize];
        }
        // Below the measured range: a quarter of the size, halves down.
        return (r / 4.0 - 0.5).ceil().max(0.0);
    }
    (f64::from(size) / 8.0 - 0.5).ceil()
}

fn build(face: FaceRef<'_>, glyph: u32, requested: f32, size: f32) -> Option<BitmapGlyph> {
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
        Vec2::new(0.0, apple_emoji_shift(requested, size))
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
    // Downscale to the device scale with a Lanczos-2 filter (closest to
    // Canvas2D among area, linear, Mitchell, Lanczos-2 and -3 on
    // `fit-emoji`); a resampled pixel spans 1 / scale source pixels, and
    // the paint pass samples it bilinearly at the glyph's phase.
    let transform = if scale < 1.0 {
        transform * Affine::scale(1.0 / scale)
    } else {
        transform
    };
    let image = if scale < 1.0 {
        lanczos2(&image, scale)
    } else {
        image
    };
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
    fn emoji_shift_follows_the_measurements() {
        assert_eq!(apple_emoji_shift(12.0, 15.0), 3.0);
        assert_eq!(apple_emoji_shift(19.0, 22.0), 4.0);
        assert_eq!(apple_emoji_shift(20.0, 22.0), 3.0);
        assert_eq!(apple_emoji_shift(36.0, 36.0), 4.0);
        assert_eq!(apple_emoji_shift(37.0, 37.0), 5.0);
        assert_eq!(apple_emoji_shift(4.0, 6.0), 1.0);
    }

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

    #[test]
    fn lanczos_keeps_flat_interiors_and_premultiplication() {
        let img = Rgba8 {
            width: 20,
            height: 20,
            data: (0..400).flat_map(|_| [100, 50, 0, 200]).collect(),
        };
        let out = lanczos2(&img, 0.4);
        assert_eq!((out.width, out.height), (8, 8));
        let mid = &out.data[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 4];
        assert_eq!(mid, &[100, 50, 0, 200]);
        assert!(out.data.chunks_exact(4).all(|p| p[0] <= p[3] && p[1] <= p[3]));
    }
}
