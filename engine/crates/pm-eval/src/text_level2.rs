//! Native-only text semantics. One evaluated frame supplies the renderer,
//! selection bounds, picks and editor layout. Level 1 never enters here.

use crate::{
    anchor::{BoundsValues, Offset, font_anchor_offset},
    evaluator::Content,
    js_sem::{JsResult, JsThrow, JsValue},
    node,
    scope::Scope,
    text_geom::{Selection, TextMeasure, TextRaster, text_content},
    transform::{Mat2d, mul},
};
use pm_model::{JsString, Node};
use pm_text::{
    compat::{GlyphLayout, TextContent},
    font::FontBytes,
    level2::{Level2Layout, Run, StyleSpan},
};
use std::collections::HashMap;

pub fn is_level2(layer: &Node) -> bool {
    node::member(Some(layer), "compat").is_some_and(|v| node::to_number(Some(v)) >= 2.0)
}

pub struct Shaped {
    pub layout: Level2Layout,
    pub faces: HashMap<String, FontBytes>,
}

/// A shaped run (or atom within it) with its evaluated layer-space pose.
pub struct Draw {
    pub run: Run,
    pub data: FontBytes,
    pub matrix: Mat2d,
    pub opacity: f64,
    pub blur: f64,
    pub color: JsValue,
}

pub struct Frame {
    pub layout: Level2Layout,
    pub draws: Vec<Draw>,
    /// The selection box that bounds and picks use: the resting styled
    /// text, so it neither collapses while animators hide every glyph nor
    /// jumps as they settle (today's animated-text selection). With range
    /// styles it also grows over the posed visible ink, as today's styled
    /// path selects. Typography anchoring is applied.
    pub selection: Selection,
    /// The resting styled ink box with typography anchoring applied: what
    /// the anchor measures and what anchor edits start from.
    pub resting: Selection,
    /// The posed visible ink (blur reach included, hidden glyphs left out)
    /// with typography anchoring applied; `None` while nothing is visible.
    pub ink: Option<Selection>,
    pub offset: Offset,
    pub size: f64,
}

impl Frame {
    pub fn caret_layout(&self, content: &TextContent) -> pm_text::compat::CaretLayout {
        pm_text::compat::CaretLayout {
            lines: self
                .layout
                .lines
                .iter()
                .zip(&self.layout.caret)
                .map(|(line, caret)| pm_text::compat::CaretLine {
                    text: line.text.clone(),
                    start: line.start,
                    x: line.x,
                    y: line.y,
                    baseline: line.baseline,
                    width: line.width,
                    boundaries: caret.clone(),
                })
                .collect(),
            line_height: self
                .layout
                .lines
                .first()
                .map_or(content.line_height(), |l| l.height),
            size: content.size(),
            length: content.text.encode_utf16().count(),
            align: content.align(),
            box_width: content.box_width.unwrap_or(0.0),
            box_height: content.box_height.unwrap_or(0.0),
        }
    }
    /// The compatibility raster covers the posed ink and the selection, so
    /// moved glyphs are never clipped and the canvas never collapses.
    pub fn raster(&self) -> TextRaster {
        let s = self.selection;
        let r = self.ink.map_or(s, |ink| union(&s, &ink));
        TextRaster {
            w: r.w.max(1.0),
            h: r.h.max(1.0),
            anchor_x: -r.x0,
            anchor_y: -r.y0,
            selection: s,
        }
    }
}

const IDENTITY: Mat2d = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
fn translate(x: f64, y: f64) -> Mat2d {
    [1.0, 0.0, 0.0, 1.0, x, y]
}
fn get<'a>(d: &'a Content, key: &str) -> &'a JsValue {
    static UNDEFINED: JsValue = JsValue::Undefined;
    d.get(&JsString::from(key)).unwrap_or(&UNDEFINED)
}

/// Resolve spans at grapheme boundaries; the last authored style wins for
/// each property. Includes whitespace, which participates in line breaking.
fn spans(d: &Content, state: &crate::text_anim::TextAnimation) -> Vec<StyleSpan> {
    if state.styles.is_empty() {
        return Vec::new();
    }
    let text = text_content(d).text;
    let units: Vec<_> = text.encode_utf16().collect();
    let mut spans: Vec<StyleSpan> = Vec::new();
    for r in pm_text::compat::graphemes(&units) {
        let mut content = d.clone();
        for style in &state.styles {
            let start = get(&style.item, "start").to_number();
            let end = get(&style.item, "end").to_number();
            if r.start as f64 >= start && (r.start as f64) < end {
                content.extend(style.p.clone());
            }
        }
        let content = text_content(&content);
        if let Some(last) = spans.last_mut()
            && last.content == content
        {
            last.end = r.end;
        } else {
            spans.push(StyleSpan {
                start: r.start,
                end: r.end,
                content,
            });
        }
    }
    spans
}

fn color_at(d: &Content, state: &crate::text_anim::TextAnimation, source: usize) -> JsValue {
    let mut color = get(d, "color").clone();
    for s in &state.styles {
        if source as f64 >= get(&s.item, "start").to_number()
            && (source as f64) < get(&s.item, "end").to_number()
            && let Some(c) = s.p.get(&JsString::from("color"))
        {
            color = c.clone();
        }
    }
    if color.truthy() {
        color
    } else {
        JsValue::from("#fff")
    }
}

impl<'d> Scope<'_, 'd> {
    pub fn level2_text(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
        animate: bool,
        anchor: bool,
    ) -> JsResult<Frame> {
        let d = self.resolved_text_content(layer, time)?;
        let tc: TextContent = text_content(&d);
        let controls = self.text_animation_state(layer, time, &GlyphLayout::default())?;
        let shaped = measure
            .level2(&tc, &spans(&d, &controls))
            .map_err(|why| JsThrow::type_error(&why))?;
        let layout = shaped.layout;
        let mut state = self.text_animation_state(layer, time, &layout.units)?;
        if !animate {
            state.animators.clear();
        }
        let poses = self.level2_glyphs(&d, &layout.units, &state);
        let poses: HashMap<_, _> = poses.iter().map(|g| (g.unit.source_start, g)).collect();
        let units: HashMap<_, _> = layout
            .units
            .characters
            .iter()
            .map(|g| (g.source_start, g))
            .collect();
        let mut draws = Vec::new();
        let posed = !state.animators.is_empty() || !state.styles.is_empty();
        for run in &layout.runs {
            let data = shaped
                .faces
                .get(&run.font)
                .ok_or_else(|| JsThrow::type_error("level 2 font bytes unavailable"))?;
            if !posed {
                draws.push(Draw {
                    run: run.clone(),
                    data: data.clone(),
                    matrix: IDENTITY,
                    opacity: 1.0,
                    blur: 0.0,
                    color: color_at(&d, &state, 0),
                });
                continue;
            }
            for (i, glyph) in run.glyphs.iter().enumerate() {
                let source = run
                    .sources
                    .get(i)
                    .copied()
                    .ok_or_else(|| JsThrow::type_error("level 2 glyph source unavailable"))?;
                let pose = poses.get(&source);
                let unit = units.get(&source);
                let (matrix, opacity, blur, color) = match (pose, unit) {
                    (Some(p), Some(u)) => {
                        let baseline = layout.lines[run.line].baseline;
                        let matrix = if p.matrix[..4] == IDENTITY[..4] {
                            // Do not subtract and re-add the baseline for a
                            // translation: cancellation would perturb an
                            // otherwise exact integer selector movement.
                            translate(p.unit.x - u.x + p.matrix[4], p.matrix[5])
                        } else {
                            mul(
                                &mul(&translate(p.unit.x, baseline), &p.matrix),
                                &translate(-u.x, -baseline),
                            )
                        };
                        (
                            matrix,
                            p.opacity,
                            p.blur,
                            p.color
                                .as_ref()
                                .map(|s| JsValue::String(s.clone()))
                                .unwrap_or_else(|| color_at(&d, &state, source)),
                        )
                    }
                    _ => (IDENTITY, 1.0, 0.0, color_at(&d, &state, source)),
                };
                let atom = Run {
                    line: run.line,
                    font: run.font.clone(),
                    size: run.size,
                    coords: run.coords.clone(),
                    synthetic_bold: run.synthetic_bold,
                    skew: run.skew,
                    glyphs: vec![*glyph],
                    sources: vec![source],
                };
                if let Some(last) = draws.last_mut()
                    && last.run.font == atom.font
                    && last.run.line == atom.line
                    && last.run.size == atom.size
                    && last.run.coords == atom.coords
                    && last.run.skew == atom.skew
                    && last.run.synthetic_bold == atom.synthetic_bold
                    && last.matrix == matrix
                    && last.opacity == opacity
                    && last.blur == blur
                    && last.color == color
                {
                    // One paint operation for equal poses retains the
                    // shaped run's coverage/compositing behavior at rest.
                    last.run.glyphs.extend(atom.glyphs);
                    last.run.sources.extend(atom.sources);
                } else {
                    draws.push(Draw {
                        run: atom,
                        data: data.clone(),
                        matrix,
                        opacity,
                        blur,
                        color,
                    });
                }
            }
        }
        let empty = Selection {
            x0: 0.0,
            y0: 0.0,
            x1: 0.0,
            y1: layout.height,
            w: 0.0,
            h: layout.height,
        };
        let mut resting = bounds(&draws, true).unwrap_or(empty);
        let mut ink = bounds(&draws, false);
        let mut selection = match (&ink, state.styles.is_empty()) {
            (Some(ink), false) => union(&resting, ink),
            _ => resting,
        };
        let mut offset = Offset::default();
        if anchor
            && let Some(reference) = BoundsValues::of(node::member(
                node::member(Some(layer), "d"),
                "fontAnchorBounds",
            ))
        {
            // Typography anchoring measures the resting styled text. An
            // animator's movement must never be canceled by the anchor.
            offset = font_anchor_offset(
                Some(&reference),
                Some(&BoundsValues::from(resting.rect())),
                &self.channel(layer, "anchor.x", time)?,
                &self.channel(layer, "anchor.y", time)?,
            );
            for b in [&mut selection, &mut resting]
                .into_iter()
                .chain(ink.as_mut())
            {
                b.x0 += offset.x;
                b.x1 += offset.x;
                b.y0 += offset.y;
                b.y1 += offset.y;
            }
        }
        Ok(Frame {
            layout,
            draws,
            selection,
            resting,
            ink,
            offset,
            size: tc.size(),
        })
    }
}

/// Transformed outline control boxes, including blur reach (`resting`
/// measures the unposed styled ink instead, hidden glyphs included). `None`
/// when no glyph has ink: empty or whitespace-only text, or every glyph
/// hidden by an animator.
fn bounds(draws: &[Draw], resting: bool) -> Option<Selection> {
    let mut b = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    for draw in draws {
        if !resting && (!draw.opacity.is_finite() || draw.opacity <= 0.0) {
            continue;
        }
        for glyph in &draw.run.glyphs {
            let Some(ink) = crate::text_measure::level2_ink(&draw.data, &draw.run, glyph.id) else {
                continue;
            };
            let m = if resting { IDENTITY } else { draw.matrix };
            let pad = if resting {
                0.0
            } else {
                draw.blur.max(0.0) * 3.0
            };
            for (x, y) in [
                (ink[0], ink[1]),
                (ink[2], ink[1]),
                (ink[0], ink[3]),
                (ink[2], ink[3]),
            ] {
                let (x, y) = (x + glyph.x, y + glyph.y);
                let (x, y) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
                b[0] = b[0].min(x - pad);
                b[1] = b[1].min(y - pad);
                b[2] = b[2].max(x + pad);
                b[3] = b[3].max(y + pad);
            }
        }
    }
    b.iter().all(|v| v.is_finite()).then(|| rect(b))
}

fn rect(b: [f64; 4]) -> Selection {
    Selection {
        x0: b[0],
        y0: b[1],
        x1: b[2],
        y1: b[3],
        w: b[2] - b[0],
        h: b[3] - b[1],
    }
}

fn union(a: &Selection, b: &Selection) -> Selection {
    rect([
        a.x0.min(b.x0),
        a.y0.min(b.y0),
        a.x1.max(b.x1),
        a.y1.max(b.y1),
    ])
}
