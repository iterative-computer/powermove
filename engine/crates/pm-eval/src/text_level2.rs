//! Native-only text semantics. One evaluated frame supplies the renderer,
//! selection bounds, picks and editor layout. Level 1 never enters here.

use crate::{
    anchor::{BoundsValues, Offset, font_anchor_offset},
    evaluator::Content,
    js_sem::{JsResult, JsThrow, JsValue},
    node,
    scope::Scope,
    text_anim::{StyleState, TextAnimation},
    text_geom::{Selection, TextMeasure, TextRaster, text_content},
    transform::{Mat2d, mul},
};
use pm_model::{JsString, Node};
use pm_text::{
    compat::TextContent,
    font::FontBytes,
    level2::{Level2Layout, Run, StyleSpan},
};
use std::collections::{HashMap, HashSet};

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
    /// The padded resting interaction box that bounds and picks use. An
    /// authored paragraph keeps its full box, even where it has no ink.
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
fn spans(d: &Content, styles: &[StyleState]) -> Vec<StyleSpan> {
    if styles.is_empty() {
        return Vec::new();
    }
    let text = text_content(d).text;
    let units: Vec<_> = text.encode_utf16().collect();
    let mut spans: Vec<StyleSpan> = Vec::new();
    for r in pm_text::compat::graphemes(&units) {
        let mut content = d.clone();
        for style in styles {
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

fn color_at(d: &Content, styles: &[StyleState], source: usize) -> JsValue {
    let mut color = get(d, "color").clone();
    for s in styles {
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
        // Controls are evaluated once: the range styles shape the text, and
        // the selector weights follow its units.
        let controls = self.text_controls(layer, time)?;
        let shaped = measure
            .level2(&tc, &spans(&d, &controls.styles))
            .map_err(|why| JsThrow::type_error(&why))?;
        let layout = shaped.layout;
        let state = if animate {
            self.text_animation_weights(layer, time, controls, &layout.units)
        } else {
            TextAnimation {
                animators: Vec::new(),
                styles: controls.styles,
            }
        };
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
                    color: color_at(&d, &state.styles, 0),
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
                                .unwrap_or_else(|| color_at(&d, &state.styles, source)),
                        )
                    }
                    _ => (IDENTITY, 1.0, 0.0, color_at(&d, &state.styles, source)),
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
        // Blank text has no ink: its box is the interaction box alone.
        let (rest, mut ink) = if tc.text.trim().is_empty() {
            (None, None)
        } else {
            bounds(&draws, &tc.text, measure)
        };
        let mut resting = rest.unwrap_or(empty);
        let mut selection = resting_selection(&tc, &layout, rest);
        if !state.styles.is_empty()
            && let Some(posed) = ink
        {
            selection = union(&selection, &posed);
        }
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

/// The resting styled ink box (no pose, hidden glyphs included) and the
/// posed visible ink (transformed outline control boxes with blur reach),
/// from one glyph box per glyph. `None` when no glyph has ink: blank text,
/// or (posed) every glyph hidden by an animator. Glyphs of whitespace,
/// controls and default-ignorables never have ink, even when a font gives
/// them no outline but an advance.
fn bounds(
    draws: &[Draw],
    text: &str,
    measure: &mut dyn TextMeasure,
) -> (Option<Selection>, Option<Selection>) {
    let units: Vec<u16> = text.encode_utf16().collect();
    let inkless: HashSet<usize> = pm_text::compat::graphemes(&units)
        .into_iter()
        .filter(|r| {
            char::decode_utf16(units[r.clone()].iter().copied()).all(|c| {
                c.is_ok_and(|ch| {
                    ch.is_whitespace()
                        || ch.is_control()
                        || pm_text::blink::chars::is_default_ignorable(ch as u32)
                })
            })
        })
        .map(|r| r.start)
        .collect();
    let mut resting_box = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut posed_box = resting_box;
    for draw in draws {
        let visible = draw.opacity.is_finite() && draw.opacity > 0.0;
        let m = draw.matrix;
        let pad = draw.blur.max(0.0) * 3.0;
        for (i, glyph) in draw.run.glyphs.iter().enumerate() {
            if draw.run.sources.get(i).is_some_and(|s| inkless.contains(s)) {
                continue;
            }
            let Some(ink) = measure.level2_ink(&draw.data, &draw.run, glyph.id) else {
                continue;
            };
            for (x, y) in [
                (ink[0], ink[1]),
                (ink[2], ink[1]),
                (ink[0], ink[3]),
                (ink[2], ink[3]),
            ] {
                let (x, y) = (x + glyph.x, y + glyph.y);
                resting_box[0] = resting_box[0].min(x);
                resting_box[1] = resting_box[1].min(y);
                resting_box[2] = resting_box[2].max(x);
                resting_box[3] = resting_box[3].max(y);
                if visible {
                    let (px, py) = (m[0] * x + m[2] * y + m[4], m[1] * x + m[3] * y + m[5]);
                    posed_box[0] = posed_box[0].min(px - pad);
                    posed_box[1] = posed_box[1].min(py - pad);
                    posed_box[2] = posed_box[2].max(px + pad);
                    posed_box[3] = posed_box[3].max(py + pad);
                }
            }
        }
    }
    let finite = |b: [f64; 4]| b.iter().all(|v| v.is_finite()).then(|| rect(b));
    (finite(resting_box), finite(posed_box))
}

/// Today's resting interaction box (`textGeometry(d).selection`): an
/// authored paragraph's box (its height, or the lines' height when it has
/// none, never under one line), aligned at the origin; otherwise the
/// resting ink, or a one-pixel caret column at the origin for blank text.
/// Both padded by `clamp(size × 0.055, 3, 12)`.
fn resting_selection(d: &TextContent, layout: &Level2Layout, ink: Option<Selection>) -> Selection {
    let pad = (d.size() * 0.055).clamp(3.0, 12.0);
    if d.paragraph.unwrap_or(false)
        && let Some(width) = d.box_width.filter(|w| w.is_finite() && *w > 0.0)
    {
        let height = d
            .box_height
            .filter(|h| h.is_finite() && *h > 0.0)
            .map_or(d.line_height() * layout.lines.len().max(1) as f64, |h| {
                h.max(d.line_height())
            });
        let x = match d.align() {
            pm_text::compat::Align::Left => 0.0,
            pm_text::compat::Align::Center => -width / 2.0,
            pm_text::compat::Align::Right => -width,
        };
        return rect([x - pad, -pad, x + width + pad, height + pad]);
    }
    // Blank multiline text keeps all of its line boxes interactive too.
    let ink = ink.unwrap_or_else(|| rect([-0.5, 0.0, 0.5, layout.height.max(d.size())]));
    rect([ink.x0 - pad, ink.y0 - pad, ink.x1 + pad, ink.y1 + pad])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraph_selection_uses_authored_box_and_alignment() {
        let layout = Level2Layout {
            lines: vec![Default::default(), Default::default()],
            ..Default::default()
        };
        for (align, x0, x1) in [
            ("left", -3.0, 263.0),
            ("center", -133.0, 133.0),
            ("right", -263.0, 3.0),
        ] {
            let d = TextContent {
                size: Some(12.0),
                leading: Some(1.25),
                align: Some(align.into()),
                paragraph: Some(true),
                box_width: Some(260.0),
                box_height: Some(220.0),
                ..Default::default()
            };
            let selection = resting_selection(&d, &layout, Some(rect([12.0, 3.0, 34.0, 14.0])));
            assert_eq!(
                (selection.x0, selection.x1, selection.y0, selection.y1),
                (x0, x1, -3.0, 223.0)
            );
            let auto = TextContent {
                box_height: None,
                ..d
            };
            assert_eq!(resting_selection(&auto, &layout, None).y1, 33.0);
        }
    }

    #[test]
    fn blank_point_text_has_an_interaction_box() {
        for text in ["   ", "", "\t\u{2003}\n"] {
            let d = TextContent {
                size: Some(8.0),
                text: text.into(),
                ..Default::default()
            };
            let selection = resting_selection(&d, &Level2Layout::default(), None);
            assert_eq!(
                (selection.x0, selection.x1, selection.y0, selection.y1),
                (-3.5, 3.5, -3.0, 11.0)
            );
            let multiline = Level2Layout {
                height: 30.0,
                ..Default::default()
            };
            assert_eq!(resting_selection(&d, &multiline, None).y1, 33.0);
        }
    }

    #[test]
    fn whitespace_glyphs_have_no_ink() {
        use crate::text_measure::{CompatMeasure, test_fonts};
        let data = FontBytes::new(test_fonts::font(400, 500, 400));
        let glyph = |id, x| pm_text::compat::PlacedGlyph { id, x, y: 0.0 };
        // "A" then U+2003, whose glyph has an advance and no outline.
        let run = Run {
            line: 0,
            font: "k".into(),
            size: 10.0,
            coords: vec![],
            synthetic_bold: false,
            skew: 0.0,
            glyphs: vec![
                glyph(2 + 0x41 - 0x21, 0.0),
                glyph(test_fonts::EM_SPACE, 5.0),
            ],
            sources: vec![0, 1],
        };
        let mut measure = CompatMeasure::new(test_fonts::Fonts::new());
        // The outline-less fallback would give the space a box.
        assert!(
            measure
                .level2_ink(&data, &run, test_fonts::EM_SPACE)
                .is_some()
        );
        let draw = Draw {
            run,
            data,
            matrix: IDENTITY,
            opacity: 1.0,
            blur: 0.0,
            color: JsValue::from("#fff"),
        };
        let (resting, ink) = bounds(std::slice::from_ref(&draw), "A\u{2003}", &mut measure);
        let resting = resting.unwrap();
        assert_eq!((resting.x0, resting.x1), (0.5, 4.5));
        assert_eq!(ink.unwrap().x1, 4.5);
    }
}
