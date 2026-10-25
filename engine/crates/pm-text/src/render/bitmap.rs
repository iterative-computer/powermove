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
//! The original strike pixels and authored placement are retained. The
//! renderer converts to its working color space before area minification.

use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Arc;

use kurbo::{Affine, Rect, Vec2};
use skrifa::bitmap::{BitmapData, BitmapFormat, BitmapGlyph as StrikeGlyph, Origin};
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
    font: u64,
    index: u32,
    glyph: u32,
    size: u32,
    requested: u32,
}

/// Whether a face has color bitmap strikes.
pub fn has_bitmaps(face: FaceRef<'_>) -> bool {
    face.font().is_some_and(|f| !f.bitmap_strikes().is_empty())
}

/// Whether `glyph` is a color vector glyph (`COLR`) of the face.
pub fn is_colr_glyph(face: FaceRef<'_>, glyph: u32) -> bool {
    face.font()
        .is_some_and(|f| f.color_glyphs().get(GlyphId::new(glyph)).is_some())
}

/// The bitmap glyph cache.
#[derive(Default)]
pub struct Bitmaps {
    map: HashMap<Key, Option<Arc<BitmapGlyph>>>,
    pub(crate) colr: super::colr::Cache,
}

impl Bitmaps {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.colr.clear();
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
            decoder
                .set_transformations(png::Transformations::ALPHA | png::Transformations::STRIP_16);
            let mut reader = decoder.read_info().ok()?;
            if reader.output_color_type() != (png::ColorType::Rgba, png::BitDepth::Eight) {
                // Gray with alpha expands to gray-alpha; widen it.
                let (ct, bd) = reader.output_color_type();
                if bd != png::BitDepth::Eight {
                    return None;
                }
                let mut buf = vec![0; reader.output_buffer_size()?];
                let info = reader.next_frame(&mut buf).ok()?;
                if info.width != width || info.height != height {
                    return None;
                }
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
                        png::ColorType::Grayscale => (0..px)
                            .flat_map(|i| [buf[i], buf[i], buf[i], 255])
                            .collect(),
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

/// Apple Color Emoji's downward shift in whole device pixels for a run at
/// `requested` px drawn at `size` px. From 20 px up it is `size / 8`
/// rounded, halves down; below, Canvas2D shifts the (enlarged) glyph
/// further, by the amounts measured at every size from 6 to 19 px
/// (`fit-emoji`, two emoji, every pixel equal after the shift).
/// Whether a face has color bitmap strikes (`sbix`, `CBDT`): glyphs Skia
/// draws as color images. Monochrome `EBDT` strikes (Courier New) draw as
/// masks.
pub fn has_color_bitmaps(face: FaceRef<'_>) -> bool {
    face.font().is_some_and(|f| {
        matches!(
            f.bitmap_strikes().format(),
            Some(BitmapFormat::Sbix | BitmapFormat::Cbdt)
        )
    })
}

/// Whether a face is Apple Color Emoji (the `sbix` font Canvas2D shifts
/// down, `apple_emoji_shift`).
pub fn is_apple_color_emoji(face: FaceRef<'_>) -> bool {
    face.font().is_some_and(|font| {
        font.bitmap_strikes().format() == Some(BitmapFormat::Sbix)
            && font
                .localized_strings(skrifa::string::StringId::POSTSCRIPT_NAME)
                .any(|name| name.chars().eq("AppleColorEmoji".chars()))
    })
}

pub fn apple_emoji_shift(requested: f32, size: f32) -> f64 {
    const SMALL: [f64; 14] = [
        1.0, 2.0, 2.0, 2.0, 2.0, 3.0, 3.0, 3.0, 3.0, 4.0, 4.0, 4.0, 4.0, 4.0,
    ];
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

/// The drawn area of a bitmap glyph relative to its origin, at device
/// scale 1: its nonzero-alpha texels through the authored placement (the
/// draw path's strike and sizes), grown by the reconstruction's reach. A
/// device pixel is nonzero exactly when its square meets this rectangle.
/// Enlargement samples bilinearly, so texel `k` weighs on samples within
/// `(k - 0.5, k + 1.5)` texels; as an area rectangle that is `scale / 2 -
/// 1 / 2` px past the texel edge. Minification integrates texel area, so
/// it reaches no further. `None` when the glyph has no color strike (the
/// painter falls back to its outline); `Some(None)` when it draws nothing.
pub fn ink_bounds(
    face: FaceRef<'_>,
    glyph: u32,
    requested: f32,
    size: f32,
) -> Option<Option<Rect>> {
    let font = face.font()?;
    let bitmap = font
        .bitmap_strikes()
        .glyph_for_size(Size::new(size), GlyphId::new(glyph))?;
    let image = decode(&bitmap.data, bitmap.width, bitmap.height)?;
    let upem = f32::from(font.head().ok()?.units_per_em());
    let (transform, _) =
        authored_placement(&bitmap, upem, is_apple_color_emoji(face), requested, size);
    let Some(content) = alpha_bounds(&image) else {
        return Some(None);
    };
    let reach = (f64::from(size) / f64::from(bitmap.ppem_y) / 2.0 - 0.5).max(0.0);
    Some(Some(
        transform.transform_rect_bbox(content).inflate(reach, reach),
    ))
}

/// The texel rectangle of an image's nonzero alpha.
fn alpha_bounds(image: &Rgba8) -> Option<Rect> {
    let w = image.width as usize;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
    for (i, px) in image.data.chunks_exact(4).enumerate() {
        if px[3] != 0 {
            let (x, y) = (i % w, i / w);
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
        }
    }
    (x1 > 0).then(|| Rect::new(x0 as f64, y0 as f64, x1 as f64, y1 as f64))
}

fn authored_placement(
    bitmap: &StrikeGlyph<'_>,
    upem: f32,
    is_apple_color_emoji: bool,
    requested: f32,
    size: f32,
) -> (Affine, Rect) {
    let scale = f64::from(size) / f64::from(bitmap.ppem_y);
    let units = f64::from(size) / f64::from(upem);
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
        transform *= Affine::translate((0.0, -f64::from(bitmap.height)));
    }
    // Keep the source strike at native resolution. `transform` carries the
    // complete strike-to-device scale; the GPU integrates its footprint
    // after converting the pixels into the project's working color space.
    let bounds = transform.transform_rect_bbox(Rect::new(
        0.0,
        0.0,
        f64::from(bitmap.width),
        f64::from(bitmap.height),
    ));
    (transform, bounds)
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
    let (transform, bounds) =
        authored_placement(&bitmap, upem, is_apple_color_emoji(face), requested, size);
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
    fn emoji_ink_bounds_follow_the_authored_strike_placement_at_several_sizes() {
        let strike = StrikeGlyph {
            data: BitmapData::Png(&[]),
            bearing_x: 0.0,
            bearing_y: 0.0,
            inner_bearing_x: 2.0,
            inner_bearing_y: 30.0,
            ppem_x: 40.0,
            ppem_y: 40.0,
            advance: None,
            width: 40,
            height: 40,
            placement_origin: Origin::TopLeft,
        };
        for (size, expected) in [
            (40.0, Rect::new(2.0, -25.0, 42.0, 15.0)),
            (80.0, Rect::new(4.0, -50.0, 84.0, 30.0)),
            (200.0, Rect::new(10.0, -125.0, 210.0, 75.0)),
        ] {
            let (transform, ink) = authored_placement(&strike, 1000.0, true, size, size);
            assert_eq!(ink, expected);
            assert_eq!(
                transform.transform_rect_bbox(Rect::new(0.0, 0.0, 40.0, 40.0)),
                ink
            );
        }
    }

    /// The pixels the painter makes nonzero for one glyph drawn at `origin`.
    #[cfg(target_os = "macos")]
    fn painted_box(
        face: FaceRef<'_>,
        glyph: u32,
        size: f32,
        bitmap_size: f32,
        origin: (f64, f64),
    ) -> [i64; 4] {
        use crate::render::scene::{Painter, Run, TextScene};
        let mut outlines = crate::render::outline::Outlines::new();
        let mut bitmaps = Bitmaps::new();
        let mut painter = Painter {
            outlines: &mut outlines,
            bitmaps: &mut bitmaps,
            unsupported: Vec::new(),
        };
        let glyphs = [crate::compat::PlacedGlyph {
            id: glyph,
            x: 0.0,
            y: 0.0,
        }];
        let run = Run {
            face,
            size,
            coords: crate::render::outline::Coords::Normalized(&[]),
            synthetic_bold: false,
            synthetic_italic: false,
            oblique: 0.0,
            glyphs: &glyphs,
            bitmap_size: Some(bitmap_size),
        };
        let mut scene = TextScene {
            width: 320,
            height: 320,
            items: Vec::new(),
        };
        painter.fill_text(
            &mut scene.items,
            &[run],
            origin,
            Affine::IDENTITY,
            [1.0; 4],
            1.0,
            0.0,
        );
        let image = crate::render::paint::render_cpu(&scene);
        let mut b = [i64::MAX, i64::MAX, i64::MIN, i64::MIN];
        // Nonzero as stored: an alpha code of at least one in RGBA8.
        for (i, px) in image.data.chunks_exact(4).enumerate() {
            if (px[3] * 255.0).round() >= 1.0 {
                let (x, y) = ((i % 320) as i64, (i / 320) as i64);
                b = [b[0].min(x), b[1].min(y), b[2].max(x + 1), b[3].max(y + 1)];
            }
        }
        b
    }

    /// Ink is what the painter draws: at every size (minified below the
    /// strike, enlarged above it, enlarged full-em draws below 24 px) and
    /// sub-pixel phase, the nonzero pixels are exactly the pixels whose
    /// squares meet the ink rectangle. Covers a shaped skin-tone sequence.
    #[cfg(target_os = "macos")]
    #[test]
    fn installed_apple_emoji_ink_is_the_painted_pixels() {
        let file = std::fs::File::open("/System/Library/Fonts/Apple Color Emoji.ttc")
            .expect("Apple Color Emoji system font");
        // The mapping borrows a stable, read-only system file for this test.
        let bytes = std::sync::Arc::new(unsafe { memmap2::MmapOptions::new().map(&file) }.unwrap());
        let face = FaceRef {
            data: &bytes[..],
            index: 0,
            id: 0,
        };
        let shared: std::sync::Arc<dyn AsRef<[u8]> + Send + Sync> = bytes.clone();
        let font = read_fonts::model::Font::new(read_fonts::model::Blob::from(shared), 0).unwrap();
        let shape = |text: &str, size: f32| {
            let shaper = harfrust::ShaperFont::new(&font).with_scale((size * 65536.0) as i32);
            let mut buffer = harfrust::Buffer::new();
            for (cluster, ch) in text.char_indices() {
                buffer.push(ch as u32, cluster as u32);
            }
            buffer.set_direction(harfrust::Direction::LeftToRight);
            harfrust::shape(&shaper, &mut buffer, harfrust::ShapeOptions::new()).unwrap();
            let [glyph] = buffer.glyph_infos() else {
                panic!("{text} must shape to one color glyph at size {size}");
            };
            glyph.glyph_id
        };
        let wave = face.font().unwrap().charmap().map('👋').unwrap().to_u32();
        // (requested, drawn): small emoji draw at an enlarged full-em size.
        for (requested, drawn) in [
            (12.0, 15.0),
            (23.0, 24.0),
            (40.0, 40.0),
            (96.0, 96.0),
            (200.0, 200.0),
        ] {
            let tone = shape("👋🏽", requested);
            assert_ne!(tone, wave, "skin-tone modifier was not shaped");
            for glyph in [wave, tone] {
                let ink = crate::render::scene::drawn_color_glyph_ink(
                    face,
                    crate::render::outline::Coords::Normalized(&[]),
                    glyph,
                    requested,
                    0.0,
                    Some(drawn),
                )
                .unwrap()
                .unwrap();
                assert_eq!(Some(Some(ink)), ink_bounds(face, glyph, requested, drawn));
                for origin in [
                    (40.0, 250.0),
                    (40.3, 250.7),
                    (40.5, 250.25),
                    (40.875, 250.5),
                ] {
                    let expected = [
                        (ink.x0 + origin.0).floor() as i64,
                        (ink.y0 + origin.1).floor() as i64,
                        (ink.x1 + origin.0).ceil() as i64,
                        (ink.y1 + origin.1).ceil() as i64,
                    ];
                    assert_eq!(
                        painted_box(face, glyph, requested, drawn, origin),
                        expected,
                        "glyph {glyph} at {requested} px (drawn {drawn}), origin {origin:?}: ink {ink:?}"
                    );
                }
            }
        }
    }
}
