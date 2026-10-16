//! Glyph outlines at device size, with synthetic bold and italic and the
//! profile's stem-darkening expansion, cached across frames.
//!
//! Outlines come from Skrifa (unhinted, at the variation location of the
//! run), so every host renders the same coverage (S4, "Determinism and
//! portability"). Paths are in device pixels, y down, relative to the glyph
//! origin on the baseline.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{Affine, BezPath, Diagonal2, Join, Rect, Shape};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::{GlyphId, MetadataProvider};

use crate::font::FaceInfo;

/// Skia's synthetic italic: `SkFont::setSkewX(-1/4)` (Blink's
/// `FontPlatformData::SetupSkFont`), in y-down device space.
pub const SYNTHETIC_ITALIC_SKEW: f64 = -0.25;

/// Skia's fake-bold outset (`SkScalerContext`, `kStdFakeBoldInterpKeys` 9
/// and 36 px, values 1/24 and 1/32 of the size): the outline is stroked
/// with this width, so each side grows by half of it.
pub fn synthetic_bold_width(size: f64) -> f64 {
    let t = ((size - 9.0) / (36.0 - 9.0)).clamp(0.0, 1.0);
    size * (1.0 / 24.0 + (1.0 / 32.0 - 1.0 / 24.0) * t)
}

/// Kurbo's expansion settings as Vello's `FontEmbolden` defaults them.
pub const EXPAND_MITER_LIMIT: f64 = 4.0;
pub const EXPAND_TOLERANCE: f64 = 0.1;

/// The outline of one glyph as drawn.
#[derive(Clone, Debug)]
pub struct Outline {
    pub path: Arc<BezPath>,
    /// Control-point bounds of `path` (a superset of the ink).
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    font: usize,
    index: u32,
    glyph: u32,
    size: u32,
    coords: Vec<(u32, u32)>,
    bold: bool,
    italic: bool,
    ex: u64,
    ey: u64,
}

/// One face's font bytes and index (the registry's full font, not a subset).
#[derive(Clone, Copy)]
pub struct FaceRef<'a> {
    pub data: &'a [u8],
    pub index: u32,
    /// Buffer identity for caching.
    pub id: usize,
}

impl<'a> FaceRef<'a> {
    pub fn of(info: &'a FaceInfo) -> FaceRef<'a> {
        FaceRef {
            data: info.data.as_slice(),
            index: info.index,
            id: info.data.ptr_id(),
        }
    }

    pub fn font(&self) -> Option<skrifa::FontRef<'a>> {
        skrifa::FontRef::from_index(self.data, self.index).ok()
    }
}

/// The glyph outline cache.
#[derive(Default)]
pub struct Outlines {
    map: HashMap<Key, Option<Arc<Outline>>>,
    /// Normalized coordinates per (font, index, user coords).
    locations: HashMap<(usize, u32, Vec<(u32, u32)>), Arc<Vec<skrifa::instance::NormalizedCoord>>>,
}

/// How a glyph is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphStyle {
    pub size: f32,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    /// Outline expansion per side in device px.
    pub expand: (f64, f64),
}

struct Pen(BezPath);

impl OutlinePen for Pen {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.move_to((f64::from(x), -f64::from(y)));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.line_to((f64::from(x), -f64::from(y)));
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.0.quad_to(
            (f64::from(cx0), -f64::from(cy0)),
            (f64::from(x), -f64::from(y)),
        );
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.0.curve_to(
            (f64::from(cx0), -f64::from(cy0)),
            (f64::from(cx1), -f64::from(cy1)),
            (f64::from(x), -f64::from(y)),
        );
    }
    fn close(&mut self) {
        self.0.close_path();
    }
}

impl Outlines {
    pub fn new() -> Self {
        Self::default()
    }

    /// Drops every cached outline (fonts changed).
    pub fn clear(&mut self) {
        self.map.clear();
        self.locations.clear();
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn location(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
    ) -> Arc<Vec<skrifa::instance::NormalizedCoord>> {
        let key = (
            face.id,
            face.index,
            coords.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
        );
        self.locations
            .entry(key)
            .or_insert_with(|| {
                if coords.is_empty() {
                    return Arc::new(Vec::new());
                }
                let Some(font) = face.font() else {
                    return Arc::new(Vec::new());
                };
                let loc = font
                    .axes()
                    .location(coords.iter().map(|(t, v)| (skrifa::Tag::from_u32(*t), *v)));
                Arc::new(loc.coords().to_vec())
            })
            .clone()
    }

    /// The outline of `glyph` drawn in `style`, or `None` for a glyph
    /// without one (spaces, bitmap-only glyphs).
    pub fn get(
        &mut self,
        face: FaceRef<'_>,
        coords: &[(u32, f32)],
        glyph: u32,
        style: GlyphStyle,
    ) -> Option<Arc<Outline>> {
        let key = Key {
            font: face.id,
            index: face.index,
            glyph,
            size: style.size.to_bits(),
            coords: coords.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
            bold: style.synthetic_bold,
            italic: style.synthetic_italic,
            ex: style.expand.0.to_bits(),
            ey: style.expand.1.to_bits(),
        };
        if let Some(hit) = self.map.get(&key) {
            return hit.clone();
        }
        let location = self.location(face, coords);
        let built = build(face, &location, glyph, style).map(Arc::new);
        // Bounded: a long session with animated sizes would grow without
        // limit otherwise.
        if self.map.len() > 65_536 {
            self.map.clear();
        }
        self.map.insert(key, built.clone());
        built
    }
}

fn build(
    face: FaceRef<'_>,
    location: &[skrifa::instance::NormalizedCoord],
    glyph: u32,
    style: GlyphStyle,
) -> Option<Outline> {
    let font = face.font()?;
    let outlines = font.outline_glyphs();
    let outline = outlines.get(GlyphId::new(glyph))?;
    let mut pen = Pen(BezPath::new());
    let settings = DrawSettings::unhinted(Size::new(style.size), LocationRef::new(location));
    outline.draw(settings, &mut pen).ok()?;
    let mut path = pen.0;
    if path.elements().is_empty() {
        return None;
    }
    let size = f64::from(style.size);
    if style.synthetic_bold {
        let half = synthetic_bold_width(size) / 2.0;
        path = kurbo::expand_path(
            &path,
            Diagonal2::new(half, half),
            Join::Miter,
            EXPAND_MITER_LIMIT,
            EXPAND_TOLERANCE,
        );
    }
    if style.synthetic_italic {
        path.apply_affine(Affine::new([1.0, 0.0, SYNTHETIC_ITALIC_SKEW, 1.0, 0.0, 0.0]));
    }
    let (ex, ey) = style.expand;
    if ex != 0.0 || ey != 0.0 {
        path = kurbo::expand_path(
            &path,
            Diagonal2::new(ex, ey),
            Join::Miter,
            EXPAND_MITER_LIMIT,
            EXPAND_TOLERANCE,
        );
    }
    let bounds = path.bounding_box();
    Some(Outline {
        path: Arc::new(path),
        bounds,
    })
}

/// Units per em of a face (1000 when the `head` table is unreadable).
pub fn units_per_em(face: FaceRef<'_>) -> f64 {
    face.font()
        .and_then(|f| f.head().ok().map(|h| f64::from(h.units_per_em())))
        .unwrap_or(1000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fake_bold_follows_skias_interpolation() {
        assert!((synthetic_bold_width(9.0) - 9.0 / 24.0).abs() < 1e-12);
        assert!((synthetic_bold_width(36.0) - 36.0 / 32.0).abs() < 1e-12);
        assert!((synthetic_bold_width(100.0) - 100.0 / 32.0).abs() < 1e-12);
        assert!((synthetic_bold_width(4.0) - 4.0 / 24.0).abs() < 1e-12);
    }
}
