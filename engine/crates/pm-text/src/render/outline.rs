//! Glyph outlines at device size, with synthetic bold and italic and the
//! cached across frames.
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

/// Outline expansion: round joins (a true dilation, as font smoothing
/// is; miter joins grew spikes several pixels long at the acute corners of
/// fonts such as Marker Felt). The miter limit only matters for synthetic
/// bold, which keeps miters like Skia's fake bold.
pub const EXPAND_MITER_LIMIT: f64 = 4.0;

/// The outline of one glyph as drawn.
#[derive(Clone, Debug)]
pub struct Outline {
    pub path: Arc<BezPath>,
    /// Control-point bounds of `path` (a superset of the ink).
    pub bounds: Rect,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Key {
    font: u64,
    index: u32,
    glyph: u32,
    size: u32,
    coords: Vec<(u32, u32)>,
    bold: (u64, u64),
    skew: u64,
    expand_tolerance: u64,
}

/// One face's font bytes and index (the registry's full font, not a subset).
#[derive(Clone, Copy)]
pub struct FaceRef<'a> {
    pub data: &'a [u8],
    pub index: u32,
    /// Buffer identity for caching.
    pub id: u64,
}

impl<'a> FaceRef<'a> {
    pub fn of(info: &'a FaceInfo) -> FaceRef<'a> {
        FaceRef {
            data: info.data.as_slice(),
            index: info.index,
            id: info.data.id(),
        }
    }

    pub fn font(&self) -> Option<skrifa::FontRef<'a>> {
        skrifa::FontRef::from_index(self.data, self.index).ok()
    }
}

/// Normalized coordinates of a face at user coordinates.
type LocationKey = (u64, u32, Vec<(u32, u32)>);

/// The glyph outline cache.
#[derive(Default)]
pub struct Outlines {
    map: HashMap<Key, Option<Arc<Outline>>>,
    /// Normalized coordinates per (font, index, user coords).
    locations: HashMap<LocationKey, Arc<Vec<skrifa::instance::NormalizedCoord>>>,
}

/// A run's variation location.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Coords<'a> {
    /// User-space `(tag, value)` pairs (level 1 layout).
    User(&'a [(u32, f32)]),
    /// Normalized F2Dot14 coordinates in `fvar` axis order (level 2 layout).
    Normalized(&'a [i16]),
}

impl Coords<'_> {
    fn key(&self) -> Vec<(u32, u32)> {
        match self {
            Coords::User(c) => c.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
            Coords::Normalized(c) => c.iter().map(|v| (u32::MAX, *v as u16 as u32)).collect(),
        }
    }
}

/// How a glyph is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlyphStyle {
    pub size: f32,
    /// Fake-bold outset per side in device px, x and y (0: not
    /// synthesized bold).
    pub synthetic_bold: (f64, f64),
    /// Synthesized oblique: `x' = x + skew × y` (y down; 0 for none).
    pub skew: f64,
    /// Kurbo fake-bold expansion tolerance in local pixels, chosen from
    /// the final device transform so its error stays below 1e-4 device px.
    pub expand_tolerance: f64,
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
        coords: Coords<'_>,
    ) -> Arc<Vec<skrifa::instance::NormalizedCoord>> {
        let coords = match coords {
            Coords::Normalized(c) => {
                return Arc::new(
                    c.iter()
                        .map(|v| skrifa::instance::NormalizedCoord::from_bits(*v))
                        .collect(),
                );
            }
            Coords::User(c) => c,
        };
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
        coords: Coords<'_>,
        glyph: u32,
        style: GlyphStyle,
    ) -> Option<Arc<Outline>> {
        let key = Key {
            font: face.id,
            index: face.index,
            glyph,
            size: style.size.to_bits(),
            coords: coords.key(),
            bold: (
                style.synthetic_bold.0.to_bits(),
                style.synthetic_bold.1.to_bits(),
            ),
            skew: style.skew.to_bits(),
            expand_tolerance: style.expand_tolerance.to_bits(),
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
    if style.synthetic_bold.0 > 0.0 || style.synthetic_bold.1 > 0.0 {
        let (bx, by) = style.synthetic_bold;
        path = kurbo::expand_path(
            &path,
            Diagonal2::new(bx, by),
            Join::Miter,
            EXPAND_MITER_LIMIT,
            style.expand_tolerance,
        );
    }
    if style.skew != 0.0 {
        path.apply_affine(Affine::new([1.0, 0.0, style.skew, 1.0, 0.0, 0.0]));
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
