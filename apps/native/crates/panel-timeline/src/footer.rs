//! The timeline's bottom bar and its scroll thumbs: zoom under the layer
//! column (a slider between its two ends, on a logarithmic scale), the
//! composition's length beside it, and under the tracks the navigator, a
//! thumb as long as the stretch of time on screen. The row thumb rides the
//! tracks' trailing edge. Geometry and its inverse live here; the host owns
//! the pointer.
use crate::render::{Op, Rect};
use pm_design::Tokens;
use serde_json::json;

/// What the bar is laid out from. Points and seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The panel's width and the bar's top and height.
    pub width: f64,
    pub top: f64,
    pub height: f64,
    pub layer_column: f64,
    /// Pixels per second now, and the range the view allows.
    pub pps: f64,
    pub pps_range: (f64, f64),
    pub scroll_time: f64,
    /// The first and last second there is anything at.
    pub time: (f64, f64),
    /// The rows' area (below the ruler) and what it scrolls.
    pub rows_top: f64,
    pub scroll_y: f64,
    pub content_height: f64,
}
/// A part of the bar the pointer can be over or hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    ZoomOut,
    ZoomIn,
    /// The slider's thumb, and its track either side of it.
    Zoom,
    ZoomTrack,
    /// The navigator's thumb, and its track either side of it.
    Time,
    TimeTrack,
    /// The row thumb, and its track above and below it.
    Rows,
    RowsTrack,
}
/// A thumb's thickness, the least length it shrinks to, and its distance
/// from the edge it rides.
const THUMB: f64 = 6.;
const THUMB_LEAST: f64 = 28.;
const THUMB_EDGE: f64 = 3.;
/// The zoom slider: its track's length and thickness, its knob, and the
/// glyphs at its ends.
const ZOOM_TRACK: f64 = 96.;
const ZOOM_LINE: f64 = 2.;
const ZOOM_KNOB: f64 = 10.;
const ZOOM_GLYPH: f64 = 8.;
/// One press of an end glyph.
pub const ZOOM_STEP: f64 = 1.5;
/// Ink steps: a track, a thumb at rest, a thumb under the pointer or held.
const TRACK_INK: f64 = 0.12;
const THUMB_INK: f64 = 0.22;
const THUMB_LIT: f64 = 0.38;

impl Layout {
    fn inset(self, t: &Tokens) -> f64 {
        t.metrics.inset as f64
    }
    fn middle(self) -> f64 {
        self.top + self.height / 2.
    }
    /// The zoom slider's track: from, to.
    fn zoom_track(self, t: &Tokens) -> Option<(f64, f64)> {
        let from = self.inset(t) + ZOOM_GLYPH + t.metrics.gap as f64;
        let to = from + ZOOM_TRACK;
        (to + t.metrics.gap as f64 + ZOOM_GLYPH + self.inset(t) <= self.layer_column)
            .then_some((from, to))
    }
    /// Where on the slider the current zoom stands, 0 to 1.
    pub fn zoom_fraction(self) -> f64 {
        let (least, most) = self.pps_range;
        ((self.pps / least).ln() / (most / least).ln()).clamp(0., 1.)
    }
    /// Pixels per second for a pointer at `x` on the slider.
    pub fn zoom_at(self, x: f64, t: &Tokens) -> Option<f64> {
        let (from, to) = self.zoom_track(t)?;
        let (least, most) = self.pps_range;
        let fraction = ((x - from) / (to - from)).clamp(0., 1.);
        Some(least * (most / least).powf(fraction))
    }
    /// The seconds on screen across the tracks.
    fn shown(self) -> f64 {
        ((self.width - self.layer_column) / self.pps).max(0.)
    }
    /// The stretch of time the navigator spans: the composition, grown to
    /// hold a view scrolled past either end.
    fn span(self) -> (f64, f64) {
        (
            self.time.0.min(self.scroll_time),
            self.time.1.max(self.scroll_time + self.shown()),
        )
    }
    fn time_track(self, t: &Tokens) -> (f64, f64) {
        (
            self.layer_column + t.metrics.gap as f64,
            self.width - t.metrics.gap as f64,
        )
    }
    /// The navigator's thumb: from, length. None when all of time is shown.
    pub fn time_thumb(self, t: &Tokens) -> Option<(f64, f64)> {
        let (from, to) = self.time_track(t);
        let (first, last) = self.span();
        let (room, all) = (to - from, last - first);
        if room <= THUMB_LEAST || all <= 0. || self.shown() >= all - 1e-6 {
            return None;
        }
        let length = (room * self.shown() / all).max(THUMB_LEAST);
        let travel = room - length;
        let at = (self.scroll_time - first) / (all - self.shown());
        Some((from + travel * at.clamp(0., 1.), length))
    }
    /// The first second on screen with the navigator's thumb starting at `x`.
    pub fn scroll_time_at(self, x: f64, t: &Tokens) -> Option<f64> {
        let (from, to) = self.time_track(t);
        let (_, length) = self.time_thumb(t)?;
        let (first, last) = self.span();
        let travel = (to - from - length).max(1e-6);
        let at = ((x - from) / travel).clamp(0., 1.);
        Some(first + at * (last - first - self.shown()))
    }
    fn rows_track(self) -> (f64, f64) {
        (self.rows_top + THUMB_EDGE, self.top - THUMB_EDGE)
    }
    /// The row thumb: from, length. None when every row is on screen.
    pub fn rows_thumb(self) -> Option<(f64, f64)> {
        let (from, to) = self.rows_track();
        let (room, view) = (to - from, self.top - self.rows_top);
        if room <= THUMB_LEAST || self.content_height <= view + 0.5 {
            return None;
        }
        let length = (room * view / self.content_height).max(THUMB_LEAST);
        let at = self.scroll_y / (self.content_height - view);
        Some((from + (room - length) * at.clamp(0., 1.), length))
    }
    /// The rows' scroll with the row thumb starting at `y`.
    pub fn scroll_y_at(self, y: f64) -> Option<f64> {
        let (from, to) = self.rows_track();
        let (_, length) = self.rows_thumb()?;
        let view = self.top - self.rows_top;
        let at = ((y - from) / (to - from - length).max(1e-6)).clamp(0., 1.);
        Some(at * (self.content_height - view))
    }
    /// The part under a point, with room round the small ones.
    pub fn hit(self, x: f64, y: f64, t: &Tokens) -> Option<Part> {
        let reach = t.metrics.hit as f64 / 2.;
        if y < self.top {
            let (from, length) = self.rows_thumb()?;
            let lane = self.width - THUMB_EDGE - THUMB - reach / 2.;
            if x < lane || y < self.rows_top {
                return None;
            }
            return Some(if y >= from && y <= from + length {
                Part::Rows
            } else {
                Part::RowsTrack
            });
        }
        if x >= self.layer_column {
            let (from, length) = self.time_thumb(t)?;
            return Some(if x >= from && x <= from + length {
                Part::Time
            } else {
                Part::TimeTrack
            });
        }
        let (from, to) = self.zoom_track(t)?;
        let knob = from + (to - from) * self.zoom_fraction();
        if (x - knob).abs() <= reach {
            Some(Part::Zoom)
        } else if x < from - t.metrics.gap as f64 / 2. && x >= from - ZOOM_GLYPH - reach {
            Some(Part::ZoomOut)
        } else if x > to + t.metrics.gap as f64 / 2. && x <= to + ZOOM_GLYPH + reach {
            Some(Part::ZoomIn)
        } else if x >= from && x <= to {
            Some(Part::ZoomTrack)
        } else {
            None
        }
    }
    /// The bar, its thumbs and the row thumb, for the timeline painter.
    /// `lit` is the part under the pointer or held; `length` the
    /// composition's length as it reads in the bar.
    pub fn ops(self, lit: Option<Part>, length: &str, t: &Tokens) -> Vec<Op> {
        let (m, c) = (t.metrics, &t.colors);
        let mut ops = vec![];
        let bar = Rect::new(0., self.top, self.width, self.height);
        let mut ground = Op::new("footer", "rect", bar, c.panel);
        ground.opacity = if t.translucent() {
            t.translucency.panel as f64
        } else {
            1.
        };
        ops.push(ground);
        ops.push(Op::new(
            "footer-gutter",
            "rect",
            Rect::new(0., self.top, self.layer_column, self.height),
            c.panel_secondary,
        ));
        let (edge, alpha) = t.line();
        for (id, r) in [
            (
                "footer-edge",
                Rect::new(0., self.top, self.width, m.border as f64),
            ),
            (
                "footer-column",
                Rect::new(
                    self.layer_column - m.border as f64,
                    self.top,
                    m.border as f64,
                    self.height,
                ),
            ),
        ] {
            let mut o = Op::new(id, "rect", r, edge);
            o.opacity = alpha as f64;
            ops.push(o);
        }
        let ink = |part: Part, held: Part| {
            if lit == Some(part) || lit == Some(held) {
                THUMB_LIT
            } else {
                THUMB_INK
            }
        };
        let middle = self.middle();
        if let Some((from, to)) = self.zoom_track(t) {
            let glyph = |id: &str, x: f64, plus: bool, part: Part, ops: &mut Vec<Op>| {
                let lit = if lit == Some(part) { 1. } else { 0.62 };
                let mut o = Op::new(
                    format!("{id}:h"),
                    "rect",
                    Rect::new(x, middle - 0.75, ZOOM_GLYPH, 1.5),
                    c.text,
                );
                o.opacity = lit;
                o.radius = 0.75;
                ops.push(o);
                if plus {
                    let mut o = Op::new(
                        format!("{id}:v"),
                        "rect",
                        Rect::new(
                            x + ZOOM_GLYPH / 2. - 0.75,
                            middle - ZOOM_GLYPH / 2.,
                            1.5,
                            ZOOM_GLYPH,
                        ),
                        c.text,
                    );
                    o.opacity = lit;
                    o.radius = 0.75;
                    ops.push(o);
                }
            };
            glyph(
                "zoom-out",
                from - m.gap as f64 - ZOOM_GLYPH,
                false,
                Part::ZoomOut,
                &mut ops,
            );
            glyph("zoom-in", to + m.gap as f64, true, Part::ZoomIn, &mut ops);
            let mut track = Op::new(
                "zoom-track",
                "rect",
                Rect::new(from, middle - ZOOM_LINE / 2., to - from, ZOOM_LINE),
                c.text,
            );
            track.opacity = TRACK_INK;
            track.radius = ZOOM_LINE / 2.;
            ops.push(track);
            let x = from + (to - from) * self.zoom_fraction();
            let mut knob = Op::new(
                "zoom-knob",
                "circle",
                Rect::new(
                    x - ZOOM_KNOB / 2.,
                    middle - ZOOM_KNOB / 2.,
                    ZOOM_KNOB,
                    ZOOM_KNOB,
                ),
                if lit == Some(Part::Zoom) {
                    c.text
                } else {
                    c.secondary
                },
            );
            // The knob covers the track: it sits on the bar's own ground.
            knob.detail = json!({"keyline":c.panel_secondary,"keylineWidth":2.});
            ops.push(knob);
            // The composition's length, at the column's trailing edge.
            let size = t.typography.minimum as f64;
            let width = size * 0.62 * length.len() as f64;
            let x = self.layer_column - self.inset(t) - width;
            if x >= to + m.gap as f64 + ZOOM_GLYPH + m.inset as f64 {
                let mut o = Op::new(
                    "length",
                    "text",
                    Rect::new(x, self.top, width, self.height),
                    c.muted,
                );
                o.text = Some(length.into());
                o.font_size = size;
                o.detail = json!({"tabular":true,"align":"right"});
                ops.push(o);
            }
        }
        if let Some((from, length)) = self.time_thumb(t) {
            let mut o = Op::new(
                "time-thumb",
                "rect",
                Rect::new(from, middle - THUMB / 2., length, THUMB),
                c.text,
            );
            o.opacity = ink(Part::Time, Part::TimeTrack);
            o.radius = THUMB / 2.;
            ops.push(o);
        }
        if let Some((from, length)) = self.rows_thumb() {
            let mut o = Op::new(
                "rows-thumb",
                "rect",
                Rect::new(self.width - THUMB_EDGE - THUMB, from, THUMB, length),
                c.text,
            );
            o.opacity = ink(Part::Rows, Part::RowsTrack);
            o.radius = THUMB / 2.;
            ops.push(o);
        }
        ops
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn layout() -> Layout {
        Layout {
            width: 1100.,
            top: 376.,
            height: 24.,
            layer_column: 300.,
            pps: 40.,
            pps_range: (4., 4000.),
            scroll_time: 5.,
            time: (0., 60.),
            rows_top: 24.,
            scroll_y: 120.,
            content_height: 2400.,
        }
    }
    #[test]
    fn the_zoom_slider_is_logarithmic_and_reads_back_where_it_was_set() {
        let (l, t) = (
            layout(),
            Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default()),
        );
        let (from, to) = l.zoom_track(&t).unwrap();
        assert!((l.zoom_at(from, &t).unwrap() - 4.).abs() < 1e-9);
        assert!((l.zoom_at(to, &t).unwrap() - 4000.).abs() < 1e-6);
        // The middle of the slider is the geometric middle of the range.
        let middle = l.zoom_at((from + to) / 2., &t).unwrap();
        assert!((middle - (4f64 * 4000.).sqrt()).abs() < 1e-6);
        let at = from + (to - from) * l.zoom_fraction();
        assert!((l.zoom_at(at, &t).unwrap() - l.pps).abs() < 1e-6);
    }
    #[test]
    fn the_navigator_thumb_is_the_time_on_screen_and_drags_back_to_it() {
        let (l, t) = (
            layout(),
            Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default()),
        );
        let (from, length) = l.time_thumb(&t).unwrap();
        let (a, b) = l.time_track(&t);
        // 20 of 60 seconds are on screen: a third of the track.
        assert!((length - (b - a) / 3.).abs() < 1e-6);
        assert!((l.scroll_time_at(from, &t).unwrap() - 5.).abs() < 1e-6);
        assert_eq!(l.scroll_time_at(a - 50., &t), Some(0.));
        assert!((l.scroll_time_at(b, &t).unwrap() - 40.).abs() < 1e-6);
        assert_eq!(l.hit(from + 1., l.top + 12., &t), Some(Part::Time));
        assert_eq!(l.hit(from - 4., l.top + 12., &t), Some(Part::TimeTrack));
    }
    #[test]
    fn a_view_that_shows_everything_has_no_thumbs() {
        let (mut l, t) = (
            layout(),
            Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default()),
        );
        l.pps = 4.;
        l.scroll_time = 0.;
        l.content_height = 200.;
        assert_eq!(l.time_thumb(&t), None);
        assert_eq!(l.rows_thumb(), None);
        assert_eq!(l.hit(700., l.top + 12., &t), None);
        assert_eq!(l.hit(l.width - 4., 100., &t), None);
        assert!(
            l.ops(None, "0:01:00:00", &t)
                .iter()
                .all(|o| !o.id.ends_with("thumb"))
        );
    }
    #[test]
    fn the_row_thumb_reads_back_the_scroll_it_stands_for() {
        let l = layout();
        let (from, length) = l.rows_thumb().unwrap();
        assert!(length >= THUMB_LEAST);
        assert!((l.scroll_y_at(from).unwrap() - 120.).abs() < 1e-6);
        let view = l.top - l.rows_top;
        assert!((l.scroll_y_at(l.top).unwrap() - (2400. - view)).abs() < 1e-6);
        let t = Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default());
        assert_eq!(l.hit(l.width - 5., from + 2., &t), Some(Part::Rows));
        assert_eq!(
            l.hit(l.width - 5., from + length + 8., &t),
            Some(Part::RowsTrack)
        );
        assert_eq!(l.hit(l.width - 40., from + 2., &t), None);
    }
    #[test]
    fn a_narrow_layer_column_gives_up_the_length_then_the_slider() {
        let (mut l, t) = (
            layout(),
            Tokens::new(pm_design::Scheme::Dark, pm_design::Accessibility::default()),
        );
        assert!(
            l.ops(None, "0:01:00:00", &t)
                .iter()
                .any(|o| o.id == "length")
        );
        l.layer_column = 190.;
        let ops = l.ops(None, "0:01:00:00", &t);
        assert!(ops.iter().any(|o| o.id == "zoom-knob"));
        assert!(!ops.iter().any(|o| o.id == "length"));
        l.layer_column = 120.;
        assert!(
            !l.ops(None, "0:01:00:00", &t)
                .iter()
                .any(|o| o.id == "zoom-knob")
        );
        assert_eq!(l.hit(40., l.top + 12., &t), None);
    }
}
