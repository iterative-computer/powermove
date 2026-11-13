//! The viewer's rulers, grid and ruler guides (View ▸ Show Rulers, Show
//! Grid, Clear Guides; a guide is dragged out of a ruler, moved, and dragged
//! back into a ruler to remove it). Drawn by GPUI over the picture in
//! composition pixels, placed from the same `ViewFrame` as the transform
//! handles, so they register with the picture at any zoom. Display only:
//! nothing here reaches a render, a capture or an export.
use super::{GizmoSurface, ViewFrame};
use gpui::*;

/// A guide's direction: a horizontal guide is a line at a `y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RulerGuide {
    pub axis: Axis,
    /// Composition pixels.
    pub at: f64,
}
#[derive(Clone, Debug, Default)]
pub struct Rulers {
    pub shown: bool,
    pub grid: bool,
    pub guides: Vec<RulerGuide>,
    /// The guide the pointer carries (a new one came out of a ruler).
    pub dragging: Option<usize>,
    /// The pointer over the viewer, for the rulers' marks.
    pub pointer: Option<Point<Pixels>>,
}

/// The finest step in composition pixels (1, 2 or 5 × 10ⁿ) whose spacing
/// on screen is at least `least` points at `scale`.
pub fn step(scale: f64, least: f64) -> f64 {
    let mut base = 1.;
    while base < 1e7 {
        for m in [1., 2., 5.] {
            if base * m * scale >= least {
                return base * m;
            }
        }
        base *= 10.;
    }
    base
}

/// Where a guide let go at `at` lands: on the composition's edge or centre
/// within `reach` pixels, else on a whole pixel.
pub fn settle(at: f64, size: f64, reach: f64) -> f64 {
    [0., size / 2., size]
        .into_iter()
        .find(|t| (at - t).abs() <= reach)
        .unwrap_or(at.round())
}

impl GizmoSurface {
    /// The ruler band's thickness, or nothing while the rulers are hidden.
    fn band(&self) -> f32 {
        if self.rulers.shown {
            self.tokens.metrics.viewer_ruler
        } else {
            0.
        }
    }
    fn in_band(&self, p: Point<Pixels>) -> Option<Axis> {
        let band = px(self.band());
        if band <= px(0.) || !self.bounds.contains(&p) {
            return None;
        }
        let o = self.bounds.origin;
        if p.y < o.y + band {
            Some(Axis::Horizontal)
        } else if p.x < o.x + band {
            Some(Axis::Vertical)
        } else {
            None
        }
    }
    /// The guide within reach of `p`, by index.
    fn guide_at(&self, p: Point<Pixels>, frame: ViewFrame) -> Option<usize> {
        let reach = self.tokens.metrics.hit as f64 / 4.;
        self.rulers
            .guides
            .iter()
            .enumerate()
            .filter_map(|(i, g)| {
                let d = match g.axis {
                    Axis::Horizontal => f32::from(p.y - frame.origin.y) as f64 - g.at * frame.scale,
                    Axis::Vertical => f32::from(p.x - frame.origin.x) as f64 - g.at * frame.scale,
                }
                .abs();
                (d <= reach).then_some((i, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }
    /// A press in a ruler starts a guide; a press on a guide carries it.
    pub(super) fn rulers_down(&mut self, p: Point<Pixels>, frame: ViewFrame) -> bool {
        if let Some(axis) = self.in_band(p) {
            let at = self.comp_along(axis, p, frame);
            self.rulers.guides.push(RulerGuide { axis, at });
            self.rulers.dragging = Some(self.rulers.guides.len() - 1);
            return true;
        }
        if self.rulers.shown
            && let Some(i) = self.guide_at(p, frame)
        {
            self.rulers.dragging = Some(i);
            return true;
        }
        false
    }
    fn comp_along(&self, axis: Axis, p: Point<Pixels>, frame: ViewFrame) -> f64 {
        match axis {
            Axis::Horizontal => f32::from(p.y - frame.origin.y) as f64 / frame.scale,
            Axis::Vertical => f32::from(p.x - frame.origin.x) as f64 / frame.scale,
        }
    }
    pub(super) fn rulers_move(&mut self, p: Point<Pixels>, frame: ViewFrame) -> bool {
        let Some(i) = self.rulers.dragging else {
            return false;
        };
        let axis = self.rulers.guides[i].axis;
        let size = match axis {
            Axis::Horizontal => frame.size[1],
            Axis::Vertical => frame.size[0],
        };
        let reach = self.tokens.metrics.gap as f64 / frame.scale;
        self.rulers.guides[i].at = settle(self.comp_along(axis, p, frame), size, reach);
        true
    }
    /// Let go: a guide dropped back on a ruler (or outside the viewer) goes.
    pub(super) fn rulers_up(&mut self, p: Point<Pixels>) -> bool {
        let Some(i) = self.rulers.dragging.take() else {
            return false;
        };
        if self.in_band(p).is_some() || !self.bounds.contains(&p) {
            self.rulers.guides.remove(i);
        }
        true
    }
    /// The cursor over a ruler or a guide.
    pub(super) fn rulers_cursor(&self, p: Point<Pixels>, frame: ViewFrame) -> Option<CursorStyle> {
        let along = |axis| match axis {
            Axis::Horizontal => CursorStyle::ResizeUpDown,
            Axis::Vertical => CursorStyle::ResizeLeftRight,
        };
        if let Some(i) = self.rulers.dragging {
            return Some(along(self.rulers.guides[i].axis));
        }
        if self.in_band(p).is_some() {
            return Some(CursorStyle::Arrow);
        }
        if !self.rulers.shown {
            return None;
        }
        self.guide_at(p, frame)
            .map(|i| along(self.rulers.guides[i].axis))
    }

    pub(super) fn rulers_paint(&self, frame: ViewFrame, window: &mut Window, cx: &mut App) {
        let t = &self.tokens;
        let b = self.bounds;
        let (o, scale) = (frame.origin, frame.scale);
        let x_of = |v: f64| o.x + px((v * scale) as f32);
        let y_of = |v: f64| o.y + px((v * scale) as f32);
        let hair = px(t.metrics.border);
        let line_x = |window: &mut Window, x: Pixels, top: Pixels, bottom: Pixels, color: Rgba| {
            window.paint_quad(
                PaintQuad::new(Bounds::new(point(x, top), size(hair, bottom - top)))
                    .background(color),
            );
        };
        let line_y = |window: &mut Window, y: Pixels, left: Pixels, right: Pixels, color: Rgba| {
            window.paint_quad(
                PaintQuad::new(Bounds::new(point(left, y), size(right - left, hair)))
                    .background(color),
            );
        };
        // The grid lies on the picture only.
        if self.rulers.grid {
            let alpha = (t.translucency.line * 255.).round() as u32;
            let grid = rgba(t.colors.viewer_grid << 8 | alpha);
            let s = step(scale, t.metrics.hit as f64);
            let (left, top) = (x_of(0.).max(b.left()), y_of(0.).max(b.top()));
            let (right, bottom) = (
                x_of(frame.size[0]).min(b.right()),
                y_of(frame.size[1]).min(b.bottom()),
            );
            let mut v = s;
            while v < frame.size[0] {
                let x = x_of(v);
                if x >= left && x <= right {
                    line_x(window, x, top, bottom, grid);
                }
                v += s;
            }
            let mut v = s;
            while v < frame.size[1] {
                let y = y_of(v);
                if y >= top && y <= bottom {
                    line_y(window, y, left, right, grid);
                }
                v += s;
            }
        }
        if !self.rulers.shown {
            return;
        }
        let guide = rgb(t.colors.viewer_guide);
        for g in &self.rulers.guides {
            match g.axis {
                Axis::Horizontal => line_y(window, y_of(g.at), b.left(), b.right(), guide),
                Axis::Vertical => line_x(window, x_of(g.at), b.top(), b.bottom(), guide),
            }
        }
        // The bands: the panel's ground with a hairline at their inner edge.
        let band = px(t.metrics.viewer_ruler);
        let ground = rgb(t.colors.panel);
        let edge = rgb(t.colors.separator);
        let ink = rgb(t.colors.muted);
        window.paint_quad(
            PaintQuad::new(Bounds::new(b.origin, size(b.size.width, band))).background(ground),
        );
        window.paint_quad(
            PaintQuad::new(Bounds::new(b.origin, size(band, b.size.height))).background(ground),
        );
        line_y(window, b.top() + band - hair, b.left(), b.right(), edge);
        line_x(window, b.left() + band - hair, b.top(), b.bottom(), edge);
        let major = step(scale, t.metrics.value_column as f64 / 2.);
        let minor = [major / 10., major / 5., major / 2.]
            .into_iter()
            .find(|m| m * scale >= t.metrics.gap as f64)
            .unwrap_or(major);
        let tick = |long: bool| {
            px(if long {
                t.metrics.viewer_ruler / 2.
            } else {
                t.metrics.viewer_ruler / 4.
            })
        };
        let font_size = px(t.typography.minimum);
        let label = |text: String, window: &mut Window| {
            let run = TextRun {
                len: text.len(),
                font: Font {
                    family: pm_design::TEXT_FONT.into(),
                    features: FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)])),
                    ..window.text_style().font()
                },
                color: rgb_to_hsla(ink),
                background_color: None,
                underline: None,
                strikethrough: None,
                letter_spacing: None,
            };
            window
                .text_system()
                .shape_line(text.into(), font_size, &[run])
        };
        let height = font_size * t.typography.dense_leading;
        let pad = px(t.space.hair);
        // Along the top: composition x; down the side: composition y.
        for axis in [Axis::Vertical, Axis::Horizontal] {
            let span = match axis {
                Axis::Vertical => (
                    f32::from(b.left() - o.x) as f64 / scale,
                    f32::from(b.right() - o.x) as f64 / scale,
                ),
                Axis::Horizontal => (
                    f32::from(b.top() - o.y) as f64 / scale,
                    f32::from(b.bottom() - o.y) as f64 / scale,
                ),
            };
            let mut v = (span.0 / minor).floor() * minor;
            while v <= span.1 {
                let long = ((v / major).round() * major - v).abs() < minor * 1e-6;
                match axis {
                    Axis::Vertical => {
                        let x = x_of(v);
                        if x > b.left() + band {
                            line_x(window, x, b.top() + band - tick(long), b.top() + band, ink);
                            if long {
                                let line = label(format!("{}", v.round() as i64), window);
                                let _ = line.paint(
                                    point(x + pad, b.top()),
                                    height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                    }
                    Axis::Horizontal => {
                        let y = y_of(v);
                        if y > b.top() + band {
                            line_y(
                                window,
                                y,
                                b.left() + band - tick(long),
                                b.left() + band,
                                ink,
                            );
                            // Too narrow a band for a number across: its
                            // digits stand one above the other.
                            if long {
                                let text = format!("{}", v.round() as i64);
                                for (n, c) in text.chars().enumerate() {
                                    let line = label(c.to_string(), window);
                                    let at = point(b.left() + pad, y + pad + height * n as f32);
                                    let _ =
                                        line.paint(at, height, TextAlign::Left, None, window, cx);
                                }
                            }
                        }
                    }
                }
                v += minor;
            }
        }
        // The pointer's place, in both rulers.
        if let Some(p) = self.rulers.pointer.filter(|p| b.contains(p)) {
            let mark = rgb(t.colors.text);
            line_x(window, p.x, b.top(), b.top() + band, mark);
            line_y(window, p.y, b.left(), b.left() + band, mark);
        }
        // The corner the bands share.
        window
            .paint_quad(PaintQuad::new(Bounds::new(b.origin, size(band, band))).background(ground));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[::core::prelude::v1::test]
    fn a_step_is_the_finest_1_2_5_spacing_that_reads() {
        assert_eq!(step(1., 64.), 100.);
        assert_eq!(step(0.5, 64.), 200.);
        assert_eq!(step(0.25, 64.), 500.);
        assert_eq!(step(4., 64.), 20.);
        assert_eq!(step(8., 6.), 1.);
    }
    #[::core::prelude::v1::test]
    fn a_guide_settles_on_an_edge_the_centre_or_a_whole_pixel() {
        assert_eq!(settle(3., 1080., 4.), 0.);
        assert_eq!(settle(538., 1080., 4.), 540.);
        assert_eq!(settle(1077.5, 1080., 4.), 1080.);
        assert_eq!(settle(300.4, 1080., 4.), 300.);
    }
}
