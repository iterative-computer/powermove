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

/// A glyph's control box in font units, and the face's units per em.
type GlyphBox = Option<([f32; 4], f32)>;

/// A face, glyph, and variation coordinates (tag, value bits).
type BoxKey = (FaceId, u32, Vec<(u32, u32)>);

/// Level 1 text measurement with ink boxes from outlines.
pub struct CompatMeasure<R: FontResolver> {
    compat: Compat<R>,
    boxes: HashMap<BoxKey, GlyphBox>,
}

impl<R: FontResolver> CompatMeasure<R> {
    pub fn new(resolver: R) -> Self {
        CompatMeasure {
            compat: Compat::new(resolver),
            boxes: HashMap::new(),
        }
    }

    pub fn compat(&mut self) -> &mut Compat<R> {
        &mut self.compat
    }

    fn glyph_box(&mut self, face: FaceId, glyph: u32, coords: &[(u32, f32)]) -> GlyphBox {
        let key = (face, glyph, coords.iter().map(|(t, v)| (*t, v.to_bits())).collect());
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
    let location: Location = font
        .axes()
        .location(coords.iter().map(|(t, v)| (Tag::from_be_bytes(t.to_be_bytes()), *v)));
    let outline = font.outline_glyphs().get(GlyphId::new(glyph))?;
    let mut pen = ControlBox {
        b: [0.0; 4],
        any: false,
    };
    outline
        .draw(
            DrawSettings::unhinted(Size::unscaled(), &location).with_path_style(PathStyle::HarfBuzz),
            &mut pen,
        )
        .ok()?;
    pen.any.then_some((pen.b, upem))
}

/// `gfx::RectF`: origin and size in `float`; edges are recomputed
/// (`right() = x + width`), which Blink's ink bounds inherit.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RectF {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl RectF {
    /// `gfx::SkRectToRectF`.
    fn from_sk(l: f32, t: f32, r: f32, b: f32) -> RectF {
        RectF { x: l, y: t, w: (r - l).max(0.0), h: (b - t).max(0.0) }
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

    /// `RectF::Union`.
    fn union(&mut self, o: &RectF) {
        if self.is_empty() {
            *self = *o;
            return;
        }
        if o.is_empty() {
            return;
        }
        let l = self.x.min(o.x);
        let t = self.y.min(o.y);
        let r = self.right().max(o.right());
        let b = self.bottom().max(o.bottom());
        *self = RectF { x: l, y: t, w: (r - l).max(0.0), h: (b - t).max(0.0) };
    }
}

impl<R: FontResolver> CompatMeasure<R> {
    /// Blink's `glyph_bounds` for a measured string (`CachingWordShaper::
    /// Width` over `ShapeResult::ComputeInkBounds`): every glyph's box
    /// offset by its pen position, accumulated in `float` per word and run,
    /// with advances and offsets in 16.16 fixed point.
    fn ink(&mut self, measured: &pm_text::blink::measure::Measured) -> RectF {
        let mut ink = RectF::default();
        let mut width = 0.0f32;
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
                let mut run_ink = RectF::default();
                let mut origin = run_advance;
                for g in &run.glyphs {
                    let advance = g.advance as f32 / 65536.0;
                    if g.glyph != UNMATCHED_VS_GLYPH
                        && let Some((b, upem)) = self.glyph_box(run.face, g.glyph, &run.coords)
                    {
                        let scale = f64::from(run.size) / f64::from(upem);
                        let [l, t, r, bt] = skia_glyph_rect(b, scale);
                        let mut rect = RectF::from_sk(l, t, r, bt);
                        if !rect.is_empty() {
                            rect.offset(origin + g.x_offset as f32 / 65536.0, -(g.y_offset as f32 / 65536.0));
                            run_ink.union(&rect);
                        }
                    }
                    origin += advance;
                }
                item_ink.union(&run_ink);
                run_advance += run.width;
            }
            item_ink.offset(width, 0.0);
            ink.union(&item_ink);
            width += item.width;
        }
        ink
    }
}

/// A glyph's bounds as Skia reports them on macOS: CoreText's bounding
/// rectangle (origin and size in `CGFloat`, y up) converted with
/// `SkRect::MakeXYWH(x, -y - h, w, h)` in `float`.
fn skia_glyph_rect(b: [f32; 4], scale: f64) -> [f32; 4] {
    // CoreText scales outline points by a `float` factor and reports the
    // box as a `CGRect` (`double`): origin and size.
    let s = scale as f32;
    let (x0, y0, x1, y1) = (b[0] * s, b[1] * s, b[2] * s, b[3] * s);
    let x = f64::from(x0);
    let y = f64::from(y0);
    let w = f64::from(x1) - x;
    let h = f64::from(y1) - y;
    // `SkRect::MakeXYWH(x, -y - h, w, h)` with `float` members.
    let left = x as f32;
    let top = (-y - h) as f32;
    [left, top, left + w as f32, top + h as f32]
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
                let ink = [f64::from(r.x), f64::from(r.y), f64::from(r.right()), f64::from(r.bottom())];
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

    fn glyph_layout(&mut self, d: &TextContent) -> GlyphLayout {
        self.compat.glyph_layout(d)
    }
}
