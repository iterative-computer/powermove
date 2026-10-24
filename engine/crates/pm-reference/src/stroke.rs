//! The exact region a stroke covers (Canvas 2D "trace a path", SVG
//! stroke shape), built from its definition rather than by a stroker.
//!
//! A stroke of half-width `h` is the union of: a band `h` either side of
//! every straight piece; at each corner the authored join (a disk of
//! radius `h` for `round`, the triangle to the two outer offset points for
//! `bevel`, that triangle and the miter tip for `miter` within the miter
//! limit, else bevel); and at each open end the authored cap (`round` a
//! disk, `square` a half-square, `butt` nothing). Curves are first split
//! into straight pieces within `tolerance`. Between two pieces of one curve
//! the band's edges meet where the offset lines cross, so the pieces tile
//! without overlapping (overlaps cost coverage precision in a cell); where
//! the curvature radius is below `h` the pieces end square and the bevel
//! triangle closes the outer gap. Either departs from the exact offset
//! curve by about `h · tolerance / ρ` for a curvature radius `ρ`.
//!
//! Every primitive is wound the same way, so the union is the nonzero fill
//! of the returned path ([`crate::coverage::fill`] with `NonZero`).

use kurbo::{BezPath, Cap, Join, PathEl, Point, Stroke, Vec2};

/// The region a stroke covers ([`outline`]).
pub struct Region {
    /// Nonzero-filled, in the path's space.
    pub path: BezPath,
    /// Some pieces overlap at the region's edge (a curvature radius below
    /// the half width): coverage counts their overlap within a cell, so
    /// cover it with finer cells.
    pub folds: bool,
}

/// The region `style` strokes along `path`; `None` for dashed strokes (not
/// modelled).
pub fn outline(path: &BezPath, style: &Stroke, tolerance: f64) -> Option<Region> {
    if !style.dash_pattern.is_empty() {
        return None;
    }
    let h = style.width.abs() / 2.0;
    let mut out = Region {
        path: BezPath::new(),
        folds: false,
    };
    if !(h > 0.0) {
        return Some(out);
    }
    for poly in polylines(path, tolerance) {
        out.folds |= stroke_polyline(&poly, style, h, tolerance, &mut out.path);
    }
    Some(out)
}

/// One subpath as straight pieces: its points, whether each is a corner
/// (where two path segments meet at an angle, and the authored join
/// applies), and closure.
struct Polyline {
    points: Vec<Point>,
    corner: Vec<bool>,
    /// The path's exact tangents arriving at and leaving each point where a
    /// path segment ends or starts (`None` inside a curve's pieces): the
    /// pieces end, and the joins and caps turn, on them, not on chords.
    tangent_in: Vec<Option<Vec2>>,
    tangent_out: Vec<Option<Vec2>>,
    closed: bool,
}

/// Whether two tangents continue one another (a smooth joint: no join).
fn smooth(a: Vec2, b: Vec2) -> bool {
    let (la, lb) = (a.hypot(), b.hypot());
    la > 0.0 && lb > 0.0 && a.dot(b) > 0.0 && (a.cross(b) / (la * lb)).abs() < 1e-6
}

/// A segment's start and end tangents (`None` when degenerate).
fn tangents(from: Point, el: PathEl) -> (Option<Vec2>, Option<Vec2>) {
    let first = |ps: &[Point]| ps.iter().map(|p| *p - from).find(|v| v.hypot() > 0.0);
    match el {
        PathEl::LineTo(p) => {
            let d = (p - from).hypot().gt(&0.0).then_some(p - from);
            (d, d)
        }
        PathEl::QuadTo(c, p) => (first(&[c, p]), [c, from].iter().map(|q| p - *q).find(|v| v.hypot() > 0.0)),
        PathEl::CurveTo(c1, c2, p) => (
            first(&[c1, c2, p]),
            [c2, c1, from].iter().map(|q| p - *q).find(|v| v.hypot() > 0.0),
        ),
        _ => (None, None),
    }
}

fn polylines(path: &BezPath, tolerance: f64) -> Vec<Polyline> {
    let mut out = Vec::new();
    let mut cur: Option<Polyline> = None;
    let mut last = Point::ZERO;
    // The end tangent of the segment that ended at `last`, and the start
    // tangent of the subpath's first segment.
    let (mut incoming, mut first_out): (Option<Vec2>, Option<Vec2>) = (None, None);
    let push = |cur: &mut Option<Polyline>, p: Point, corner: bool| {
        let poly = cur.as_mut().expect("a subpath");
        if poly.points.last() != Some(&p) {
            poly.points.push(p);
            poly.corner.push(corner);
            poly.tangent_in.push(None);
            poly.tangent_out.push(None);
        } else if corner {
            *poly.corner.last_mut().unwrap() = true;
        }
    };
    // A segment starting at `last` with tangent `t`: a smooth joint there is
    // no corner.
    let joint = |cur: &mut Option<Polyline>, incoming: Option<Vec2>, t: Option<Vec2>| {
        if let (Some(a), Some(b), Some(poly)) = (incoming, t, cur.as_mut())
            && smooth(a, b)
            && poly.points.len() > 1
        {
            *poly.corner.last_mut().unwrap() = false;
        }
    };
    let start = |p: Point| Polyline {
        points: vec![p],
        corner: vec![true],
        tangent_in: vec![None],
        tangent_out: vec![None],
        closed: false,
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                out.extend(cur.take());
                cur = Some(start(p));
                last = p;
                (incoming, first_out) = (None, None);
            }
            PathEl::LineTo(p) | PathEl::QuadTo(_, p) | PathEl::CurveTo(_, _, p) => {
                if cur.is_none() {
                    cur = Some(start(last));
                }
                let (t0, t1) = tangents(last, *el);
                if t0.is_none() {
                    continue;
                }
                joint(&mut cur, incoming, t0);
                if first_out.is_none() {
                    first_out = t0;
                }
                if let Some(poly) = cur.as_mut() {
                    *poly.tangent_out.last_mut().unwrap() = t0;
                }
                if !matches!(el, PathEl::LineTo(_)) {
                    let mut seg = BezPath::new();
                    seg.move_to(last);
                    seg.push(*el);
                    kurbo::flatten(seg.iter(), tolerance, |e| {
                        if let PathEl::LineTo(q) = e {
                            push(&mut cur, q, false);
                        }
                    });
                }
                push(&mut cur, p, true);
                if let Some(poly) = cur.as_mut() {
                    *poly.tangent_in.last_mut().unwrap() = t1;
                }
                (last, incoming) = (p, t1);
            }
            PathEl::ClosePath => {
                if let Some(mut poly) = cur.take() {
                    let origin = poly.points[0];
                    // The closing line, when the subpath does not end where it
                    // began.
                    let mut into_start = incoming;
                    if last != origin {
                        let mut c = Some(poly);
                        joint(&mut c, incoming, Some(origin - last));
                        poly = c.unwrap();
                        into_start = Some(origin - last);
                        *poly.tangent_out.last_mut().unwrap() = into_start;
                    } else if poly.points.len() > 1 {
                        poly.points.pop();
                        poly.corner.pop();
                        poly.tangent_in.pop();
                        poly.tangent_out.pop();
                    }
                    poly.tangent_in[0] = into_start;
                    if let (Some(a), Some(b)) = (into_start, first_out) {
                        poly.corner[0] = !smooth(a, b);
                    }
                    poly.closed = true;
                    out.push(poly);
                    // A segment after `closePath` starts at the subpath's start.
                    cur = Some(start(origin));
                    last = origin;
                    (incoming, first_out) = (None, None);
                }
            }
        }
    }
    out.extend(cur);
    out.retain(|p| p.points.len() > 1);
    out
}

/// Appends `points` as a closed contour wound counterclockwise (positive
/// area in y-down pixel space means clockwise on screen; any one sense for
/// all primitives makes the union the nonzero fill).
fn polygon(out: &mut BezPath, points: &[Point]) {
    let area: f64 = (0..points.len())
        .map(|i| points[i].to_vec2().cross(points[(i + 1) % points.len()].to_vec2()))
        .sum();
    if area == 0.0 {
        return;
    }
    let mut ordered: Vec<Point> = points.to_vec();
    if area < 0.0 {
        ordered.reverse();
    }
    out.move_to(ordered[0]);
    for p in &ordered[1..] {
        out.line_to(*p);
    }
    out.close_path();
}

/// The circular sector about `center` from offset `from` to offset `to`
/// (radius `|from|`, the shorter way round): a round join's outer wedge or
/// a round cap's half disk. It shares its straight edges with the pieces it
/// meets, so it adds no overlap at the region's edge.
fn sector(out: &mut BezPath, center: Point, from: Vec2, to: Vec2, tolerance: f64) {
    let r = from.hypot();
    let (a0, mut a1) = (from.atan2(), to.atan2());
    let mut sweep = a1 - a0;
    while sweep > core::f64::consts::PI {
        sweep -= core::f64::consts::TAU;
    }
    while sweep < -core::f64::consts::PI {
        sweep += core::f64::consts::TAU;
    }
    if sweep.abs() < 1e-12 || !(r > 0.0) {
        return;
    }
    // Chords within `tolerance / 4` of the arc.
    let step = 2.0 * (1.0 - (tolerance / 4.0 / r).min(1.0)).acos();
    let n = ((sweep.abs() / step.max(1e-6)).ceil() as usize).clamp(1, 1 << 16);
    a1 = a0 + sweep;
    let mut points = vec![center];
    points.extend((0..=n).map(|k| {
        let a = a0 + (a1 - a0) * k as f64 / n as f64;
        center + Vec2::new(a.cos(), a.sin()) * r
    }));
    polygon(out, &points);
}

fn normal(d: Vec2) -> Vec2 {
    Vec2::new(-d.y, d.x) / d.hypot()
}

/// Appends the polyline's stroke region; whether any smooth joint folds.
fn stroke_polyline(poly: &Polyline, style: &Stroke, h: f64, tolerance: f64, out: &mut BezPath) -> bool {
    let p = &poly.points;
    let n = p.len();
    let pieces = if poly.closed { n } else { n - 1 };
    let dir = |i: usize| p[(i + 1) % n] - p[i];
    // At a smooth joint whose inner offset stays inside both pieces, the two
    // pieces share their offset points (where their offset lines meet), so
    // their quads tile the band without overlapping; elsewhere each piece
    // ends square and a join primitive closes the gap.
    let shared: Vec<Option<Vec2>> = (0..n)
        .map(|i| {
            let has_join = poly.closed || (i > 0 && i < n - 1);
            // A corner whose turn moves no join's edge by more than the
            // tolerance (h·θ²/8) joins as a smooth joint.
            let turn = poly.tangent_in[i]
                .zip(poly.tangent_out[i])
                .map_or(0.0, |(a, b)| (a.dot(b) / (a.hypot() * b.hypot())).clamp(-1.0, 1.0).acos());
            if !has_join || (poly.corner[i] && h * turn * turn / 8.0 > tolerance) {
                return None;
            }
            let (d0, d1) = (dir((i + n - 1) % n), dir(i));
            let (n0, n1) = (normal(d0), normal(d1));
            let sum = n0 + n1;
            let reach = h * (1.0 - n0.dot(n1)).max(0.0).sqrt() / (1.0 + n0.dot(n1)).max(1e-12).sqrt();
            (reach <= 0.5 * d0.hypot().min(d1.hypot())).then(|| sum * (2.0 * h / sum.hypot2()))
        })
        .collect();
    let folds = (0..n).any(|i| {
        let has_join = poly.closed || (i > 0 && i < n - 1);
        shared[i].is_none() && has_join && !poly.corner[i]
    });
    // Where a piece ends square (a corner, an open end), it ends across the
    // path's exact tangent there.
    let leaving = |i: usize| poly.tangent_out[i].unwrap_or_else(|| dir(i));
    let arriving = |i: usize| poly.tangent_in[i].unwrap_or_else(|| dir((i + n - 1) % n));
    for i in 0..pieces {
        let j = (i + 1) % n;
        let (a, b) = (p[i], p[j]);
        let oa = shared[i].unwrap_or_else(|| normal(leaving(i)) * h);
        let ob = shared[j].unwrap_or_else(|| normal(arriving(j)) * h);
        polygon(out, &[a + oa, b + ob, b - ob, a - oa]);
    }
    let joins: Vec<usize> = if poly.closed { (0..n).collect() } else { (1..n - 1).collect() };
    for i in joins.into_iter().filter(|&i| shared[i].is_none()) {
        let at = p[i];
        let (d0, d1) = (arriving(i), leaving(i));
        let turn = d0.cross(d1);
        // The outer side: left of the travel direction when turning right.
        let side = if turn > 0.0 { -1.0 } else { 1.0 };
        let (o0, o1) = (normal(d0) * h * side, normal(d1) * h * side);
        let cos = (d0.dot(d1) / (d0.hypot() * d1.hypot())).clamp(-1.0, 1.0);
        let angle = cos.acos();
        let bevel = [at, at + o0, at + o1];
        if !poly.corner[i] || angle < 1e-6 {
            polygon(out, &bevel);
            continue;
        }
        match style.join {
            Join::Round => sector(out, at, o0, o1, tolerance),
            Join::Bevel => polygon(out, &bevel),
            Join::Miter => {
                // Miter length over line width: 1 / sin(θ/2), θ the angle
                // between the pieces (π − the turn).
                let half = (core::f64::consts::PI - angle) / 2.0;
                if half > 0.0 && 1.0 / half.sin() <= style.miter_limit {
                    let tip = at + (o0 + o1).normalize() * (h / half.sin());
                    polygon(out, &[at, at + o0, tip, at + o1]);
                } else {
                    polygon(out, &bevel);
                }
            }
        }
    }
    if !poly.closed {
        for (end, dir, cap) in [
            (p[0], -leaving(0).normalize(), style.start_cap),
            (p[n - 1], arriving(n - 1).normalize(), style.end_cap),
        ] {
            match cap {
                Cap::Butt => {}
                Cap::Round => {
                    // Two quarter sectors: the half disk beyond the end.
                    let o = Vec2::new(-dir.y, dir.x) * h;
                    sector(out, end, o, dir * h, tolerance);
                    sector(out, end, dir * h, -o, tolerance);
                }
                Cap::Square => {
                    let o = Vec2::new(-dir.y, dir.x) * h;
                    let f = dir * h;
                    polygon(out, &[end + o, end + o + f, end - o + f, end - o]);
                }
            }
        }
    }
    folds
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coverage::{FillRule, fill};
    use kurbo::{Circle, Rect, Shape};

    fn area(path: &BezPath) -> f64 {
        fill(path, FillRule::NonZero, 4, 400, 400).alpha.iter().sum()
    }

    #[test]
    fn a_square_stroke_is_its_frame() {
        let r = Rect::new(100.0, 100.0, 200.0, 180.0).to_path(1e-9);
        let miter = Stroke::new(10.0).with_join(Join::Miter);
        // Outer 110 × 90 minus inner 90 × 70.
        let a = area(&outline(&r, &miter, 1e-4).unwrap().path);
        assert!((a - (110.0 * 90.0 - 90.0 * 70.0)).abs() < 1e-6, "{a}");
        // Round corners: the outer corners lose (1 − π/4)·25 each.
        let round = Stroke::new(10.0).with_join(Join::Round);
        let a = area(&outline(&r, &round, 1e-4).unwrap().path);
        let expected = 110.0 * 90.0 - 90.0 * 70.0 - 4.0 * 25.0 * (1.0 - core::f64::consts::FRAC_PI_4);
        assert!((a - expected).abs() < 1e-2, "{a} {expected}");
    }

    #[test]
    fn a_circle_stroke_is_an_annulus() {
        let c = Circle::new((200.0, 200.0), 80.0).to_path(1e-9);
        // Per pixel against the exact annulus (two circles of opposite
        // winding, no overlapping contours) at the gates' 16 cells.
        let stroke = fill(&outline(&c, &Stroke::new(12.0), 1e-4).unwrap().path, FillRule::NonZero, 16, 400, 400);
        let mut annulus = Circle::new((200.0, 200.0), 86.0).to_path(1e-9);
        annulus.extend(Circle::new((200.0, 200.0), 74.0).to_path(1e-9).reverse_subpaths());
        let exact = fill(&annulus, FillRule::NonZero, 16, 400, 400);
        let mut worst: f64 = 0.0;
        for y in 100..300 {
            for x in 100..300 {
                worst = worst.max((stroke.at(x, y) - exact.at(x, y)).abs());
            }
        }
        assert!(worst * 255.0 < 0.5, "{} CV", worst * 255.0);
    }

    #[test]
    fn caps_extend_open_lines() {
        let mut l = BezPath::new();
        l.move_to((100.0, 200.0));
        l.line_to((300.0, 200.0));
        let butt = area(&outline(&l, &Stroke::new(10.0).with_caps(Cap::Butt), 1e-4).unwrap().path);
        assert!((butt - 2000.0).abs() < 1e-6, "{butt}");
        let square = area(&outline(&l, &Stroke::new(10.0).with_caps(Cap::Square), 1e-4).unwrap().path);
        assert!((square - 2100.0).abs() < 1e-6, "{square}");
        let round = area(&outline(&l, &Stroke::new(10.0).with_caps(Cap::Round), 1e-4).unwrap().path);
        assert!((round - (2000.0 + core::f64::consts::PI * 25.0)).abs() < 1e-2, "{round}");
    }

    #[test]
    fn sharp_miters_fall_back_to_bevels() {
        let mut v = BezPath::new();
        v.move_to((100.0, 100.0));
        v.line_to((200.0, 300.0));
        v.line_to((300.0, 100.0));
        let style = Stroke::new(10.0).with_join(Join::Miter);
        let within = area(&outline(&v, &style.clone().with_miter_limit(10.0), 1e-4).unwrap().path);
        let beyond = area(&outline(&v, &style.with_miter_limit(1.5), 1e-4).unwrap().path);
        assert!(within > beyond + 10.0, "{within} {beyond}");
    }
}
