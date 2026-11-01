//! Renderer-free, font-backed geometry for kernel commands and headless clients.
//!
//! Layout level is explicit: never upgrade saved level-1 text by measuring it
//! through level 2. Input typography must already be evaluated at the command's
//! time. Coordinates are unpadded layer space, y down, x from the alignment
//! origin. All source ranges/carets use UTF-16, as the extension contract does.
//! This module does not choose interaction padding, selection visuals or copy.

use crate::compat::{Align, Boundary, Compat, GlyphLayout, TextContent};
use crate::font::FontResolver;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}
impl Rect {
    pub fn width(self) -> f64 {
        self.x1 - self.x0
    }
    pub fn height(self) -> f64 {
        self.y1 - self.y0
    }
    pub fn center(self) -> [f64; 2] {
        [(self.x0 + self.x1) / 2.0, (self.y0 + self.y1) / 2.0]
    }
    pub fn union(self, other: Self) -> Self {
        Self {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayoutLevel {
    Compat1,
    Native2,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredLine {
    pub text: String,
    pub start: usize,
    /// End of the content; excludes an authored hard break.
    pub end: usize,
    pub bounds: Rect,
    pub baseline: f64,
    /// Font metrics, distinct from the line box's leading. Level 1's authored
    /// baseline/line height have no equivalent metric assertion here.
    pub font_ascent: Option<f64>,
    pub font_descent: Option<f64>,
    pub caret: Vec<Boundary>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum InkBounds {
    /// Layout only; callers must not mistake advance boxes for glyph ink.
    NotMeasured,
    Empty,
    Bounds(Rect),
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextMeasurement {
    pub level: LayoutLevel,
    /// Union of unpadded line boxes, including whitespace and blank lines.
    pub layout_bounds: Rect,
    /// Authored paragraph box, if present; independent of overflow/ink.
    pub paragraph_box: Option<Rect>,
    pub ink: InkBounds,
    pub lines: Vec<MeasuredLine>,
    /// Character/word/line units, shaped together for Split Text/animators.
    pub units: GlyphLayout,
    pub source_length: usize,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CaretAffinity {
    Upstream,
    Downstream,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaretGeometry {
    pub index: usize,
    pub line: usize,
    pub x: f64,
    pub top: f64,
    pub bottom: f64,
    pub baseline: f64,
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum MeasureError {
    #[error("non-finite or unrepresentable text value: {0}")]
    InvalidValue(&'static str),
    #[error("no fonts have been registered")]
    NoFonts,
    #[error("registered fonts do not cover {0} visible clusters")]
    MissingGlyphs(usize),
    #[error("font bytes missing for shaped run: {0}")]
    MissingFace(String),
    #[error("font outline could not be measured: {0}")]
    InvalidFont(String),
}

fn validate(d: &TextContent) -> Result<(), MeasureError> {
    for (key, value) in [
        ("size", d.size),
        ("weight", d.weight),
        ("tracking", d.tracking),
        ("leading", d.leading),
        ("boxWidth", d.box_width),
        ("boxHeight", d.box_height),
    ] {
        if value.is_some_and(|v| !v.is_finite() || !(v as f32).is_finite()) {
            return Err(MeasureError::InvalidValue(key));
        }
    }
    if d.axes
        .iter()
        .any(|(_, v)| !v.is_finite() || !(*v as f32).is_finite())
    {
        return Err(MeasureError::InvalidValue("fontAxis"));
    }
    Ok(())
}
fn paragraph_box(d: &TextContent, layout: Rect) -> Option<Rect> {
    let width = d
        .box_width
        .filter(|w| *w > 0.0 && d.paragraph.unwrap_or(false))?;
    let x = match d.align() {
        Align::Left => 0.0,
        Align::Center => -width / 2.0,
        Align::Right => -width,
    };
    Some(Rect {
        x0: x,
        y0: 0.0,
        x1: x + width,
        y1: d.box_height.filter(|h| *h > 0.0).unwrap_or(layout.height()),
    })
}
fn assemble(
    d: &TextContent,
    level: LayoutLevel,
    lines: Vec<MeasuredLine>,
    units: GlyphLayout,
) -> TextMeasurement {
    let layout_bounds = lines
        .iter()
        .map(|l| l.bounds)
        .reduce(Rect::union)
        .unwrap_or(Rect {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: 0.0,
        });
    TextMeasurement {
        level,
        layout_bounds,
        paragraph_box: paragraph_box(d, layout_bounds),
        ink: InkBounds::NotMeasured,
        lines,
        units,
        source_length: d.text.encode_utf16().count(),
    }
}

/// Existing compatibility semantics over a host-provided font resolver. The
/// resolver's readiness/fallback errors must also be checked by its owner
/// (`ReplayResolver::misses`, `ServedResolver::take_failures`). No system API
/// is called by layout; provide a ready resolver on the kernel worker.
pub fn measure_compat<R: FontResolver>(
    engine: &mut Compat<R>,
    d: &TextContent,
) -> Result<TextMeasurement, MeasureError> {
    validate(d)?;
    let caret = engine.caret_layout(d);
    let height = caret.line_height;
    let lines = caret
        .lines
        .into_iter()
        .map(|l| MeasuredLine {
            text: l.text.clone(),
            start: l.start,
            end: l.start + l.text.encode_utf16().count(),
            bounds: Rect {
                x0: l.x,
                y0: l.y,
                x1: l.x + l.width,
                y1: l.y + height,
            },
            baseline: l.baseline,
            font_ascent: None,
            font_descent: None,
            caret: l.boundaries,
        })
        .collect();
    Ok(assemble(
        d,
        LayoutLevel::Compat1,
        lines,
        engine.glyph_layout(d),
    ))
}

/// The boundary for a ready native layout service or Linux test double. The
/// kernel owns its scheduling and font generation; this trait has no platform,
/// renderer, async runtime, JavaScript or project writer requirement.
pub trait TextMeasure {
    fn measure_text(&mut self, content: &TextContent) -> Result<TextMeasurement, MeasureError>;
}
impl<R: FontResolver> TextMeasure for Compat<R> {
    fn measure_text(&mut self, content: &TextContent) -> Result<TextMeasurement, MeasureError> {
        measure_compat(self, content)
    }
}

#[cfg(feature = "level2")]
pub fn measure_level2(
    engine: &mut crate::level2::Level2,
    d: &TextContent,
    spans: &[crate::level2::StyleSpan],
) -> Result<TextMeasurement, MeasureError> {
    validate(d)?;
    for span in spans {
        validate(&span.content)?;
    }
    if engine.families().next().is_none() {
        return Err(MeasureError::NoFonts);
    }
    let layout = engine.layout_styled(d, spans);
    measurement_from_level2(d, &layout)
}
#[cfg(feature = "level2")]
fn measurement_from_level2(
    d: &TextContent,
    layout: &crate::level2::Level2Layout,
) -> Result<TextMeasurement, MeasureError> {
    if layout.missing_glyphs > 0 {
        return Err(MeasureError::MissingGlyphs(layout.missing_glyphs));
    }
    let lines = layout
        .lines
        .iter()
        .zip(&layout.caret)
        .map(|(l, caret)| MeasuredLine {
            text: l.text.clone(),
            start: l.start,
            end: l.end,
            bounds: Rect {
                x0: l.x,
                y0: l.y,
                x1: l.x + l.width,
                y1: l.y + l.height,
            },
            baseline: l.baseline,
            font_ascent: Some(l.ascent),
            font_descent: Some(l.descent),
            caret: caret.clone(),
        })
        .collect();
    Ok(assemble(
        d,
        LayoutLevel::Native2,
        lines,
        layout.units.clone(),
    ))
}
#[cfg(feature = "level2")]
impl TextMeasure for crate::level2::Level2 {
    fn measure_text(&mut self, content: &TextContent) -> Result<TextMeasurement, MeasureError> {
        measure_level2(self, content, &[])
    }
}

impl TextMeasurement {
    /// Every valid visual caret for this source offset. At a soft wrap there
    /// can be two (upstream end / downstream beginning). Never cuts a grapheme.
    pub fn carets(&self, index: usize) -> Vec<CaretGeometry> {
        self.lines
            .iter()
            .enumerate()
            .flat_map(|(row, line)| {
                line.caret
                    .iter()
                    .filter(move |c| c.index == index)
                    .map(move |c| CaretGeometry {
                        index: c.index,
                        line: row,
                        x: c.x,
                        top: line.bounds.y0,
                        bottom: line.bounds.y1,
                        baseline: line.baseline,
                    })
            })
            .collect()
    }
    pub fn caret(&self, index: usize, affinity: CaretAffinity) -> Option<CaretGeometry> {
        let positions = self.carets(index);
        match affinity {
            CaretAffinity::Upstream => positions.first().copied(),
            CaretAffinity::Downstream => positions.last().copied(),
        }
    }
    /// Closest legal caret in visual coordinates (including RTL). No chosen
    /// interaction radius or UI behavior; callers decide how to use this fact.
    pub fn nearest_caret(&self, x: f64, y: f64) -> Option<CaretGeometry> {
        if !x.is_finite() || !y.is_finite() {
            return None;
        }
        let (row, line) = self.lines.iter().enumerate().min_by(|(_, a), (_, b)| {
            let distance = |l: &MeasuredLine| {
                if y < l.bounds.y0 {
                    l.bounds.y0 - y
                } else if y > l.bounds.y1 {
                    y - l.bounds.y1
                } else {
                    0.0
                }
            };
            distance(a).total_cmp(&distance(b))
        })?;
        let c = line
            .caret
            .iter()
            .min_by(|a, b| (a.x - x).abs().total_cmp(&(b.x - x).abs()))?;
        Some(CaretGeometry {
            index: c.index,
            line: row,
            x: c.x,
            top: line.bounds.y0,
            bottom: line.bounds.y1,
            baseline: line.baseline,
        })
    }
}

/// Font-ink measurement needs only `ink`, never the `render` feature. Full font
/// faces (or outline-preserving subsets), keyed as in `Level2::add_font`, are
/// supplied by the owner of the font collection. Font collections should be
/// split to single-face SFNTs before registration when using this API.
#[cfg(all(feature = "level2", feature = "ink"))]
pub fn measure_level2_with_ink<'a>(
    engine: &mut crate::level2::Level2,
    d: &TextContent,
    spans: &[crate::level2::StyleSpan],
    mut font: impl FnMut(&str) -> Option<&'a crate::font::FontBytes>,
) -> Result<TextMeasurement, MeasureError> {
    validate(d)?;
    for span in spans {
        validate(&span.content)?;
    }
    if engine.families().next().is_none() {
        return Err(MeasureError::NoFonts);
    }
    let layout = engine.layout_styled(d, spans);
    let mut measurement = measurement_from_level2(d, &layout)?;
    let mut bounds: Option<Rect> = None;
    for run in &layout.runs {
        let data = font(&run.font).ok_or_else(|| MeasureError::MissingFace(run.font.clone()))?;
        for g in &run.glyphs {
            if let Some(b) = ink::glyph_ink(data, run, g.id)? {
                let b = Rect {
                    x0: b[0] + g.x,
                    y0: b[1] + g.y,
                    x1: b[2] + g.x,
                    y1: b[3] + g.y,
                };
                bounds = Some(bounds.map_or(b, |old| old.union(b)));
            }
        }
    }
    measurement.ink = bounds.map_or(InkBounds::Empty, InkBounds::Bounds);
    Ok(measurement)
}
#[cfg(all(feature = "level2", feature = "ink"))]
mod ink;
