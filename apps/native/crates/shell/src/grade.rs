//! The colour grading controls the panels draw: a colour wheel (a puck over a
//! disc of the colours it pushes toward) and a curve editor (the monotone
//! cubic the Color Curves effect renders, through its points). The values
//! are the Color Wheels and Color Curves parameters; nothing here is a
//! second model of the grade.
use gpui::{
    App, Background, Bounds, Corners, PaintQuad, PathBuilder, Pixels, Point, RenderImage, Window,
    point, px, size,
};
use pm_design::Tokens;
use std::{cell::RefCell, collections::HashMap, sync::Arc};

/// The channel offset at a wheel's rim: a full push moves each channel by
/// at most about this much (Rec.709 weights make red and blue reach further).
pub const WHEEL_REACH: f64 = 0.5;

/// A zero-luma colour offset for a puck at (`u`, `v`) on vectorscope axes
/// (Cb to the right, Cr up), Rec.709 weights: what a wheel adds to red,
/// green and blue.
pub fn offset(u: f64, v: f64) -> [f64; 3] {
    let red = 1.5748 * v;
    let blue = 1.8556 * u;
    let green = -(0.2126 * red + 0.0722 * blue) / 0.7152;
    [red, green, blue]
}
pub fn luma(d: [f64; 3]) -> f64 {
    0.2126 * d[0] + 0.7152 * d[1] + 0.0722 * d[2]
}
/// The puck of a wheel whose channels hold `rgb` around `neutral` (0 for
/// lift, gamma and offset; 1 for gain), inside the unit disc.
pub fn puck(rgb: [f64; 3], neutral: f64) -> (f64, f64) {
    let d = rgb.map(|c| c - neutral);
    let y = luma(d);
    let (u, v) = ((d[2] - y) / 1.8556, (d[0] - y) / 1.5748);
    unit((u / WHEEL_REACH, v / WHEEL_REACH))
}
/// Keeps a point inside the unit disc.
pub fn unit((u, v): (f64, f64)) -> (f64, f64) {
    let r = u.hypot(v);
    if r > 1. { (u / r, v / r) } else { (u, v) }
}
/// The channels for `puck`, keeping the brightness `keep` holds (the part
/// of a wheel's channels its master would also move), within `range`, at
/// the parameters' precision.
pub fn channels(puck: (f64, f64), keep: [f64; 3], neutral: f64, range: [f64; 2]) -> [f64; 3] {
    let y = luma(keep.map(|c| c - neutral));
    let (u, v) = unit(puck);
    offset(u * WHEEL_REACH, v * WHEEL_REACH)
        .map(|d| ((neutral + y + d).clamp(range[0], range[1]) * 1000.).round() / 1000.)
}

/// One Color Curves effect's points as its parameters hold them:
/// `[count, x0, y0, x1, y1, …]` (`count` 2 to 15; the points past `count`
/// are unused).
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub points: Vec<(f64, f64)>,
}
pub const CURVE_POINTS: usize = 15;
impl Curve {
    pub fn from_values(values: &[f64]) -> Self {
        let count = values
            .first()
            .map_or(2, |n| n.round().clamp(2., CURVE_POINTS as f64) as usize);
        let points = (0..count)
            .filter_map(|i| Some((*values.get(1 + 2 * i)?, *values.get(2 + 2 * i)?)))
            .collect();
        Self { points }
    }
    /// The parameter values for these points; slots past the last point
    /// keep `rest` (what they held).
    pub fn values(&self, rest: &[f64]) -> Vec<f64> {
        let mut out = vec![self.points.len() as f64];
        for i in 0..CURVE_POINTS {
            let (x, y) = self.points.get(i).copied().unwrap_or_else(|| {
                (
                    rest.get(1 + 2 * i).copied().unwrap_or(1.),
                    rest.get(2 + 2 * i).copied().unwrap_or(1.),
                )
            });
            out.extend([precise(x), precise(y)]);
        }
        out
    }
    /// The points in input order, those whose input lies within 1e-6 of the
    /// one before merged into the one written last (as the effect reads them).
    pub fn ordered(&self) -> Vec<(f64, f64)> {
        let mut indexed: Vec<(usize, (f64, f64))> =
            self.points.iter().copied().enumerate().collect();
        indexed.sort_by(|a, b| a.1.0.total_cmp(&b.1.0));
        // As the effect reads them: each input against the one just before it
        // (not against the point kept), so a run of near inputs is one point.
        let mut out: Vec<(usize, (f64, f64))> = vec![];
        let mut prev = f64::NEG_INFINITY;
        for (i, p) in indexed {
            match out.last_mut() {
                Some(last) if p.0 - prev < 1e-6 => {
                    if i > last.0 {
                        *last = (i, p);
                    }
                }
                _ => out.push((i, p)),
            }
            prev = p.0;
        }
        out.into_iter().map(|(_, p)| p).collect()
    }
    /// The output for input `v`: a monotone cubic through the points
    /// (Fritsch-Carlson tangents, the end tangents the end secants),
    /// straight beyond the first and last point; the shader's curve.
    pub fn sample(&self, v: f64) -> f64 {
        sample(&self.ordered(), v)
    }
}
fn precise(v: f64) -> f64 {
    (v * 1000.).round() / 1000.
}
fn sample(p: &[(f64, f64)], v: f64) -> f64 {
    let n = p.len();
    if n == 0 {
        return v;
    }
    if n < 2 {
        return p[0].1 + (v - p[0].0);
    }
    let slope = |i: usize| (p[i + 1].1 - p[i].1) / (p[i + 1].0 - p[i].0);
    let tangent = |i: usize| {
        if i == 0 {
            return slope(0);
        }
        if i >= n - 1 {
            return slope(n - 2);
        }
        let (a, b) = (slope(i - 1), slope(i));
        if a * b <= 0. {
            return 0.;
        }
        let (ha, hb) = (p[i].0 - p[i - 1].0, p[i + 1].0 - p[i].0);
        3. * (ha + hb) / ((2. * hb + ha) / a + (hb + 2. * ha) / b)
    };
    if v <= p[0].0 {
        return p[0].1 + tangent(0) * (v - p[0].0);
    }
    if v >= p[n - 1].0 {
        return p[n - 1].1 + tangent(n - 1) * (v - p[n - 1].0);
    }
    let i = (1..n - 1).rev().find(|&k| v >= p[k].0).unwrap_or(0);
    let h = p[i + 1].0 - p[i].0;
    let t = (v - p[i].0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2. * t3 - 3. * t2 + 1.) * p[i].1
        + (t3 - 2. * t2 + t) * h * tangent(i)
        + (-2. * t3 + 3. * t2) * p[i + 1].1
        + (t3 - t2) * h * tangent(i + 1)
}

/// The disc a wheel's puck sits on: at each point the colour a push there
/// adds, over a neutral ground, fading to the ground at the centre. Made
/// once per size and appearance.
fn disc(side: u32, dark: bool) -> Arc<RenderImage> {
    thread_local! {
        static DISCS: RefCell<HashMap<(u32, bool), Arc<RenderImage>>> = RefCell::new(HashMap::new());
    }
    DISCS.with(|d| {
        d.borrow_mut()
            .entry((side, dark))
            .or_insert_with(|| {
                // Quiet, as a grading tool's wheels are: a neutral ground that
                // stays grey near the centre, the colours rising toward the rim.
                let (ground, strength) = if dark { (0.25, 0.26) } else { (0.78, 0.15) };
                let r = side as f64 / 2.;
                let mut bgra = Vec::with_capacity((side * side * 4) as usize);
                for row in 0..side {
                    for col in 0..side {
                        let (x, y) = ((col as f64 + 0.5 - r) / r, (r - row as f64 - 0.5) / r);
                        let radius = x.hypot(y);
                        let edge = ((1. - radius) * r + 0.5).clamp(0., 1.);
                        let rise = radius.min(1.).powf(0.6) / radius.max(1e-6);
                        let (u, v) = (x * rise * strength, y * rise * strength);
                        let rgb = offset(u, v).map(|c| {
                            let s = (ground + c).clamp(0., 1.);
                            (s * 255.).round() as u8
                        });
                        bgra.extend([rgb[2], rgb[1], rgb[0], (edge * 255.).round() as u8]);
                    }
                }
                let image = image::RgbaImage::from_raw(side, side, bgra).expect("a disc");
                Arc::new(RenderImage::new([image::Frame::new(image)]))
            })
            .clone()
    })
}

/// How a control is drawn this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub disabled: bool,
    /// Under the pointer or being dragged.
    pub active: bool,
}

/// A colour wheel in `bounds` (centred, `metrics.grade_wheel` across): the
/// disc, a hairline rim, the centre cross and the puck.
pub fn paint_wheel(
    bounds: Bounds<Pixels>,
    puck: (f64, f64),
    look: Look,
    t: &Tokens,
    window: &mut Window,
    _cx: &mut App,
) {
    let c = &t.colors;
    let d = t
        .metrics
        .grade_wheel
        .min(f32::from(bounds.size.width))
        .min(f32::from(bounds.size.height));
    let centre = bounds.center();
    let disc_bounds = Bounds::new(centre - point(px(d / 2.), px(d / 2.)), size(px(d), px(d)));
    let alpha = if look.disabled { 0.4 } else { 1. };
    let scale = window.scale_factor();
    let image = disc(
        (d * scale).round() as u32,
        t.scheme == pm_design::Scheme::Dark,
    );
    let _ = window.paint_image(
        disc_bounds,
        disc_bounds,
        Corners::all(px(d / 2.)),
        0.,
        image,
        0,
        look.disabled,
    );
    // The rim: the edge ink's hairline just inside the disc.
    window.paint_quad(round(
        disc_bounds,
        gpui::transparent_black(),
        t.metrics.border,
        crate::kit::ink(c.text, t.translucency.edge * alpha),
    ));
    // The neutral centre: a short cross.
    let arm = t.metrics.gap;
    let line = crate::kit::ink(c.text, t.translucency.line * 2. * alpha);
    let w = t.metrics.border;
    window.paint_quad(fill(
        Bounds::new(
            centre - point(px(arm), px(w / 2.)),
            size(px(arm * 2.), px(w)),
        ),
        line,
    ));
    window.paint_quad(fill(
        Bounds::new(
            centre - point(px(w / 2.), px(arm)),
            size(px(w), px(arm * 2.)),
        ),
        line,
    ));
    // The puck: a ring that reads on every colour of the disc.
    let reach = d / 2. - t.metrics.keyframe / 2.;
    let at = centre + point(px(puck.0 as f32 * reach), px(-puck.1 as f32 * reach));
    let ring = if look.active {
        t.metrics.keyframe + t.metrics.border * 2.
    } else {
        t.metrics.keyframe
    };
    let b = Bounds::new(
        at - point(px(ring / 2.), px(ring / 2.)),
        size(px(ring), px(ring)),
    );
    // A ring that reads on every colour of the disc: white with a dark
    // keyline in dark, the text ink with a light keyline in light.
    let dark = t.scheme == pm_design::Scheme::Dark;
    let (ring_ink, keyline) = if dark {
        (0xffffff, 0x000000)
    } else {
        (c.text, 0xffffff)
    };
    window.paint_quad(round(
        b,
        gpui::transparent_black(),
        t.metrics.border * 2.,
        crate::kit::ink(ring_ink, alpha),
    ));
    let outer = Bounds::new(
        b.origin - point(px(w), px(w)),
        size(px(ring + 2. * w), px(ring + 2. * w)),
    );
    window.paint_quad(round(
        outer,
        gpui::transparent_black(),
        w,
        crate::kit::ink(keyline, 0.5 * alpha),
    ));
}

/// Where the curve editor's graph sits in `bounds`, and the mapping between
/// curve values (0 to 1 both ways) and points.
#[derive(Clone, Copy, Debug)]
pub struct Graph {
    pub area: Bounds<Pixels>,
}
impl Graph {
    pub fn new(bounds: Bounds<Pixels>) -> Self {
        Self { area: bounds }
    }
    pub fn to_point(&self, (x, y): (f64, f64)) -> Point<Pixels> {
        let a = self.area;
        point(
            a.origin.x + a.size.width * x as f32,
            a.origin.y + a.size.height * (1. - y as f32),
        )
    }
    pub fn to_value(&self, p: Point<Pixels>) -> (f64, f64) {
        let a = self.area;
        (
            f64::from((p.x - a.origin.x) / a.size.width),
            1. - f64::from((p.y - a.origin.y) / a.size.height),
        )
    }
}
/// The ink a curve of `channel` (0 Master, 1 Red, 2 Green, 3 Blue) is drawn in.
pub fn channel_ink(channel: usize, t: &Tokens) -> u32 {
    match channel {
        1 => t.colors.channel_red,
        2 => t.colors.channel_green,
        3 => t.colors.channel_blue,
        _ => t.colors.text,
    }
}

/// The curve editor: a quarter grid, the identity diagonal, the curve and
/// its points (`held` is the point being dragged; `leaving` one dragged off,
/// which goes on release).
#[allow(clippy::too_many_arguments)]
pub fn paint_curve(
    graph: Graph,
    curve: &Curve,
    channel: usize,
    held: Option<usize>,
    leaving: bool,
    look: Look,
    t: &Tokens,
    window: &mut Window,
) {
    let c = &t.colors;
    let alpha = if look.disabled { 0.4 } else { 1. };
    let w = t.metrics.border;
    // The graph's ground: a value well (the ink at the wash step), its
    // corner the controls' radius, around the inset graph.
    let pad = px(t.metrics.keyframe / 2. + w);
    let a = graph.area;
    window.paint_quad(
        PaintQuad::new(Bounds::new(
            a.origin - point(pad, pad),
            size(a.size.width + pad * 2., a.size.height + pad * 2.),
        ))
        .corner_radii(Corners::all(px(t.metrics.control_radius)))
        .background(crate::kit::ink(c.text, t.translucency.hover)),
    );
    let grid = crate::kit::ink(c.text, t.translucency.grid * alpha);
    for i in 1..4 {
        let f = i as f32 / 4.;
        let a = graph.area;
        window.paint_quad(fill(
            Bounds::new(
                point(a.origin.x + a.size.width * f - px(w / 2.), a.origin.y),
                size(px(w), a.size.height),
            ),
            grid,
        ));
        window.paint_quad(fill(
            Bounds::new(
                point(a.origin.x, a.origin.y + a.size.height * f - px(w / 2.)),
                size(a.size.width, px(w)),
            ),
            grid,
        ));
    }
    let mut identity = PathBuilder::stroke(px(w));
    identity.move_to(graph.to_point((0., 0.)));
    identity.line_to(graph.to_point((1., 1.)));
    if let Ok(path) = identity.build() {
        window.paint_path(
            path,
            crate::kit::ink(c.text, t.translucency.line * 2. * alpha),
        );
    }
    let ink = channel_ink(channel, t);
    let steps = (f32::from(graph.area.size.width) / 2.).ceil().max(16.) as usize;
    let mut line = PathBuilder::stroke(px(w * 1.5));
    for s in 0..=steps {
        let x = s as f64 / steps as f64;
        let y = curve.sample(x).clamp(-0.02, 1.02);
        let p = graph.to_point((x, y));
        if s == 0 {
            line.move_to(p);
        } else {
            line.line_to(p);
        }
    }
    if let Ok(path) = line.build() {
        window.paint_path(path, crate::kit::ink(ink, alpha));
    }
    let d = t.metrics.keyframe - 2.;
    for (i, p) in curve.points.iter().enumerate() {
        let at = graph.to_point((p.0.clamp(0., 1.), p.1.clamp(0., 1.)));
        let side = if held == Some(i) { d + 2. * w } else { d };
        let b = Bounds::new(
            at - point(px(side / 2.), px(side / 2.)),
            size(px(side), px(side)),
        );
        let gone = leaving && held == Some(i);
        let stroke = crate::kit::ink(ink, alpha * if gone { 0.3 } else { 1. });
        let inside = if held == Some(i) {
            stroke
        } else {
            crate::kit::ink(c.panel, alpha)
        };
        window.paint_quad(round(b, inside, w * 1.5, stroke));
    }
}

/// A filled rectangle.
fn fill(b: Bounds<Pixels>, color: impl Into<Background>) -> PaintQuad {
    PaintQuad::new(b).background(color)
}
/// A circle in `b` (a square), filled and stroked.
fn round(
    b: Bounds<Pixels>,
    fill: impl Into<Background>,
    width: f32,
    stroke: impl Into<Background>,
) -> PaintQuad {
    PaintQuad::new(b)
        .corner_radii(Corners::all(b.size.width / 2.))
        .background(fill)
        .border_widths(px(width))
        .border_color(stroke)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_puck_and_its_channels_round_trip_and_keep_brightness() {
        for (u, v) in [(0.3, -0.2), (-0.7, 0.4), (0., 0.9)] {
            let rgb = channels((u, v), [0.05, 0.05, 0.05], 0., [-1., 1.]);
            // The kept brightness stays: the push itself adds no luma.
            assert!((luma(rgb) - 0.05).abs() < 2e-3, "{rgb:?}");
            let back = puck(rgb, 0.);
            assert!(
                (back.0 - u).abs() < 5e-3 && (back.1 - v).abs() < 5e-3,
                "{back:?}"
            );
        }
        // Gain is neutral at one.
        assert_eq!(channels((0., 0.), [1.2, 1.2, 1.2], 1., [0., 8.]), [1.2; 3]);
        // Up the Cr axis is toward red; right along Cb toward blue.
        let up = offset(0., 1.);
        assert!(up[0] > 0. && up[1] < 0.);
        let right = offset(1., 0.);
        assert!(right[2] > 0. && right[0] == 0.);
        // A puck past the rim stays on it.
        assert!((puck([1., -1., 1.], 0.).0.hypot(puck([1., -1., 1.], 0.).1) - 1.).abs() < 1e-9);
    }

    #[test]
    fn the_curve_is_the_effects_curve() {
        let identity = Curve::from_values(&[2., 0., 0., 1., 1.]);
        for x in [0., 0.25, 0.6, 1., 1.4] {
            assert!((identity.sample(x) - x).abs() < 1e-12);
        }
        // A lifted midtone: through its points, monotone between them.
        let lifted = Curve::from_values(&[3., 0., 0., 0.5, 0.7, 1., 1.]);
        assert!((lifted.sample(0.5) - 0.7).abs() < 1e-12);
        let mut last = lifted.sample(0.);
        for s in 1..=100 {
            let y = lifted.sample(s as f64 / 100.);
            assert!(y >= last - 1e-12, "monotone");
            last = y;
        }
        // Points written out of order read in input order; two on one input
        // are the one written last.
        let shuffled = Curve::from_values(&[4., 1., 1., 0., 0., 0.5, 0.2, 0.5, 0.8]);
        assert_eq!(shuffled.ordered(), [(0., 0.), (0.5, 0.8), (1., 1.)]);
        // A run of near inputs is one point, as the effect chains them.
        let run =
            Curve::from_values(&[5., 0., 0., 0.5, 0.1, 0.5000009, 0.2, 0.5000018, 0.3, 1., 1.]);
        assert_eq!(run.ordered().len(), 3);
        assert_eq!(run.ordered()[1].1, 0.3);
        // The values written back keep the slots past the points.
        let rest = [2., 0., 0., 1., 1., 0.3, 0.3];
        let v = Curve {
            points: vec![(0., 0.), (1., 1.)],
        }
        .values(&rest);
        assert_eq!(v.len(), 1 + 2 * CURVE_POINTS);
        assert_eq!(&v[..7], &[2., 0., 0., 1., 1., 0.3, 0.3]);
    }
}
