//! The colour grading controls the panels draw: a colour wheel (a puck over a
//! disc of the colours it pushes toward) and a curve editor (the monotone
//! cubic the Color Curves effect renders, through its points). The values
//! are the Color Wheels and Color Curves parameters; nothing here is a
//! second model of the grade.
use gpui::{
    App, Background, Bounds, BoxShadow, ContentMask, Corners, PaintQuad, PathBuilder, Pixels,
    Point, RenderImage, Window, linear_color_stop, linear_gradient, point, px, size,
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
                // Quiet and matte, as a grading tool's wheels are: a neutral
                // ground that stays grey near the centre, the colours rising
                // toward the rim.
                let (ground, strength) = if dark { (0.22, 0.24) } else { (0.84, 0.14) };
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

const WHITE: u32 = 0xffffff;
const BLACK: u32 = 0x000000;

/// A rim of light inside `area`'s edge (the timeline strip's): a ring in
/// `side`, the band over the top edge and its corners again in `top`, so the
/// light is brightest there.
fn rim(
    area: Bounds<Pixels>,
    radius: Pixels,
    width: Pixels,
    side: (u32, f32),
    top: (u32, f32),
    window: &mut Window,
) {
    let ring = |(rgb, alpha): (u32, f32)| {
        PaintQuad::new(area)
            .border_color(crate::kit::ink(rgb, alpha))
            .border_widths(width)
            .corner_radii(radius)
    };
    window.paint_quad(ring(side));
    let band = Bounds::new(area.origin, size(area.size.width, radius.max(width * 4.)));
    let seen = window.content_mask().bounds.intersect(&band);
    if seen.size.width > px(0.) && seen.size.height > px(0.) {
        window.with_content_mask(
            Some(ContentMask {
                bounds: band,
                fade_out: Default::default(),
            }),
            |window| window.paint_quad(ring(top)),
        );
    }
}
fn shadow(rgb: u32, alpha: f32, y: f32, blur: f32) -> BoxShadow {
    BoxShadow {
        color: crate::kit::ink(rgb, alpha).into(),
        offset: point(px(0.), px(y)),
        blur_radius: px(blur),
        spread_radius: px(0.),
        inset: false,
    }
}
fn vertical(top: (u32, f32), bottom: (u32, f32)) -> Background {
    linear_gradient(
        180.,
        linear_color_stop(crate::kit::ink(top.0, top.1), 0.),
        linear_color_stop(crate::kit::ink(bottom.0, bottom.1), 1.),
    )
}

/// The ground a Colour panel section stands on: a machined plate, lit from
/// its top edge.
pub fn paint_plate(bounds: Bounds<Pixels>, t: &Tokens, window: &mut Window) {
    let dark = t.scheme == pm_design::Scheme::Dark;
    let radius = px(t.metrics.panel_radius);
    let corners = Corners::all(radius);
    let w = px(t.metrics.border);
    let strong = if t.accessibility.increase_contrast {
        2.
    } else {
        1.
    };
    window.paint_drop_shadows(
        bounds,
        corners,
        0.,
        &[if dark {
            shadow(BLACK, 0.45, 1.5, 4.)
        } else {
            shadow(BLACK, 0.12, 1., 3.)
        }],
    );
    let body = if dark {
        vertical((WHITE, 0.085), (WHITE, 0.055))
    } else {
        vertical((WHITE, 1.), (WHITE, 0.82))
    };
    window.paint_quad(
        PaintQuad::new(bounds)
            .background(body)
            .corner_radii(corners),
    );
    // A machined edge: a light chamfer along the top, a darker side.
    let (side, top) = if dark {
        ((WHITE, 0.05 * strong), (WHITE, 0.16 * strong))
    } else {
        ((BLACK, 0.10 * strong), (WHITE, 1.))
    };
    rim(bounds, radius, w, side, top, window);
}

/// The colour a push at (`u`, `v`) in the unit disc shows (the puck's own
/// ink: the push at a glance), bright enough to read on the disc.
fn push_colour(puck: (f64, f64), dark: bool) -> u32 {
    let ground = if dark { 0.62 } else { 0.55 };
    let rgb = offset(puck.0 * 0.45, puck.1 * 0.45)
        .map(|c| ((ground + c).clamp(0., 1.) * 255.).round() as u32);
    (rgb[0] << 16) | (rgb[1] << 8) | rgb[2]
}
/// The angle (radians, counter-clockwise from the +Cb axis) the primary or
/// secondary `rgb` lies at on the wheel.
fn hue_angle(rgb: [f64; 3]) -> f64 {
    let y = luma(rgb);
    let (u, v) = ((rgb[2] - y) / 1.8556, (rgb[0] - y) / 1.5748);
    v.atan2(u)
}
/// An arc of a circle around `centre`, `radius` across, from `from` through
/// `sweep` radians (counter-clockwise positive, screen y down).
fn arc(
    centre: Point<Pixels>,
    radius: f32,
    from: f64,
    sweep: f64,
    width: f32,
) -> Option<gpui::Path<Pixels>> {
    let steps = ((sweep.abs() * radius as f64 / 2.).ceil() as usize).max(2);
    let mut path = PathBuilder::stroke(px(width));
    for s in 0..=steps {
        let a = from + sweep * s as f64 / steps as f64;
        let p = centre
            + point(
                px((a.cos() as f32) * radius),
                px(-(a.sin() as f32) * radius),
            );
        if s == 0 {
            path.move_to(p);
        } else {
            path.line_to(p);
        }
    }
    path.build().ok()
}
/// A round rim lit from above: a band round `radius`, brightest at the top
/// and fading to nothing by the sides (a round object's light, where a
/// straight band would end in a bright stub).
fn lit_ring(centre: Point<Pixels>, radius: f32, width: f32, alpha: f32, window: &mut Window) {
    const PARTS: usize = 12;
    let span = std::f64::consts::PI * 0.85;
    let half = std::f64::consts::FRAC_PI_2;
    for k in 0..PARTS {
        let from = half - span / 2. + span * k as f64 / PARTS as f64;
        let mid = from + span / PARTS as f64 / 2. - half;
        let fall = (mid / (span / 2.) * half).cos().max(0.).powf(1.5) as f32;
        if let Some(p) = arc(centre, radius, from, span / PARTS as f64 + 0.01, width) {
            window.paint_path(p, crate::kit::ink(WHITE, alpha * fall));
        }
    }
}

/// A wheel's diameter in `bounds`: the square it has (the kit sizes it from
/// `metrics.grade_wheel` up to `metrics.grade_wheel_max` with the panel).
pub fn wheel_diameter(bounds: Bounds<Pixels>) -> f32 {
    f32::from(bounds.size.width).min(f32::from(bounds.size.height))
}
/// How far from a wheel's centre the puck's centre goes at the rim, for a
/// wheel `d` across: inside the bezel, the puck wholly on the disc.
pub fn wheel_reach(d: f32, t: &Tokens) -> f32 {
    (d / 2. - bezel(d) - t.metrics.keyframe / 2. - 1.).max(1.)
}
fn bezel(d: f32) -> f32 {
    (d * 0.065).max(5.)
}

/// A colour wheel centred in `bounds`: a machined bezel with notches where
/// the primaries and secondaries lie (and the master as an arc from the top:
/// clockwise up, counter-clockwise down), the matte recessed disc, the
/// neutral mark, a line from neutral to the puck, and the puck, a knob.
/// `master` is the master's push from neutral, -1 to 1 of its range.
pub fn paint_wheel(
    bounds: Bounds<Pixels>,
    puck: (f64, f64),
    master: Option<f64>,
    look: Look,
    t: &Tokens,
    window: &mut Window,
    _cx: &mut App,
) {
    let c = &t.colors;
    let dark = t.scheme == pm_design::Scheme::Dark;
    let strong = if t.accessibility.increase_contrast {
        1.8
    } else {
        1.
    };
    let d = wheel_diameter(bounds);
    let centre = bounds.center();
    let circle = |diameter: f32| {
        Bounds::new(
            centre - point(px(diameter / 2.), px(diameter / 2.)),
            size(px(diameter), px(diameter)),
        )
    };
    let alpha = if look.disabled { 0.4 } else { 1. };
    let w = t.metrics.border;
    let bezel = bezel(d);
    let outer = circle(d);
    let inner_d = d - bezel * 2.;
    let disc_bounds = circle(inner_d);
    let round_of = |b: Bounds<Pixels>| Corners::all(b.size.width / 2.);
    window.paint_drop_shadows(
        outer,
        round_of(outer),
        0.,
        &[if dark {
            shadow(BLACK, 0.6 * alpha, 2., 5.)
        } else {
            shadow(BLACK, 0.16 * alpha, 1.5, 4.)
        }],
    );
    let body = if dark {
        vertical((WHITE, 0.15 * alpha), (WHITE, 0.06 * alpha))
    } else {
        vertical((WHITE, alpha), (0xe2e2e2, alpha))
    };
    window.paint_quad(
        PaintQuad::new(outer)
            .background(body)
            .corner_radii(round_of(outer)),
    );
    let edge = if dark { (BLACK, 0.6) } else { (BLACK, 0.16) };
    window.paint_quad(round(
        outer,
        gpui::transparent_black(),
        w,
        crate::kit::ink(edge.0, edge.1 * strong * alpha),
    ));
    let light = if dark { 0.3 } else { 1. };
    lit_ring(centre, d / 2. - w * 1.5, w, light * alpha, window);
    // Notches where each primary and secondary lies: the direction a push
    // takes reads at the edge.
    for hue in [
        [1., 0., 0.],
        [1., 1., 0.],
        [0., 1., 0.],
        [0., 1., 1.],
        [0., 0., 1.],
        [1., 0., 1.],
    ] {
        let a = hue_angle(hue);
        let rgb = ((hue[0] * 200. + 40.) as u32) << 16
            | ((hue[1] * 200. + 40.) as u32) << 8
            | (hue[2] * 200. + 40.) as u32;
        let (cos, sin) = (a.cos() as f32, -(a.sin() as f32));
        let r0 = d / 2. - bezel * 0.8;
        let r1 = d / 2. - bezel * 0.25;
        let mut tick = PathBuilder::stroke(px(1.5));
        tick.move_to(centre + point(px(cos * r0), px(sin * r0)));
        tick.line_to(centre + point(px(cos * r1), px(sin * r1)));
        if let Ok(path) = tick.build() {
            window.paint_path(path, crate::kit::ink(rgb, 0.9 * alpha));
        }
    }
    let scale = window.scale_factor();
    let image = disc((inner_d * scale).round() as u32, dark);
    let _ = window.paint_image(
        disc_bounds,
        disc_bounds,
        round_of(disc_bounds),
        0.,
        image,
        0,
        look.disabled,
    );
    window.paint_inset_shadows(
        disc_bounds,
        round_of(disc_bounds),
        0.,
        &[BoxShadow {
            inset: true,
            ..shadow(BLACK, if dark { 0.7 } else { 0.28 } * alpha, 3., 7.)
        }],
    );
    // The master on the bezel: an arc from the top, its length the push.
    let r = d / 2. - bezel / 2.;
    let ink = crate::kit::ink(if dark { WHITE } else { c.text }, 0.22 * alpha * strong);
    // Neutral: a short mark at the top.
    if let Some(p) = arc(
        centre,
        r,
        std::f64::consts::FRAC_PI_2 + 0.035,
        -0.07,
        bezel * 0.45,
    ) {
        window.paint_path(p, ink);
    }
    if let Some(m) = master.filter(|m| m.abs() > 1e-4) {
        // On a square-root scale: an everyday push (a tenth of the range) is
        // a third of the way round, the end of the range most of it; the
        // direction and the order of two pushes stay true.
        let m = m.clamp(-1., 1.);
        let sweep = -m.signum() * m.abs().sqrt() * std::f64::consts::PI * 0.8;
        if let Some(p) = arc(centre, r, std::f64::consts::FRAC_PI_2, sweep, bezel * 0.45) {
            let ink = if dark { WHITE } else { c.text };
            window.paint_path(p, crate::kit::ink(ink, 0.85 * alpha));
        }
    }
    // The neutral centre: a short cross.
    let arm = t.metrics.gap;
    let line = crate::kit::ink(c.text, t.translucency.line * 2. * alpha * strong);
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
    let reach = wheel_reach(d, t);
    let at = centre + point(px(puck.0 as f32 * reach), px(-puck.1 as f32 * reach));
    if puck.0.hypot(puck.1) > 0.02 {
        // The push from neutral: a line out to the puck.
        let mut tail = PathBuilder::stroke(px(1.5));
        tail.move_to(centre);
        tail.line_to(at);
        if let Ok(path) = tail.build() {
            let ink = if dark { WHITE } else { c.text };
            window.paint_path(path, crate::kit::ink(ink, 0.55 * alpha));
        }
    }
    // The puck has weight: a shadow under it, a light body, its push colour
    // at its centre.
    let side = t.metrics.keyframe + 3. + if look.active { 3. } else { 0. };
    let b = Bounds::new(
        at - point(px(side / 2.), px(side / 2.)),
        size(px(side), px(side)),
    );
    window.paint_drop_shadows(
        b,
        round_of(b),
        0.,
        &[shadow(
            BLACK,
            0.55 * alpha,
            1.5,
            if look.active { 6. } else { 3. },
        )],
    );
    let body = if dark {
        vertical((0xf4f4f4, alpha), (0xcfcfcf, alpha))
    } else {
        vertical((WHITE, alpha), (0xe2e2e2, alpha))
    };
    window.paint_quad(PaintQuad::new(b).background(body).corner_radii(round_of(b)));
    window.paint_quad(round(
        b,
        gpui::transparent_black(),
        w,
        crate::kit::ink(BLACK, 0.35 * alpha),
    ));
    let dot = side * 0.42;
    let core = Bounds::new(
        at - point(px(dot / 2.), px(dot / 2.)),
        size(px(dot), px(dot)),
    );
    window.paint_quad(
        PaintQuad::new(core)
            .background(crate::kit::ink(push_colour(puck, dark), alpha))
            .corner_radii(round_of(core)),
    );
    // A keyline outside, so the puck reads on every colour.
    window.paint_quad(round(
        b.dilate(px(w)),
        gpui::transparent_black(),
        w,
        crate::kit::ink(BLACK, 0.3 * alpha),
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

/// The curve editor: its well, a quarter grid, the identity diagonal, the
/// curve and its points (`held` is the point being dragged; `leaving` one
/// dragged off, which goes on release).
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
    // A well sunk into the plate: darker than it, shaded along its top
    // inside, its lower edge catching the light.
    window.paint_quad(
        PaintQuad::new(well)
            .background(crate::kit::ink(BLACK, if dark { 0.38 } else { 0.06 }))
            .corner_radii(corners),
    );
    window.paint_inset_shadows(
        well,
        corners,
        0.,
        &[BoxShadow {
            inset: true,
            ..shadow(BLACK, if dark { 0.6 } else { 0.18 }, 2., 4.)
        }],
    );
    let lip = Bounds::new(
        point(well.origin.x + corners.bottom_left, well.bottom() - px(w)),
        size(well.size.width - corners.bottom_left * 2., px(w)),
    );
    window.paint_quad(fill(
        lip,
        crate::kit::ink(WHITE, if dark { 0.10 } else { 0.9 } * strong),
    ));
    let grid = crate::kit::ink(c.text, t.translucency.grid * alpha * strong);
    for i in 1..4 {
        let f = i as f32 / 4.;
        let x = point(a.origin.x + a.size.width * f - px(w / 2.), a.origin.y);
        let y = point(a.origin.x, a.origin.y + a.size.height * f - px(w / 2.));
        window.paint_quad(fill(Bounds::new(x, size(px(w), a.size.height)), grid));
        window.paint_quad(fill(Bounds::new(y, size(a.size.width, px(w))), grid));
    }
    let mut identity = PathBuilder::stroke(px(w));
    identity.move_to(graph.to_point((0., 0.)));
    identity.line_to(graph.to_point((1., 1.)));
    if let Ok(path) = identity.build() {
        window.paint_path(
            path,
            crate::kit::ink(c.text, t.translucency.line * 2. * alpha * strong),
        );
    }
    let ink = channel_ink(channel, t);
    let steps = (f32::from(graph.area.size.width) / 2.).ceil().max(16.) as usize;
    let mut line = PathBuilder::stroke(px(w * 2.));
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
    // Points you want to grab: knobs with a shadow, filled while held.
    let d = t.metrics.keyframe;
    for (i, p) in curve.points.iter().enumerate() {
        let at = graph.to_point((p.0.clamp(0., 1.), p.1.clamp(0., 1.)));
        let side = if held == Some(i) { d + 2. * w + 2. } else { d };
        let b = Bounds::new(
            at - point(px(side / 2.), px(side / 2.)),
            size(px(side), px(side)),
        );
        let gone = leaving && held == Some(i);
        let fade = alpha * if gone { 0.3 } else { 1. };
        window.paint_drop_shadows(
            b,
            Corners::all(b.size.width / 2.),
            0.,
            &[shadow(BLACK, 0.55 * fade, 1., 2.5)],
        );
        let body = if held == Some(i) {
            crate::kit::ink(ink, fade)
        } else {
            crate::kit::ink(if dark { 0xf2f2f2 } else { WHITE }, fade)
        };
        window.paint_quad(round(b, body, w * 1.5, crate::kit::ink(ink, fade)));
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
