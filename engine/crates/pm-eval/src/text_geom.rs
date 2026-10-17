//! Text layer geometry: the raster box, anchor, and selection bounds today's
//! renderer derives from Canvas2D measurements (`legacy/gl/raster.ts`
//! `textGeometry`, `textRasterGeometry`, `rasterAnimatedText`, and the font
//! anchor adjustment in `PM.raster`).
//!
//! Measurements come from a [`TextMeasure`]: the wrapped lines with their
//! `measureText` widths and ink boxes (Blink's `actualBoundingBox*`), plain
//! advance widths, and the glyph layout. [`crate::text_measure`] implements
//! it over `pm-text`'s level 1 layout.

use pm_model::{JsString, Node};
use pm_text::compat::{GlyphLayout, TextContent};

use crate::anchor::{BoundsValues, font_anchor_offset};
use crate::evaluator::Content;
use crate::js_sem::{JsResult, JsValue};
use crate::node;
use crate::scope::{Scope, clamp, js_max, js_min};
use crate::space3d::Rect;
use crate::text_anim::TextAnimation;

/// One wrapped line as the canvas measures it.
#[derive(Clone, Debug, PartialEq)]
pub struct MeasuredLine {
    pub text: String,
    /// `measureText(line).width`.
    pub width: f64,
    /// The union of the line's glyph ink boxes (`x0`, `y0`, `x1`, `y1`, y
    /// down), relative to the line's pen origin on the baseline: Blink's
    /// `glyph_bounds`. An empty union is `[0, 0, 0, 0]`.
    pub ink: [f64; 4],
}

/// The text measurements geometry depends on.
pub trait TextMeasure {
    /// `textLines(d, meas, lh)` with each line's width and ink, measured with
    /// `font = fontStr(d)` and `letterSpacing = d.tracking px`.
    fn measure_lines(&mut self, d: &TextContent) -> Vec<MeasuredLine>;
    /// `measureText(text).width` on a fresh context: `font = fontStr(d)` and
    /// no letter spacing.
    fn plain_width(&mut self, d: &TextContent, text: &str) -> f64;
    /// `PM.textLayout` (`glyphLayout` on the layer's context).
    fn glyph_layout(&mut self, d: &TextContent) -> GlyphLayout;
    /// `measureText(text).width` with `font = fontStr(d)` and
    /// `letterSpacing = d.tracking px` (the captions measuring canvas).
    fn spaced_width(&mut self, d: &TextContent, text: &str) -> f64;
}

/// Selection bounds with their size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Selection {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub w: f64,
    pub h: f64,
}

impl Selection {
    pub fn rect(&self) -> Rect {
        Rect {
            x0: self.x0,
            y0: self.y0,
            x1: self.x1,
            y1: self.y1,
        }
    }

    fn shifted(&self, dx: f64, dy: f64) -> Selection {
        Selection {
            x0: self.x0 + dx,
            x1: self.x1 + dx,
            y0: self.y0 + dy,
            y1: self.y1 + dy,
            w: self.w,
            h: self.h,
        }
    }
}

/// `textGeometry(d)` (without the raster density).
#[derive(Clone, Debug, PartialEq)]
pub struct TextGeometry {
    /// Raster box size in layer pixels (padding included).
    pub w: f64,
    pub h: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub selection: Selection,
    pub lines: Vec<String>,
    pub line_height: f64,
    pub size: f64,
    pub pad: f64,
    /// The alignment x of every line.
    pub x: f64,
}

impl TextGeometry {
    /// `textRasterGeometry(d, scale)`'s density: `min(scale, 8192 / w, 8192 / h)`.
    pub fn density(&self, scale: f64) -> f64 {
        js_min(js_min(scale, 8192.0 / self.w), 8192.0 / self.h)
    }
}

/// What `PM.raster(L, 1, T)` reports for a text layer's placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextRaster {
    pub w: f64,
    pub h: f64,
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub selection: Selection,
}

/// What `rasterAnimatedText` paints: posed glyphs (range-font advances
/// applied) with their baselines, and the canvas box `[x0, x1] × [y0, y1]`
/// in layer pixels (the canvas spans `max(1, x1 − x0)` × `max(1, y1 − y0)`).
#[derive(Clone, Debug, PartialEq)]
pub struct AnimatedRaster {
    pub glyphs: Vec<crate::text_anim::PosedGlyph>,
    /// `unit.y + d.size × 0.82` per glyph.
    pub baselines: Vec<f64>,
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// The `TextContent` the layout reads from resolved content.
pub fn text_content(d: &Content) -> TextContent {
    let mut map = serde_json::Map::new();
    for (k, v) in d {
        if let Some(j) = v.to_json() {
            map.insert(k.to_string_lossy(), value_to_serde(&j));
        }
    }
    TextContent::from_json(&serde_json::Value::Object(map))
}

fn value_to_serde(v: &pm_model::Value) -> serde_json::Value {
    use pm_model::Value as V;
    match v {
        V::Null => serde_json::Value::Null,
        V::Bool(b) => serde_json::Value::Bool(*b),
        V::Number(n) => serde_json::Number::from_f64(*n)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        V::String(s) => serde_json::Value::String(s.to_string_lossy()),
        V::Array(a) => serde_json::Value::Array(a.iter().map(value_to_serde).collect()),
        V::Object(m) => serde_json::Value::Object(
            m.iter()
                .map(|(k, v)| (k.to_string_lossy(), value_to_serde(v)))
                .collect(),
        ),
    }
}

fn get<'c>(d: &'c Content, key: &str) -> &'c JsValue {
    static UNDEFINED: JsValue = JsValue::Undefined;
    d.get(&JsString::from(key)).unwrap_or(&UNDEFINED)
}

fn is_align(d: &Content, name: &str) -> bool {
    matches!(get(d, "align"), JsValue::String(s) if s.as_str() == Some(name))
}

/// `textGeometry(d)`.
pub fn text_geometry(d: &Content, measure: &mut dyn TextMeasure) -> TextGeometry {
    let size = {
        let s = crate::scope::or0(get(d, "size").to_number());
        js_max(1.0, if s == 0.0 { 16.0 } else { s })
    };
    let pad = (size * 0.6).ceil() + 24.0;
    let center = is_align(d, "center");
    let right = is_align(d, "right");
    let leading = get(d, "leading");
    let lh = size
        * if leading.truthy() {
            leading.to_number()
        } else {
            1.15
        };
    let measured = measure.measure_lines(&text_content(d));
    let mut w_max = 1.0;
    for m in &measured {
        w_max = js_max(w_max, m.width);
    }
    let paragraph = get(d, "paragraph").truthy();
    let bw = get(d, "boxWidth").to_number();
    let bh = get(d, "boxHeight").to_number();
    let paragraph_width = if paragraph && bw.is_finite() {
        js_max(1.0, bw)
    } else {
        0.0
    };
    let paragraph_height = if paragraph && bh.is_finite() {
        js_max(1.0, bh)
    } else {
        0.0
    };
    let w = js_max(w_max, paragraph_width).ceil() + pad * 2.0;
    let hh = js_max(lh * measured.len() as f64, paragraph_height).ceil() + pad * 2.0;
    let x = if center {
        w / 2.0
    } else if right {
        w - pad
    } else {
        pad
    };
    let anchor_x = x;
    let anchor_y = pad;
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    );
    for (i, m) in measured.iter().enumerate() {
        let baseline = pad + lh * i as f64 + size * 0.82;
        // Blink's TextMetrics: offsets from the alignment point.
        let dx = if center {
            m.width / 2.0
        } else if right {
            m.width
        } else {
            0.0
        };
        // `text_align_dx_` and the glyph bounds are `float`, and so is the
        // arithmetic (`-glyph_bounds.x() + text_align_dx_`).
        let left_metric = f64::from(-(m.ink[0] as f32) + dx as f32);
        let right_metric = f64::from(m.ink[2] as f32 - dx as f32);
        let ascent_metric = -m.ink[1];
        let descent_metric = m.ink[3];
        let fallback_left = if center {
            x - m.width / 2.0
        } else if right {
            x - m.width
        } else {
            x
        };
        let fallback_right = fallback_left + m.width;
        let horizontal = left_metric.is_finite()
            && right_metric.is_finite()
            && (left_metric != 0.0 || right_metric != 0.0);
        let left = if horizontal {
            x - left_metric
        } else {
            fallback_left
        };
        let right = if horizontal {
            x + right_metric
        } else {
            fallback_right
        };
        let top = if ascent_metric.is_finite() && ascent_metric > 0.0 {
            baseline - ascent_metric
        } else {
            baseline - size * 0.8
        };
        let bottom = if descent_metric.is_finite() && descent_metric >= 0.0 {
            baseline + descent_metric
        } else {
            baseline + size * 0.2
        };
        min_x = js_min(min_x, left);
        max_x = js_max(max_x, right);
        min_y = js_min(min_y, top);
        max_y = js_max(max_y, bottom);
    }
    if !min_x.is_finite() || max_x <= min_x {
        min_x = x - 0.5;
        max_x = x + 0.5;
    }
    if !min_y.is_finite() || max_y <= min_y {
        min_y = pad;
        max_y = pad + size;
    }
    let ip = clamp(size * 0.055, 3.0, 12.0);
    let mut sel = Selection {
        x0: min_x - anchor_x - ip,
        y0: min_y - anchor_y - ip,
        x1: max_x - anchor_x + ip,
        y1: max_y - anchor_y + ip,
        w: 0.0,
        h: 0.0,
    };
    if paragraph_width > 0.0 {
        sel.x0 = pad - anchor_x - ip;
        sel.x1 = pad + paragraph_width - anchor_x + ip;
        sel.y0 = pad - anchor_y - ip;
        let height = if bh > 0.0 {
            js_max(paragraph_height, lh)
        } else {
            lh * measured.len() as f64
        };
        sel.y1 = pad + height - anchor_y + ip;
    }
    sel.w = sel.x1 - sel.x0;
    sel.h = sel.y1 - sel.y0;
    TextGeometry {
        w,
        h: hh,
        anchor_x,
        anchor_y,
        selection: sel,
        lines: measured.into_iter().map(|m| m.text).collect(),
        line_height: lh,
        size,
        pad,
        x,
    }
}

impl<'d> Scope<'_, 'd> {
    /// `PM.raster(L, 1, T)`'s placement for a text layer: the raster box,
    /// anchor, and selection, with range fonts, animator poses, and the font
    /// anchor applied.
    pub fn text_raster(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
    ) -> JsResult<TextRaster> {
        self.text_raster_with(layer, time, measure, true, true)
    }

    fn text_raster_with(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
        animators: bool,
        anchor: bool,
    ) -> JsResult<TextRaster> {
        let d = self.resolved_text_content(layer, time)?;
        let ld = node::member(Some(layer), "d");
        let count = |key: &str| {
            node::member(ld, key)
                .map(|n| node::get(n, b"length").truthy())
                .unwrap_or(false)
        };
        let has_animators = animators && count("animators");
        let has_styles = count("styles");
        let mut raster = if has_animators || has_styles {
            let layout = measure.glyph_layout(&text_content(&d));
            let mut state = self.text_animation_state(layer, time, &layout)?;
            if !animators {
                state.animators.clear();
            }
            self.animated_text_raster(&d, &layout, &state, measure)
        } else {
            let g = text_geometry(&d, measure);
            TextRaster {
                w: g.w,
                h: g.h,
                anchor_x: g.anchor_x,
                anchor_y: g.anchor_y,
                selection: g.selection,
            }
        };
        let reference = BoundsValues::of(node::member(ld, "fontAnchorBounds")).filter(|_| anchor);
        if let Some(reference) = reference {
            // Character animator movement must not be canceled by typography anchoring.
            let type_bounds = if has_animators {
                self.text_raster_with(layer, time, measure, false, false)?
                    .selection
            } else {
                raster.selection
            };
            let ax = self.channel(layer, "anchor.x", time)?;
            let ay = self.channel(layer, "anchor.y", time)?;
            let offset = font_anchor_offset(
                Some(&reference),
                Some(&BoundsValues::from(type_bounds.rect())),
                &ax,
                &ay,
            );
            raster.anchor_x -= offset.x;
            raster.anchor_y -= offset.y;
            raster.selection = raster.selection.shifted(offset.x, offset.y);
        }
        Ok(raster)
    }

    /// `rasterAnimatedText`'s placement.
    fn animated_text_raster(
        &mut self,
        d: &Content,
        layout: &GlyphLayout,
        state: &TextAnimation,
        measure: &mut dyn TextMeasure,
    ) -> TextRaster {
        let Some(raster) = self.animated_raster_glyphs(d, layout, state, measure) else {
            let g = text_geometry(d, measure);
            return TextRaster {
                w: g.w,
                h: g.h,
                anchor_x: g.anchor_x,
                anchor_y: g.anchor_y,
                selection: g.selection,
            };
        };
        let AnimatedRaster { x0, y0, x1, y1, .. } = raster;
        let w = js_max(1.0, x1 - x0);
        let h = js_max(1.0, y1 - y0);
        let selection = if !state.styles.is_empty() {
            Selection {
                x0,
                y0,
                x1,
                y1,
                w,
                h,
            }
        } else {
            text_geometry(d, measure).selection
        };
        TextRaster {
            w,
            h,
            anchor_x: -x0,
            anchor_y: -y0,
            selection,
        }
    }

    /// The glyphs `rasterAnimatedText` paints and the box they span, or
    /// `None` when the animation is settled (no range styles and every
    /// glyph at rest): that text renders exactly like plain text. Each
    /// glyph's `unit.x` includes the range-font advance shift; it is drawn
    /// at `(unit.x, baseline)` through its matrix.
    pub fn animated_raster_glyphs(
        &mut self,
        d: &Content,
        layout: &GlyphLayout,
        state: &TextAnimation,
        measure: &mut dyn TextMeasure,
    ) -> Option<AnimatedRaster> {
        let mut glyphs = self.animated_glyphs(d, layout, state);
        // A settled animation renders exactly like plain text.
        if state.styles.is_empty() && glyphs.iter().all(|g| g.is_identity()) {
            return None;
        }
        let base_font = text_content(d);
        let align = if is_align(d, "center") {
            0.5
        } else if is_align(d, "right") {
            1.0
        } else {
            0.0
        };
        // Range font metrics contribute to the following characters' advances.
        let mut lines: Vec<usize> = Vec::new();
        for g in &glyphs {
            if !lines.contains(&g.unit.line) {
                lines.push(g.unit.line);
            }
        }
        for line in lines {
            let mut advance = 0.0;
            for g in glyphs.iter_mut().filter(|g| g.unit.line == line) {
                g.unit.x += advance;
                let base = measure.plain_width(&base_font, &g.unit.text);
                let styled = measure.plain_width(&text_content(&g.style), &g.unit.text);
                advance += styled - base;
            }
            for g in glyphs.iter_mut().filter(|g| g.unit.line == line) {
                g.unit.x -= advance * align;
            }
        }
        let d_size = get(d, "size").to_number();
        let (mut x0, mut y0, mut x1, mut y1) = (0.0f64, 0.0f64, 1.0f64, js_max(1.0, d_size));
        let mut baselines = Vec::with_capacity(glyphs.len());
        for g in &glyphs {
            let baseline = g.unit.y + d_size * 0.82;
            baselines.push(baseline);
            // NaN opacity draws nothing, as `g.opacity > 0` filters it.
            if g.opacity.is_nan() || g.opacity <= 0.0 {
                continue;
            }
            let w = measure.plain_width(&text_content(&g.style), &g.unit.text);
            let style_size = get(&g.style, "size").to_number();
            let h = if style_size == 0.0 || style_size.is_nan() {
                d_size
            } else {
                style_size
            };
            let m = g.matrix;
            let pad = 4.0 + g.blur * 2.0;
            for (x, y) in [(0.0, -h), (w, -h), (0.0, h * 0.3), (w, h * 0.3)] {
                let xx = g.unit.x + m[0] * x + m[2] * y + m[4];
                let yy = baseline + m[1] * x + m[3] * y + m[5];
                x0 = js_min(x0, xx - pad);
                y0 = js_min(y0, yy - pad);
                x1 = js_max(x1, xx + pad);
                y1 = js_max(y1, yy + pad);
            }
        }
        Some(AnimatedRaster {
            glyphs,
            baselines,
            x0,
            y0,
            x1,
            y1,
        })
    }

    /// `PM.textRasterGeometry(layer, scale, time)` without the canvas sizes:
    /// the plain text geometry (no animators, styles, or font anchor).
    pub fn text_geometry(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
    ) -> JsResult<TextGeometry> {
        let d = self.resolved_text_content(layer, time)?;
        Ok(text_geometry(&d, measure))
    }

    /// `PM.textLayout(layer, time)`.
    pub fn text_layout(
        &mut self,
        layer: &'d Node,
        time: f64,
        measure: &mut dyn TextMeasure,
    ) -> JsResult<GlyphLayout> {
        let d = self.resolved_text_content(layer, time)?;
        Ok(measure.glyph_layout(&text_content(&d)))
    }
}
