//! The easing curve controls the Easing panel draws: an editor (a cubic
//! Bézier from (0, 0) to (1, 1) with its two direction handles to drag, as
//! CSS writes `cubic-bezier(x1, y1, x2, y2)`) and a preset's thumbnail. The
//! numbers are the kernel's easing curve (`EaseSpec::Curve`); nothing here
//! is a second model of a keyframe's ease.
use gpui::{
    Bounds, BoxShadow, Corners, PaintQuad, PathBuilder, Pixels, Point, Window, point, px, size,
};
use pm_design::Tokens;

/// `[x1, y1, x2, y2]`: the outgoing handle, then the incoming one.
pub type Curve = [f64; 4];

/// How far the editor shows past 0 and 1 vertically when the handles stay
/// inside: room to grab a handle at the square's edge and pull it out.
pub const MARGIN: f64 = 0.15;
/// The greatest overshoot a handle may take (the kernel's limit).
pub const OVERSHOOT: f64 = 4.;

/// The vertical span an editor shows for `curve`: the unit square with a
/// margin, grown to include both handles.
pub fn range(curve: &Curve) -> (f64, f64) {
    (
        curve[1].min(curve[3]).min(0.) - MARGIN,
        curve[1].max(curve[3]).max(1.) + MARGIN,
    )
}

/// Where a curve sits in a control: `area` maps x 0..1 across and values
/// `min`..`max` up.
#[derive(Clone, Copy, Debug)]
pub struct Graph {
    pub area: Bounds<Pixels>,
    pub min: f64,
    pub max: f64,
}
impl Graph {
    pub fn new(area: Bounds<Pixels>, (min, max): (f64, f64)) -> Self {
        Self {
            area,
            min,
            max: max.max(min + 1e-6),
        }
    }
    pub fn to_point(&self, (x, y): (f64, f64)) -> Point<Pixels> {
        let a = self.area;
        let f = (y - self.min) / (self.max - self.min);
        point(
            a.origin.x + a.size.width * x as f32,
            a.origin.y + a.size.height * (1. - f) as f32,
        )
    }
    pub fn to_value(&self, p: Point<Pixels>) -> (f64, f64) {
        let a = self.area;
        let f = 1. - f64::from((p.y - a.origin.y) / a.size.height);
        (
            f64::from((p.x - a.origin.x) / a.size.width),
            self.min + f * (self.max - self.min),
        )
    }
}

/// The curve's point at parameter `s` (0 to 1).
pub fn at(curve: &Curve, s: f64) -> (f64, f64) {
    let b = |p1: f64, p2: f64| {
        let r = 1. - s;
        3. * r * r * s * p1 + 3. * r * s * s * p2 + s * s * s
    };
    (b(curve[0], curve[2]), b(curve[1], curve[3]))
}

/// The eased value at progress `x` (0 to 1): the curve's height where it
/// crosses `x` (x always rises, since both handles' x stay in 0..1).
pub fn ease(curve: &Curve, x: f64) -> f64 {
    let x = x.clamp(0., 1.);
    let (mut lo, mut hi) = (0., 1.);
    for _ in 0..48 {
        let mid = (lo + hi) / 2.;
        if at(curve, mid).0 < x {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    at(curve, (lo + hi) / 2.).1
}

/// A handle dragged to `(x, y)`: x stays in 0..1, y within the kernel's
/// overshoot and the span shown (`within`); with Shift both snap to a
/// twentieth, rounded to thousandths as the fields show them.
pub fn place((x, y): (f64, f64), within: (f64, f64), snap: bool) -> (f64, f64) {
    let step = |v: f64| if snap { (v * 20.).round() / 20. } else { v };
    let tidy = |v: f64| (v * 1000.).round() / 1000.;
    (
        tidy(step(x).clamp(0., 1.)),
        tidy(step(y).clamp(within.0.max(-OVERSHOOT), within.1.min(OVERSHOOT))),
    )
}

/// The handle (0 outgoing, 1 incoming) within `reach` points of `p`, the
/// nearer when both are.
pub fn handle_at(graph: &Graph, curve: &Curve, p: Point<Pixels>, reach: f32) -> Option<usize> {
    [(curve[0], curve[1]), (curve[2], curve[3])]
        .iter()
        .enumerate()
        .map(|(i, h)| {
            (
                i,
                f32::from((graph.to_point(*h) - p).magnitude() as f32 * px(1.)),
            )
        })
        .filter(|(_, d)| *d <= reach)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// What the editor draws besides the curve.
#[derive(Clone, Copy, Debug, Default)]
pub struct Look {
    pub disabled: bool,
    /// The handle being dragged.
    pub held: Option<usize>,
    /// Where the preview dot runs, as progress (0 to 1).
    pub dot: Option<f64>,
}

/// The editor: its well, the unit square and the linear diagonal faintly,
/// the curve in the accent, the handle lines and their knobs, the preview
/// dot.
pub fn paint_editor(graph: Graph, curve: &Curve, look: Look, t: &Tokens, window: &mut Window) {
    let c = &t.colors;
    let dark = t.scheme == pm_design::Scheme::Dark;
    let strong = if t.accessibility.increase_contrast {
        1.8
    } else {
        1.
    };
    let alpha = if look.disabled { 0.4 } else { 1. };
    let w = t.metrics.border;
    let pad = px(t.metrics.keyframe / 2. + w);
    let a = graph.area;
    let well = Bounds::new(
        a.origin - point(pad, pad),
        size(a.size.width + pad * 2., a.size.height + pad * 2.),
    );
    let corners = Corners::all(px(t.metrics.control_radius));
    window.paint_quad(
        PaintQuad::new(well)
            .background(crate::kit::ink(0, if dark { 0.38 } else { 0.06 }))
            .corner_radii(corners),
    );
    window.paint_inset_shadows(
        well,
        corners,
        0.,
        &[BoxShadow {
            inset: true,
            color: crate::kit::ink(0, if dark { 0.6 } else { 0.18 }).into(),
            offset: point(px(0.), px(2.)),
            blur_radius: px(4.),
            spread_radius: px(0.),
        }],
    );
    // The unit square: where the motion starts (0) and lands (1).
    let faint = crate::kit::ink(c.text, t.translucency.grid * alpha * strong);
    for (from, to) in [
        ((0., 0.), (1., 0.)),
        ((0., 1.), (1., 1.)),
        ((0., 0.), (0., 1.)),
        ((1., 0.), (1., 1.)),
    ] {
        line(graph.to_point(from), graph.to_point(to), w, faint, window);
    }
    // Linear, for reference.
    line(
        graph.to_point((0., 0.)),
        graph.to_point((1., 1.)),
        w,
        crate::kit::ink(c.text, t.translucency.line * 2. * alpha * strong),
        window,
    );
    let ink = crate::kit::ink(c.accent, alpha);
    let handles = [(curve[0], curve[1]), (curve[2], curve[3])];
    let ends = [(0., 0.), (1., 1.)];
    for (end, handle) in ends.iter().zip(handles) {
        line(
            graph.to_point(*end),
            graph.to_point(handle),
            w,
            crate::kit::ink(c.secondary, alpha),
            window,
        );
    }
    let mut path = PathBuilder::stroke(px(w * 2.));
    path.move_to(graph.to_point((0., 0.)));
    path.cubic_bezier_to(
        graph.to_point((1., 1.)),
        graph.to_point(handles[0]),
        graph.to_point(handles[1]),
    );
    if let Ok(path) = path.build() {
        window.paint_path(path, ink);
    }
    for end in ends {
        dot(
            graph.to_point(end),
            w * 4.,
            crate::kit::ink(c.text, alpha),
            window,
        );
    }
    let d = t.metrics.keyframe;
    for (i, h) in handles.iter().enumerate() {
        let held = look.held == Some(i);
        let side = if held { d + 2. * w + 2. } else { d };
        let at = graph.to_point(*h);
        let b = Bounds::new(
            at - point(px(side / 2.), px(side / 2.)),
            size(px(side), px(side)),
        );
        window.paint_drop_shadows(
            b,
            Corners::all(b.size.width / 2.),
            0.,
            &[BoxShadow {
                color: crate::kit::ink(0, 0.55 * alpha).into(),
                offset: point(px(0.), px(1.)),
                blur_radius: px(2.5),
                spread_radius: px(0.),
                inset: false,
            }],
        );
        let body = if held {
            ink
        } else {
            crate::kit::ink(if dark { 0xf2f2f2 } else { 0xffffff }, alpha)
        };
        window.paint_quad(
            PaintQuad::new(b)
                .corner_radii(Corners::all(b.size.width / 2.))
                .background(body)
                .border_widths(px(w * 1.5))
                .border_color(ink),
        );
    }
    if let Some(x) = look.dot {
        dot(graph.to_point((x, ease(curve, x))), d * 0.8, ink, window);
    }
}

/// A preset's thumbnail: its curve over the faint diagonal, fitted to
/// `bounds` with the overshoot it has.
pub fn paint_thumbnail(bounds: Bounds<Pixels>, curve: &Curve, t: &Tokens, window: &mut Window) {
    let c = &t.colors;
    let w = t.metrics.border;
    let inset = px(w * 2.);
    let area = Bounds::new(
        bounds.origin + point(inset, inset),
        size(
            bounds.size.width - inset * 2.,
            bounds.size.height - inset * 2.,
        ),
    );
    let (mut lo, mut hi) = (0f64, 1f64);
    for s in 0..=32 {
        let y = at(curve, s as f64 / 32.).1;
        (lo, hi) = (lo.min(y), hi.max(y));
    }
    let graph = Graph::new(area, (lo, hi));
    line(
        graph.to_point((0., 0.)),
        graph.to_point((1., 1.)),
        w,
        crate::kit::ink(c.text, t.translucency.line * 2.),
        window,
    );
    let mut path = PathBuilder::stroke(px(w * 1.5));
    path.move_to(graph.to_point((0., 0.)));
    path.cubic_bezier_to(
        graph.to_point((1., 1.)),
        graph.to_point((curve[0], curve[1])),
        graph.to_point((curve[2], curve[3])),
    );
    if let Ok(path) = path.build() {
        window.paint_path(path, crate::kit::ink(c.text, 1.));
    }
}

fn line(from: Point<Pixels>, to: Point<Pixels>, width: f32, ink: gpui::Rgba, window: &mut Window) {
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(from);
    path.line_to(to);
    if let Ok(path) = path.build() {
        window.paint_path(path, ink);
    }
}
fn dot(at: Point<Pixels>, side: f32, ink: gpui::Rgba, window: &mut Window) {
    let b = Bounds::new(
        at - point(px(side / 2.), px(side / 2.)),
        size(px(side), px(side)),
    );
    window.paint_quad(
        PaintQuad::new(b)
            .corner_radii(Corners::all(b.size.width / 2.))
            .background(ink),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph() -> Graph {
        Graph::new(
            Bounds::new(point(px(10.), px(20.)), size(px(200.), px(240.))),
            range(&[0.42, 0., 0.58, 1.]),
        )
    }

    #[test]
    fn the_curve_runs_from_rest_to_rest_and_eases_as_css_does() {
        let linear = [0., 0., 1., 1.];
        for x in [0., 0.25, 0.5, 0.9, 1.] {
            assert!((ease(&linear, x) - x).abs() < 1e-6, "{x}");
        }
        let ease_in_out = [0.42, 0., 0.58, 1.];
        assert!(ease(&ease_in_out, 0.).abs() < 1e-9);
        assert!((ease(&ease_in_out, 1.) - 1.).abs() < 1e-9);
        assert!((ease(&ease_in_out, 0.5) - 0.5).abs() < 1e-6);
        // Slow in: under the diagonal early, over it late.
        assert!(ease(&ease_in_out, 0.2) < 0.2 && ease(&ease_in_out, 0.8) > 0.8);
        // Back Out overshoots past 1 before landing.
        let back_out = [0.34, 1.56, 0.64, 1.];
        assert!((0..=100).any(|i| ease(&back_out, i as f64 / 100.) > 1.05));
    }

    #[test]
    fn points_and_values_round_trip_and_the_range_holds_the_handles() {
        let g = graph();
        for v in [(0., 0.), (1., 1.), (0.3, -0.1), (0.7, 1.12)] {
            let back = g.to_value(g.to_point(v));
            assert!(
                (back.0 - v.0).abs() < 1e-5 && (back.1 - v.1).abs() < 1e-5,
                "{back:?}"
            );
        }
        assert_eq!(range(&[0.42, 0., 0.58, 1.]), (-MARGIN, 1. + MARGIN));
        let (lo, hi) = range(&[0.68, -0.6, 0.32, 1.6]);
        assert!(lo < -0.6 && hi > 1.6);
    }

    #[test]
    fn a_dragged_handle_keeps_x_in_the_square_and_snaps_with_shift() {
        assert_eq!(place((1.3, 0.5), (-0.2, 1.2), false), (1., 0.5));
        assert_eq!(place((-0.2, 0.5), (-0.2, 1.2), false), (0., 0.5));
        // Overshoot is allowed up to what is shown.
        assert_eq!(place((0.5, 1.5), (-0.2, 1.2), false), (0.5, 1.2));
        assert_eq!(place((0.5, -9.), (-9., 9.), false), (0.5, -OVERSHOOT));
        assert_eq!(place((0.437, 0.912), (-0.2, 1.2), true), (0.45, 0.9));
        assert_eq!(
            place((0.43712, 0.91234), (-0.2, 1.2), false),
            (0.437, 0.912)
        );
    }

    #[test]
    fn a_press_finds_the_nearer_handle_within_reach() {
        let g = graph();
        let curve = [0.42, 0., 0.58, 1.];
        let out = g.to_point((0.42, 0.));
        assert_eq!(handle_at(&g, &curve, out, 8.), Some(0));
        let inc = g.to_point((0.58, 1.)) + point(px(3.), px(-2.));
        assert_eq!(handle_at(&g, &curve, inc, 8.), Some(1));
        assert_eq!(handle_at(&g, &curve, g.to_point((0.5, 0.5)), 8.), None);
    }
}
