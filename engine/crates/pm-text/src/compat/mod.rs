//! Level 1 (`compat`) layout: today's text layout reproduced exactly.
//!
//! - Wrapping and glyph units are `legacy/core/text-layout.ts`
//!   (`textSourceLines`, `glyphLayout`), ported line for line with
//!   JavaScript's UTF-16 semantics (`wrap`).
//! - Every width is `CanvasRenderingContext2D.measureText` as Chromium 152
//!   computes it for raster.ts's `fontStr(d)` context: the Rust port of
//!   Blink's `measureText` over HarfRust (`canvas`, `crate::blink`),
//!   including CoreText `trak` tracking and optical sizing.
//! - Caret geometry is raster.ts `textCaretLayout`; baselines sit at
//!   `row × lineHeight + size × 0.82`, line height `size × (leading || 1.15)`.
//! - Glyph runs place glyphs the way raster.ts `rasterText` draws a line with
//!   `fillText` (`line_runs`), for the renderer.
//!
//! Gate (plan section 12): identical line breaks and advances within
//! 0.25 px of Canvas2D on the S4 corpus (`tests/corpus.rs`).

pub mod canvas;
pub mod content;
pub mod wrap;

pub use canvas::CanvasMeasurer;
pub use content::{Align, TextContent};
pub use wrap::{
    GlyphLayout, Measure, SourceLine, Unit, glyph_layout, graphemes, text_source_lines,
};

pub use crate::blink::measure::FontCacheSeed;
use crate::blink::shaper::UNMATCHED_VS_GLYPH;
use crate::fallback_cache::FallbackCached;
use crate::font::{FaceId, FontResolver, PreparedCache};
use serde::{Deserialize, Serialize};

/// The first baseline sits `size × BASELINE` below the line top.
pub const BASELINE: f64 = 0.82;

/// One caret boundary: a UTF-16 source index and its x in layer space.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Boundary {
    pub index: usize,
    pub x: f64,
}

/// One wrapped line as the on-canvas text editor sees it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaretLine {
    pub text: String,
    pub start: usize,
    pub x: f64,
    pub y: f64,
    pub baseline: f64,
    pub width: f64,
    pub boundaries: Vec<Boundary>,
}

/// raster.ts `textCaretLayout(d)`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaretLayout {
    pub lines: Vec<CaretLine>,
    pub line_height: f64,
    pub size: f64,
    /// UTF-16 length of the text.
    pub length: usize,
    pub align: Align,
    pub box_width: f64,
    pub box_height: f64,
}

/// A glyph placed in layer space (x from the alignment origin, y down from
/// the first line's top).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlacedGlyph {
    pub id: u32,
    pub x: f64,
    pub y: f64,
}

/// Glyphs of one face at one size and location.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlyphRun {
    pub face: FaceId,
    pub size: f32,
    /// User-space variation coordinates (tag, value).
    pub coords: Vec<(u32, f32)>,
    pub synthetic_bold: bool,
    pub synthetic_italic: bool,
    pub glyphs: Vec<PlacedGlyph>,
}

/// One line as `fillText` draws it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineRuns {
    pub text: String,
    pub start: usize,
    /// Pen start after alignment.
    pub x: f64,
    pub baseline: f64,
    pub width: f64,
    pub runs: Vec<GlyphRun>,
}

/// The level 1 layout engine over a font resolver. It owns Blink's
/// character fallback cache and the prepared-face cache, so measurements
/// depend on earlier ones exactly as they do in one Chromium renderer.
pub struct Compat<R> {
    resolver: FallbackCached<R>,
    cache: PreparedCache,
    seeds: Vec<FontCacheSeed>,
}

impl<R: FontResolver> Compat<R> {
    pub fn new(resolver: R) -> Self {
        Compat {
            resolver: FallbackCached::new(resolver),
            cache: PreparedCache::default(),
            seeds: Vec::new(),
        }
    }

    /// Declares the platform data Blink's font cache already holds in the
    /// renderer being reproduced (see [`FontCacheSeed`]). Empty by default:
    /// a fresh renderer.
    pub fn set_font_cache_seeds(&mut self, seeds: Vec<FontCacheSeed>) {
        self.seeds = seeds;
    }

    pub fn font_cache_seeds(&self) -> &[FontCacheSeed] {
        &self.seeds
    }

    pub fn resolver(&self) -> &R {
        &self.resolver.inner
    }

    pub fn resolver_mut(&mut self) -> &mut R {
        &mut self.resolver.inner
    }

    pub fn into_resolver(self) -> R {
        self.resolver.inner
    }

    /// Starts a fresh measuring session: forgets Blink's ideograph fallback
    /// choices (a renderer process restart).
    pub fn reset_session(&mut self) {
        self.resolver.reset();
    }

    /// A measuring context configured for `d`.
    pub fn measurer(&mut self, d: &TextContent) -> CanvasMeasurer<'_> {
        CanvasMeasurer::new(&mut self.resolver, &mut self.cache, d, &self.seeds)
    }

    /// `measureText(text).width` in `d`'s context.
    pub fn measure(&mut self, d: &TextContent, text: &str) -> f64 {
        let units: Vec<u16> = text.encode_utf16().collect();
        self.measurer(d).width(&units)
    }

    /// `textSourceLines(d, meas, lineHeight)`.
    pub fn source_lines(&mut self, d: &TextContent) -> Vec<SourceLine> {
        let lh = d.line_height();
        text_source_lines(d, &mut self.measurer(d), lh)
    }

    /// raster.ts `textLayout(d)` (`glyphLayout` on the layer's context).
    pub fn glyph_layout(&mut self, d: &TextContent) -> GlyphLayout {
        glyph_layout(d, &mut self.measurer(d))
    }

    /// raster.ts `textCaretLayout(d)`.
    pub fn caret_layout(&mut self, d: &TextContent) -> CaretLayout {
        let size = d.size();
        let lh = d.line_height();
        let align = d.align();
        let mut m = self.measurer(d);
        let lines = text_source_lines(d, &mut m, lh)
            .into_iter()
            .enumerate()
            .map(|(row, line)| {
                let width = m.width(&line.text);
                let start_x = aligned_start(align, width);
                let mut boundaries = vec![Boundary {
                    index: line.start,
                    x: start_x,
                }];
                for r in graphemes(&line.text) {
                    boundaries.push(Boundary {
                        index: line.start + r.end,
                        x: start_x + m.width(&line.text[..r.end]),
                    });
                }
                let y = row as f64 * lh;
                CaretLine {
                    text: String::from_utf16_lossy(&line.text),
                    start: line.start,
                    x: start_x,
                    y,
                    baseline: y + size * BASELINE,
                    width,
                    boundaries,
                }
            })
            .collect();
        CaretLayout {
            lines,
            line_height: lh,
            size,
            length: d.text.encode_utf16().count(),
            align,
            box_width: d.box_width.filter(|w| !w.is_nan()).unwrap_or(0.0),
            box_height: d.box_height.filter(|h| !h.is_nan()).unwrap_or(0.0),
        }
    }

    /// The glyphs `rasterText` paints: each wrapped line drawn with
    /// `fillText` at the aligned x and baseline `row × lh + size × 0.82`,
    /// pens advanced by the shaped advances (letter spacing included).
    pub fn line_runs(&mut self, d: &TextContent) -> Vec<LineRuns> {
        let size = d.size();
        let lh = d.line_height();
        let align = d.align();
        let mut m = self.measurer(d);
        let lines = text_source_lines(d, &mut m, lh);
        let mut out = Vec::with_capacity(lines.len());
        for (row, line) in lines.iter().enumerate() {
            let measured = m.shape(&line.text);
            let width = measured.width as f64;
            let x = aligned_start(align, width);
            let baseline = row as f64 * lh + size * BASELINE;
            let runs = placed_runs(&m, &measured, x, baseline);
            out.push(LineRuns {
                text: String::from_utf16_lossy(&line.text),
                start: line.start,
                x,
                baseline,
                width,
                runs,
            });
        }
        out
    }
}

impl<R: FontResolver> Compat<R> {
    /// The glyphs `fillText(text, 0, 0)` paints in `d`'s context with
    /// `textAlign = 'left'` and `letterSpacing = tracking px` (`tracking`
    /// overrides `d.tracking`): no wrapping, pens from 0 on the baseline
    /// at y = 0. Returns the runs and the measured width.
    pub fn text_runs(&mut self, d: &TextContent, text: &str, tracking: f64) -> (Vec<GlyphRun>, f64) {
        let mut ctx = d.clone();
        ctx.tracking = Some(tracking);
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut m = self.measurer(&ctx);
        let measured = m.shape(&units);
        let runs = placed_runs(&m, &measured, 0.0, 0.0);
        (runs, measured.width as f64)
    }
}

/// Places a shaped string's glyphs from pen `x` on `baseline`.
fn placed_runs(
    m: &CanvasMeasurer<'_>,
    measured: &crate::blink::measure::Measured,
    x: f64,
    baseline: f64,
) -> Vec<GlyphRun> {
    let mut pen = x;
    let mut runs = Vec::new();
    for item in &measured.items {
        // Runs are in logical order; an RTL item's runs read right to left.
        let ordered: Vec<_> = if item.runs.iter().any(|r| r.rtl) {
            item.runs.iter().rev().collect()
        } else {
            item.runs.iter().collect()
        };
        for run in ordered {
            let info = m.resolver.face(run.face);
            let mut glyphs = Vec::with_capacity(run.glyphs.len());
            for g in &run.glyphs {
                if g.glyph != UNMATCHED_VS_GLYPH {
                    glyphs.push(PlacedGlyph {
                        id: g.glyph,
                        x: pen + g.x_offset as f64 / 65536.0,
                        y: baseline - g.y_offset as f64 / 65536.0,
                    });
                }
                pen += g.advance as f64 / 65536.0;
            }
            runs.push(GlyphRun {
                face: run.face,
                size: run.size,
                coords: run.coords.clone(),
                synthetic_bold: info.synthetic_bold,
                synthetic_italic: info.synthetic_italic,
                glyphs,
            });
        }
    }
    runs
}

fn aligned_start(align: Align, width: f64) -> f64 {
    match align {
        Align::Center => -width / 2.0,
        Align::Right => -width,
        Align::Left => 0.0,
    }
}
