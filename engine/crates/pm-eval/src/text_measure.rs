//! [`TextMeasure`] over `pm-text`'s level 1 layout (the Rust port of Blink's
//! `measureText`), with Blink's ink boxes.
//!
//! Widths, wrapping, and the glyph layout are `pm_text::compat`. The ink box
//! of a line (Blink's `glyph_bounds`, which `actualBoundingBox*` derive
//! from) is the union of each placed glyph's outline box: CoreText's glyph
//! bounding rectangle, which on the corpus is the control box of the glyph's
//! outline in font units scaled by `size / unitsPerEm`, placed at the glyph's
//! pen position and baseline offset.
//!
//! Color glyphs (`sbix`/`CBDT` strikes, `COLR`) have no outline. Their box is
//! what the renderer draws, from the painter's own placement
//! (`pm_text::render::scene::drawn_color_glyph_ink`): the strike image at the
//! full-em size CoreText draws it at, or the COLR paint bounds. This is also
//! the image rectangle CoreText reports to Blink for those glyphs.
//!
//! Outlines come from the face's own bytes (`FaceInfo::data`). The
//! metrics-and-shaping subsets `pm-wasm` receives have no outline tables, so
//! ink answers need whole faces there; see `docs/engine/pm-eval-layers.md`.

use std::collections::HashMap;

use pm_text::compat::{Compat, GlyphLayout, TextContent};
use pm_text::font::{FaceId, FontResolver};
use skrifa::instance::{Location, Size};
use skrifa::outline::{DrawSettings, OutlinePen, pen::PathStyle};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};

use crate::text_geom::{MeasuredLine, TextMeasure};

mod level2;
mod lru;
#[cfg(test)]
pub(crate) mod test_fonts;

/// Level 2 uses font-metric baselines and normalized Parley coordinates.
/// Keep outline geometry independent of platform rasterization.
pub(crate) fn level2_ink(
    data: &pm_text::font::FontBytes,
    run: &pm_text::level2::Run,
    glyph: u32,
) -> Option<[f64; 4]> {
    let skew = -libm::tan(f64::from(run.skew) * core::f64::consts::PI / 180.0);
    // Color glyphs: the level 2 painter's strike or COLR program placement.
    if let Some(r) = pm_text::render::scene::drawn_color_glyph_ink(
        pm_text::render::outline::FaceRef {
            data: data.as_slice(),
            index: 0,
            id: data.id(),
        },
        pm_text::render::outline::Coords::Normalized(&run.coords),
        glyph,
        run.size,
        skew,
        None,
    ) {
        return Some([r.x0, r.y0, r.x1, r.y1]);
    }
    let font = FontRef::new(data.as_slice()).ok()?;
    let coords: Vec<_> = run
        .coords
        .iter()
        .map(|v| skrifa::instance::NormalizedCoord::from_bits(*v))
        .collect();
    let mut pen = ControlBox {
        b: [0.0; 4],
        any: false,
    };
    if let Some(outline) = font.outline_glyphs().get(GlyphId::new(glyph)) {
        outline
            .draw(
                DrawSettings::unhinted(Size::new(run.size), coords.as_slice()),
                &mut pen,
            )
            .ok()?;
    }
    if !pen.any {
        // Nothing is drawn for a glyph with neither outline nor color source.
        return None;
    }
    let mut ink = [
        f64::from(pen.b[0]),
        -f64::from(pen.b[3]),
        f64::from(pen.b[2]),
        -f64::from(pen.b[1]),
    ];
    if run.synthetic_bold {
        // The CoreText profile's synthetic face is an outline dilation.
        let x = f64::from(run.size) * 0.00875;
        let y = x * 0.925;
        ink = [ink[0] - x, ink[1] - y, ink[2] + x, ink[3] + y];
    }
    let (a, b) = (skew * ink[1], skew * ink[3]);
    ink[0] += a.min(b);
    ink[2] += a.max(b);
    Some(ink)
}

/// A glyph's control box in font units, and the face's units per em.
type GlyphBox = Option<([f32; 4], f32)>;

/// A face, glyph, and variation coordinates (tag, value bits).
type BoxKey = (FaceId, u32, Vec<(u32, u32)>);
/// A color face, glyph, size bits, and variation coordinates.
type ColorBoxKey = (FaceId, u32, u32, Vec<(u32, u32)>);
const COLOR_BOX_LIMIT_ENTRIES: usize = 4096;

const LEVEL2_GLYPH_LIMIT_ENTRIES: usize = 16_384;
const LEVEL2_GLYPH_LIMIT_BYTES: usize = 2 * 1024 * 1024;
/// A split face's identity, glyph, size, coordinates, synthetic bold and
/// oblique (bits).
type Level2GlyphKey = (u64, u32, u32, Vec<i16>, bool, u32);

/// What one cached glyph box retains: the key twice (the map and the
/// recency index), the coordinates' allocations, and the slot.
fn level2_glyph_entry_bytes(key: &Level2GlyphKey) -> usize {
    let coords = if key.3.capacity() == 0 {
        0
    } else {
        key.3.capacity() * std::mem::size_of::<i16>() + 16
    };
    2 * (std::mem::size_of::<Level2GlyphKey>() + coords)
        + std::mem::size_of::<(Option<[f64; 4]>, u64, usize)>()
        + std::mem::size_of::<u64>()
}

/// Level 2 cache use and its bounds.
///
/// Every bound holds whenever a level 2 call returns.
///
/// - Split faces: at most `face_limit_entries` faces and `face_limit_bytes`
///   bytes, least recently used first. Byte counts include faces that share
///   the resolver's bytes (`face_copied_bytes` is the part split out of
///   collection files). While a request runs, its own faces may exceed the
///   ceiling; it is enforced, on those faces too, before the request returns.
/// - Parley collections (`set_entries`, at most `set_limit_entries`, the
///   active one included) reference only faces in the split-face cache:
///   evicting a face drops every collection and layout that used it. Their
///   own metadata (`set_registrations` faces) is not counted in bytes.
/// - Layouts: at most `layout_limit_entries` and `layout_limit_bytes`, each
///   charged its key (whole span contents included) and layout capacities.
/// - Glyph boxes: at most `glyph_limit_entries` and `glyph_limit_bytes`.
///
/// `retained_bytes` sums the three byte counts; it never exceeds
/// `limit_bytes`. Frames own clones of the fonts they draw, outside the
/// cache, until they are dropped.
#[derive(Clone, Copy, Debug, Default)]
pub struct Level2CacheStats {
    pub face_entries: usize,
    pub face_bytes: usize,
    pub face_copied_bytes: usize,
    pub face_limit_entries: usize,
    pub face_limit_bytes: usize,
    pub set_entries: usize,
    pub set_registrations: usize,
    pub set_limit_entries: usize,
    pub layout_entries: usize,
    pub layout_bytes: usize,
    pub layout_limit_entries: usize,
    pub layout_limit_bytes: usize,
    pub glyph_entries: usize,
    pub glyph_bytes: usize,
    pub glyph_limit_entries: usize,
    pub glyph_limit_bytes: usize,
    pub retained_bytes: usize,
    pub limit_bytes: usize,
    pub face_hits: u64,
    pub face_splits: u64,
    pub face_evictions: u64,
    pub set_hits: u64,
    pub set_builds: u64,
    pub set_evictions: u64,
    pub registrations: u64,
    pub layouts: u64,
    pub layout_hits: u64,
    pub glyph_hits: u64,
    pub glyph_misses: u64,
}

/// Level 1 text measurement with ink boxes from outlines.
pub struct CompatMeasure<R: FontResolver> {
    compat: Compat<R>,
    boxes: HashMap<BoxKey, GlyphBox>,
    color_boxes: HashMap<ColorBoxKey, Option<RectF>>,
    level2: level2::Level2Cache,
    level2_boxes: lru::Lru<Level2GlyphKey, Option<[f64; 4]>>,
    level2_glyph_hits: u64,
    level2_glyph_misses: u64,
}

impl<R: FontResolver> CompatMeasure<R> {
    pub fn new(resolver: R) -> Self {
        CompatMeasure {
            compat: Compat::new(resolver),
            boxes: HashMap::new(),
            color_boxes: HashMap::new(),
            level2: level2::Level2Cache::default(),
            level2_boxes: lru::Lru::default(),
            level2_glyph_hits: 0,
            level2_glyph_misses: 0,
        }
    }

    pub fn compat(&mut self) -> &mut Compat<R> {
        &mut self.compat
    }

    pub fn level2_cache_stats(&self) -> Level2CacheStats {
        let l2 = &self.level2;
        let c = l2.counters;
        Level2CacheStats {
            face_entries: l2.face_entries(),
            face_bytes: l2.face_bytes(),
            face_copied_bytes: l2.copied_bytes(),
            face_limit_entries: l2.face_limit_entries,
            face_limit_bytes: l2.face_limit_bytes,
            set_entries: l2.set_entries(),
            set_registrations: l2.set_registrations(),
            set_limit_entries: level2::SET_LIMIT_ENTRIES,
            layout_entries: l2.layout_entries(),
            layout_bytes: l2.layout_bytes(),
            layout_limit_entries: level2::LAYOUT_LIMIT_ENTRIES,
            layout_limit_bytes: level2::LAYOUT_LIMIT_BYTES,
            glyph_entries: self.level2_boxes.len(),
            glyph_bytes: self.level2_boxes.bytes(),
            glyph_limit_entries: LEVEL2_GLYPH_LIMIT_ENTRIES,
            glyph_limit_bytes: LEVEL2_GLYPH_LIMIT_BYTES,
            retained_bytes: l2.face_bytes() + l2.layout_bytes() + self.level2_boxes.bytes(),
            limit_bytes: l2.face_limit_bytes
                + level2::LAYOUT_LIMIT_BYTES
                + LEVEL2_GLYPH_LIMIT_BYTES,
            face_hits: c.face_hits,
            face_splits: c.face_splits,
            face_evictions: c.face_evictions,
            set_hits: c.set_hits,
            set_builds: c.set_builds,
            set_evictions: c.set_evictions,
            registrations: c.registrations,
            layouts: c.layouts,
            layout_hits: c.layout_hits,
            glyph_hits: self.level2_glyph_hits,
            glyph_misses: self.level2_glyph_misses,
        }
    }

    fn glyph_box(&mut self, face: FaceId, glyph: u32, coords: &[(u32, f32)]) -> GlyphBox {
        let key = (
            face,
            glyph,
            coords.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
        );
        if let Some(b) = self.boxes.get(&key) {
            return *b;
        }
        let info = self.compat.resolver().face(face);
        let computed = outline_box(info.data.as_slice(), info.index, glyph, coords);
        self.boxes.insert(key, computed);
        computed
    }

    /// The drawn box of a color glyph relative to its pen origin (y down),
    /// placed as the level 1 painter places it; `None` for outline glyphs.
    fn color_box(
        &mut self,
        face: FaceId,
        glyph: u32,
        size: f32,
        coords: &[(u32, f32)],
    ) -> Option<RectF> {
        let facts = &self.compat.resolver().face(face).facts;
        if !facts.has_color_table {
            return None;
        }
        let sbix = facts.has_sbix;
        let key = (
            face,
            glyph,
            size.to_bits(),
            coords.iter().map(|(t, v)| (*t, v.to_bits())).collect(),
        );
        if let Some(b) = self.color_boxes.get(&key) {
            return *b;
        }
        // The painter draws `sbix` strikes at CoreText's full-em size.
        let em = if sbix {
            self.compat.resolver_mut().em_advance(face, size)
        } else {
            None
        };
        let info = self.compat.resolver().face(face);
        let skew = if info.synthetic_italic {
            pm_text::render::outline::SYNTHETIC_ITALIC_SKEW
        } else {
            0.0
        };
        let computed = pm_text::render::scene::drawn_color_glyph_ink(
            pm_text::render::outline::FaceRef::of(info),
            pm_text::render::outline::Coords::User(coords),
            glyph,
            size,
            skew,
            em,
        )
        .map(|r| RectF::from_sk(r.x0 as f32, r.y0 as f32, r.x1 as f32, r.y1 as f32));
        if self.color_boxes.len() >= COLOR_BOX_LIMIT_ENTRIES {
            self.color_boxes.clear();
        }
        self.color_boxes.insert(key, computed);
        computed
    }
}

struct ControlBox {
    b: [f32; 4],
    any: bool,
}

impl ControlBox {
    fn point(&mut self, x: f32, y: f32) {
        if !self.any {
            self.b = [x, y, x, y];
            self.any = true;
            return;
        }
        self.b[0] = self.b[0].min(x);
        self.b[1] = self.b[1].min(y);
        self.b[2] = self.b[2].max(x);
        self.b[3] = self.b[3].max(y);
    }
}

impl OutlinePen for ControlBox {
    fn move_to(&mut self, x: f32, y: f32) {
        self.point(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.point(x, y);
    }
    fn quad_to(&mut self, cx0: f32, cy0: f32, x: f32, y: f32) {
        self.point(cx0, cy0);
        self.point(x, y);
    }
    fn curve_to(&mut self, cx0: f32, cy0: f32, cx1: f32, cy1: f32, x: f32, y: f32) {
        self.point(cx0, cy0);
        self.point(cx1, cy1);
        self.point(x, y);
    }
    fn close(&mut self) {}
}

/// The control box of a glyph's outline at a variation location, in font
/// units (y up), with the face's units per em.
fn outline_box(data: &[u8], index: u32, glyph: u32, coords: &[(u32, f32)]) -> GlyphBox {
    let font = FontRef::from_index(data, index).ok()?;
    let upem = f32::from(font.head().ok()?.units_per_em());
    let location: Location = font.axes().location(
        coords
            .iter()
            .map(|(t, v)| (Tag::from_be_bytes(t.to_be_bytes()), *v)),
    );
    let outline = font.outline_glyphs().get(GlyphId::new(glyph))?;
    let mut pen = ControlBox {
        b: [0.0; 4],
        any: false,
    };
    outline
        .draw(
            DrawSettings::unhinted(Size::unscaled(), &location)
                .with_path_style(PathStyle::HarfBuzz),
            &mut pen,
        )
        .ok()?;
    pen.any.then_some((pen.b, upem))
}

/// `gfx::RectF`: origin and size in `float`. Edges are recomputed
/// (`right() = x + width`), and `gfx::SizeF` clamps a width or height of at
/// most `8 × f32::EPSILON` to 0 (an empty rectangle).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

/// `gfx::SizeF::kTrivial`.
const SIZE_TRIVIAL: f32 = 8.0 * f32::EPSILON;

/// `gfx::SizeF::clamp`.
fn size_clamp(v: f32) -> f32 {
    if v > SIZE_TRIVIAL { v } else { 0.0 }
}

/// `gfx::SizeF::next`: the next `float` up, from at least `kTrivial`.
fn size_next(v: f32) -> f32 {
    next_up(v.max(SIZE_TRIVIAL))
}

/// `std::nextafter(v, +infinity)`, saturating at `f32::MAX` as
/// `gfx::BoundingRect` does.
fn next_up(v: f32) -> f32 {
    if v.is_nan() || v >= f32::MAX {
        return v.min(f32::MAX);
    }
    let bits = v.to_bits();
    f32::from_bits(if v == 0.0 {
        1
    } else if v > 0.0 {
        bits + 1
    } else {
        bits - 1
    })
}

impl RectF {
    /// `gfx::RectF(x, y, width, height)`.
    fn new(x: f32, y: f32, w: f32, h: f32) -> RectF {
        RectF {
            x,
            y,
            w: size_clamp(w),
            h: size_clamp(h),
        }
    }

    /// `gfx::SkRectToRectF`, which is `gfx::BoundingRect(left_top,
    /// right_bottom)`: when `left + (right - left)` is not exactly `right`,
    /// the width grows to the next `float` (likewise the height), whichever
    /// way the sum rounded.
    fn from_sk(l: f32, t: f32, r: f32, b: f32) -> RectF {
        let (left, right) = (l.min(r), l.max(r));
        let (top, bottom) = (t.min(b), t.max(b));
        let mut w = right - left;
        if left + w != right {
            w = next_up(w);
        }
        let mut h = bottom - top;
        if top + h != bottom {
            h = next_up(h);
        }
        RectF::new(left, top, w, h)
    }

    fn is_empty(&self) -> bool {
        self.w == 0.0 || self.h == 0.0
    }

    fn right(&self) -> f32 {
        self.x + self.w
    }

    fn bottom(&self) -> f32 {
        self.y + self.h
    }

    fn offset(&mut self, dx: f32, dy: f32) {
        self.x += dx;
        self.y += dy;
    }

    /// `RectF::Union` (`UnionEvenIfEmpty` for two non-empty rectangles): when
    /// `x + (r - x)` rounds below `r`, the width grows to the next `float`
    /// (likewise the height), so the union contains both rectangles.
    fn union(&mut self, o: &RectF) {
        if self.is_empty() {
            *self = *o;
            return;
        }
        if o.is_empty() {
            return;
        }
        let rx = self.x.min(o.x);
        let ry = self.y.min(o.y);
        let rr = self.right().max(o.right());
        let rb = self.bottom().max(o.bottom());
        *self = RectF::new(rx, ry, rr - rx, rb - ry);
        if self.right() < rr && self.w < f32::MAX {
            self.w = size_next(self.w);
        }
        if self.bottom() < rb && self.h < f32::MAX {
            self.h = size_next(self.h);
        }
    }
}

/// Blink's `GlyphBoundsAccumulator` for a horizontal run: edges kept as
/// `float` minima and maxima (the first non-empty glyph initializes them).
#[derive(Default)]
struct GlyphBounds {
    min_x: f32,
    max_x: f32,
    min_y: f32,
    max_y: f32,
}

impl GlyphBounds {
    /// `Unite(bounds_for_glyph, origin, glyph_offset)`.
    fn unite(&mut self, mut glyph: RectF, origin: f32, offset: (f32, f32)) {
        if glyph.is_empty() {
            return;
        }
        glyph.x += origin;
        glyph.offset(offset.0, offset.1);
        if self.min_x == 0.0 && self.max_x == 0.0 {
            self.min_x = glyph.x;
            self.max_x = glyph.right();
            self.min_y = glyph.y;
            self.max_y = glyph.bottom();
        } else {
            self.min_x = self.min_x.min(glyph.x);
            self.max_x = self.max_x.max(glyph.right());
            self.min_y = self.min_y.min(glyph.y);
            self.max_y = self.max_y.max(glyph.bottom());
        }
    }

    /// `BuildBounds`.
    fn build(&self) -> RectF {
        RectF::new(
            self.min_x,
            self.min_y,
            self.max_x - self.min_x,
            self.max_y - self.min_y,
        )
    }
}

impl<R: FontResolver> CompatMeasure<R> {
    /// Blink's `glyph_bounds` for a measured string (`TextMetrics::
    /// MeasureRuns`): per item, `ShapeResult::ComputeInkBounds` offset by the
    /// item's x and united in `float`. Within a run, each glyph's box is
    /// placed at its pen origin, an `InlineLayoutUnit` (16.16 fixed point)
    /// starting at the run's advance rounded up and advanced by the glyph
    /// advances exactly, converted to `float` per glyph.
    fn ink(&mut self, measured: &pm_text::blink::measure::Measured) -> RectF {
        let mut ink = RectF::default();
        let mut xpos = 0.0f32;
        for item in &measured.items {
            let mut item_ink = RectF::default();
            let mut run_advance = 0.0f32;
            let rtl = item.runs.iter().any(|r| r.rtl);
            let runs: Vec<&pm_text::blink::shaper::Run> = if rtl {
                item.runs.iter().rev().collect()
            } else {
                item.runs.iter().collect()
            };
            for run in runs {
                let mut bounds = GlyphBounds::default();
                // `InlineLayoutUnit::FromFloatCeil(run_advance)`.
                let mut origin = (run_advance * 65536.0).ceil() as i64;
                for g in &run.glyphs {
                    let glyph = if g.glyph == UNMATCHED_VS_GLYPH {
                        None
                    } else if let Some(b) = self.color_box(run.face, g.glyph, run.size, &run.coords)
                    {
                        Some(b)
                    } else {
                        self.glyph_box(run.face, g.glyph, &run.coords)
                            .map(|(b, upem)| {
                                let [l, t, r, bt] = skia_glyph_rect(b, upem, run.size);
                                RectF::from_sk(l, t, r, bt)
                            })
                    };
                    if let Some(glyph) = glyph {
                        let offset = (fixed_to_float(g.x_offset), -fixed_to_float(g.y_offset));
                        bounds.unite(glyph, origin as f32 / 65536.0, offset);
                    }
                    origin += i64::from(g.advance);
                }
                item_ink.union(&bounds.build());
                run_advance += run.width;
            }
            item_ink.offset(xpos, 0.0);
            ink.union(&item_ink);
            xpos += item.width;
        }
        ink
    }
}

/// `HarfBuzzPositionToFloat` (`SkFixedToFloat`).
fn fixed_to_float(v: i32) -> f32 {
    v as f32 / 65536.0
}

/// A glyph's ink box as Blink reports it on macOS
/// (`SkFontGetBoundsForGlyph`): the bounds of the glyph's path,
/// `SkFont::getPath(glyph).getBounds()`, in `float`, y down.
///
/// `SkFont::getPath` asks the strike at Skia's canonical path size
/// (`SkFontPriv::kCanonicalTextSizeForPaths`, 64 px) and scales the path by
/// `size / 64` in `float` (`SkFont::setupForAsPaths`). At 64 px CoreText
/// builds the path in `CGFloat` (`CTFontCreatePathForGlyph`), which
/// `SkScalerContext_Mac::generatePath` converts to `float` points (the 4x
/// unhinting scale in x and the y flip are exact). The path's bounds are the
/// bounds of its points, control points included: the outline's control box.
///
/// So each edge is `f32(f32(v × 64 / upem) × (size / 64))`. Scaling by the
/// size directly differs in the last bit at fractional sizes.
fn skia_glyph_rect(b: [f32; 4], upem: f32, size: f32) -> [f32; 4] {
    const CANONICAL: f32 = 64.0;
    let at_canonical = |v: f32| (f64::from(v) * f64::from(CANONICAL) / f64::from(upem)) as f32;
    let scale = size / CANONICAL;
    let map = |v: f32| at_canonical(v) * scale;
    // Font units are y up; the path is y down.
    [map(b[0]), -map(b[3]), map(b[2]), -map(b[1])]
}

/// HarfBuzz's glyph for an unmatched variation selector, which Blink drops.
const UNMATCHED_VS_GLYPH: u32 = pm_text::blink::shaper::UNMATCHED_VS_GLYPH;

impl<R: FontResolver> CompatMeasure<R> {
    /// Whether any face `d` lays out with carries variation coordinates
    /// (a variable font instance), whose ink boxes depend on variation
    /// arithmetic.
    pub fn uses_variations(&mut self, d: &TextContent) -> bool {
        let lines = self.compat.source_lines(d);
        let mut m = self.compat.measurer(d);
        lines.iter().any(|l| {
            m.shape(&l.text)
                .items
                .iter()
                .any(|item| item.runs.iter().any(|r| !r.coords.is_empty()))
        })
    }
}

impl<R: FontResolver> TextMeasure for CompatMeasure<R> {
    fn level2(
        &mut self,
        d: &TextContent,
        spans: &[pm_text::level2::StyleSpan],
    ) -> Result<crate::text_level2::Shaped, String> {
        level2::shape(&mut self.compat, &mut self.level2, d, spans)
    }
    fn level2_ink(
        &mut self,
        data: &pm_text::font::FontBytes,
        run: &pm_text::level2::Run,
        glyph: u32,
    ) -> Option<[f64; 4]> {
        let key = (
            data.id(),
            glyph,
            run.size.to_bits(),
            run.coords.clone(),
            run.synthetic_bold,
            run.skew.to_bits(),
        );
        if let Some(ink) = self.level2_boxes.get(&key) {
            self.level2_glyph_hits += 1;
            return *ink;
        }
        self.level2_glyph_misses += 1;
        let ink = level2_ink(data, run, glyph);
        let bytes = level2_glyph_entry_bytes(&key);
        self.level2_boxes.insert(key, ink, bytes);
        while self.level2_boxes.len() > LEVEL2_GLYPH_LIMIT_ENTRIES
            || self.level2_boxes.bytes() > LEVEL2_GLYPH_LIMIT_BYTES
        {
            if self.level2_boxes.pop_lru(|_| false).is_none() {
                break;
            }
        }
        ink
    }
    fn measure_lines(&mut self, d: &TextContent) -> Vec<MeasuredLine> {
        let lines = self.compat.source_lines(d);
        let shaped: Vec<(String, pm_text::blink::measure::Measured)> = {
            let mut m = self.compat.measurer(d);
            lines
                .iter()
                .map(|l| (String::from_utf16_lossy(&l.text), m.shape(&l.text)))
                .collect()
        };
        shaped
            .into_iter()
            .map(|(text, measured)| {
                let r = self.ink(&measured);
                let ink = [
                    f64::from(r.x),
                    f64::from(r.y),
                    f64::from(r.right()),
                    f64::from(r.bottom()),
                ];
                MeasuredLine {
                    text,
                    width: f64::from(measured.width),
                    ink,
                }
            })
            .collect()
    }

    fn plain_width(&mut self, d: &TextContent, text: &str) -> f64 {
        let mut plain = d.clone();
        plain.tracking = None;
        self.compat.measure(&plain, text)
    }

    fn spaced_width(&mut self, d: &TextContent, text: &str) -> f64 {
        self.compat.measure(d, text)
    }

    fn glyph_layout(&mut self, d: &TextContent) -> GlyphLayout {
        self.compat.glyph_layout(d)
    }
}
