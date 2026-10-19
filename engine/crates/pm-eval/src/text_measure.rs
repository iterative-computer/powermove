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
//! Outlines come from the face's own bytes (`FaceInfo::data`). The
//! metrics-and-shaping subsets `pm-wasm` receives have no outline tables, so
//! glyphs without an outline (subsets, bitmap emoji) contribute no ink there;
//! see `docs/engine/pm-eval-layers.md`.

use std::collections::HashMap;

use pm_text::compat::{Compat, GlyphLayout, TextContent};
use pm_text::font::{FaceId, FontResolver};
use skrifa::instance::{Location, Size};
use skrifa::outline::{DrawSettings, OutlinePen, pen::PathStyle};
use skrifa::raw::TableProvider;
use skrifa::{FontRef, GlyphId, MetadataProvider, Tag};

use crate::text_geom::{MeasuredLine, TextMeasure};

mod level2;

/// Level 2 uses font-metric baselines and normalized Parley coordinates.
/// Keep outline geometry independent of platform rasterization.
pub(crate) fn level2_ink(
    data: &pm_text::font::FontBytes,
    run: &pm_text::level2::Run,
    glyph: u32,
) -> Option<[f64; 4]> {
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
        // Color bitmap glyphs have no outline; their advance and the font's
        // ascent/descent form their typographic box. Spaces have no ink.
        let metrics = font.glyph_metrics(Size::new(run.size), coords.as_slice());
        let advance = metrics.advance_width(GlyphId::new(glyph))?;
        let metrics = font.metrics(Size::new(run.size), coords.as_slice());
        if font.charmap().map(' ') == Some(GlyphId::new(glyph)) {
            return None;
        }
        return Some([
            0.0,
            -f64::from(metrics.ascent),
            f64::from(advance),
            -f64::from(metrics.descent),
        ]);
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
    let skew = -libm::tan(f64::from(run.skew) * core::f64::consts::PI / 180.0);
    let (a, b) = (skew * ink[1], skew * ink[3]);
    ink[0] += a.min(b);
    ink[2] += a.max(b);
    Some(ink)
}

/// A glyph's control box in font units, and the face's units per em.
type GlyphBox = Option<([f32; 4], f32)>;

/// A face, glyph, and variation coordinates (tag, value bits).
type BoxKey = (FaceId, u32, Vec<(u32, u32)>);

/// Level 1 text measurement with ink boxes from outlines.
pub struct CompatMeasure<R: FontResolver> {
    compat: Compat<R>,
    boxes: HashMap<BoxKey, GlyphBox>,
    level2_faces: HashMap<(u64, u32), pm_text::font::FontBytes>,
}

impl<R: FontResolver> CompatMeasure<R> {
    pub fn new(resolver: R) -> Self {
        CompatMeasure {
            compat: Compat::new(resolver),
            boxes: HashMap::new(),
            level2_faces: HashMap::new(),
        }
    }

    pub fn compat(&mut self) -> &mut Compat<R> {
        &mut self.compat
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
                    if g.glyph != UNMATCHED_VS_GLYPH
                        && let Some((b, upem)) = self.glyph_box(run.face, g.glyph, &run.coords)
                    {
                        let [l, t, r, bt] = skia_glyph_rect(b, upem, run.size);
                        let offset = (fixed_to_float(g.x_offset), -fixed_to_float(g.y_offset));
                        bounds.unite(RectF::from_sk(l, t, r, bt), origin as f32 / 65536.0, offset);
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
        level2::shape(&mut self.compat, &mut self.level2_faces, d, spans)
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
