//! Independent f64 dash partition for the stroke reference.
//!
//! The dash array defines alternating painted and unpainted distances along
//! each subpath. Distances on curves are integrated from the Bézier
//! derivative; interval endpoints are found by bisection and the curve is
//! split geometrically. No engine or kurbo dash implementation is used.

use kurbo::{BezPath, CubicBez, Line, ParamCurve, PathEl, PathSeg, Point, QuadBez, Vec2};

pub(super) struct Dashes {
    pub path: BezPath,
    /// A zero-length painted interval has two caps at one point.
    pub dots: Vec<(Point, Vec2)>,
}

#[derive(Default)]
struct Subpath {
    start: Point,
    segments: Vec<PathSeg>,
    closed: bool,
}

fn subpaths(path: &BezPath) -> Vec<Subpath> {
    let (mut out, mut cur) = (Vec::new(), Subpath::default());
    let mut last = Point::ZERO;
    for &el in path.elements() {
        match el {
            PathEl::MoveTo(p) => {
                if !cur.segments.is_empty() {
                    out.push(cur);
                }
                cur = Subpath {
                    start: p,
                    ..Subpath::default()
                };
                last = p;
            }
            PathEl::LineTo(p) => {
                cur.segments.push(PathSeg::Line(Line::new(last, p)));
                last = p;
            }
            PathEl::QuadTo(c, p) => {
                cur.segments.push(PathSeg::Quad(QuadBez::new(last, c, p)));
                last = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                cur.segments
                    .push(PathSeg::Cubic(CubicBez::new(last, c1, c2, p)));
                last = p;
            }
            PathEl::ClosePath => {
                if !cur.segments.is_empty() {
                    let origin = cur.start;
                    if last != cur.start {
                        cur.segments.push(PathSeg::Line(Line::new(last, cur.start)));
                    }
                    cur.closed = true;
                    out.push(cur);
                    cur = Subpath {
                        start: origin,
                        ..Subpath::default()
                    };
                }
                last = cur.start;
            }
        }
    }
    if !cur.segments.is_empty() {
        out.push(cur);
    }
    out
}

fn speed(seg: PathSeg, t: f64) -> f64 {
    let d = match seg {
        PathSeg::Line(s) => s.p1 - s.p0,
        PathSeg::Quad(s) => ((s.p1 - s.p0) * (1.0 - t) + (s.p2 - s.p1) * t) * 2.0,
        PathSeg::Cubic(s) => {
            ((s.p1 - s.p0) * (1.0 - t) * (1.0 - t)
                + (s.p2 - s.p1) * 2.0 * (1.0 - t) * t
                + (s.p3 - s.p2) * t * t)
                * 3.0
        }
    };
    d.hypot()
}

fn simpson(seg: PathSeg, a: f64, b: f64) -> f64 {
    (b - a) * (speed(seg, a) + 4.0 * speed(seg, (a + b) * 0.5) + speed(seg, b)) / 6.0
}

fn integrate(seg: PathSeg, a: f64, b: f64, whole: f64, error: f64, depth: u8) -> f64 {
    let middle = (a + b) * 0.5;
    let (left, right) = (simpson(seg, a, middle), simpson(seg, middle, b));
    let delta = left + right - whole;
    if depth == 0 || delta.abs() <= 15.0 * error {
        return left + right + delta / 15.0;
    }
    integrate(seg, a, middle, left, error * 0.5, depth - 1)
        + integrate(seg, middle, b, right, error * 0.5, depth - 1)
}

fn length(seg: PathSeg, end: f64) -> f64 {
    if let PathSeg::Line(s) = seg {
        return (s.p1 - s.p0).hypot() * end;
    }
    if end <= 0.0 {
        return 0.0;
    }
    // Four initial panels keep the integral honest near a cusp or a tight
    // turn whose endpoint speeds alone would miss it.
    (0..4)
        .map(|i| {
            let (a, b) = (end * i as f64 / 4.0, end * (i + 1) as f64 / 4.0);
            integrate(seg, a, b, simpson(seg, a, b), 2.5e-9, 16)
        })
        .sum()
}

fn parameter(seg: PathSeg, distance: f64, total: f64) -> f64 {
    if distance <= 0.0 {
        return 0.0;
    }
    if distance >= total {
        return 1.0;
    }
    if matches!(seg, PathSeg::Line(_)) {
        return distance / total;
    }
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let middle = (lo + hi) * 0.5;
        if length(seg, middle) < distance {
            lo = middle;
        } else {
            hi = middle;
        }
    }
    (lo + hi) * 0.5
}

fn point(seg: PathSeg, t: f64) -> Point {
    match seg {
        PathSeg::Line(s) => s.p0 + (s.p1 - s.p0) * t,
        PathSeg::Quad(s) => {
            let u = 1.0 - t;
            Point::new(
                u * u * s.p0.x + 2.0 * u * t * s.p1.x + t * t * s.p2.x,
                u * u * s.p0.y + 2.0 * u * t * s.p1.y + t * t * s.p2.y,
            )
        }
        PathSeg::Cubic(s) => {
            let u = 1.0 - t;
            Point::new(
                u.powi(3) * s.p0.x
                    + 3.0 * u * u * t * s.p1.x
                    + 3.0 * u * t * t * s.p2.x
                    + t.powi(3) * s.p3.x,
                u.powi(3) * s.p0.y
                    + 3.0 * u * u * t * s.p1.y
                    + 3.0 * u * t * t * s.p2.y
                    + t.powi(3) * s.p3.y,
            )
        }
    }
}

fn tangent(seg: PathSeg, t: f64) -> Vec2 {
    let mut d = match seg {
        PathSeg::Line(s) => s.p1 - s.p0,
        PathSeg::Quad(s) => ((s.p1 - s.p0) * (1.0 - t) + (s.p2 - s.p1) * t) * 2.0,
        PathSeg::Cubic(s) => {
            ((s.p1 - s.p0) * (1.0 - t) * (1.0 - t)
                + (s.p2 - s.p1) * 2.0 * (1.0 - t) * t
                + (s.p3 - s.p2) * t * t)
                * 3.0
        }
    };
    if d.hypot() <= 1e-14 {
        d = seg.end() - seg.start();
    }
    d
}

fn split(seg: PathSeg, t: f64) -> (PathSeg, PathSeg) {
    let lerp = |a: Point, b: Point| a + (b - a) * t;
    match seg {
        PathSeg::Line(s) => {
            let m = lerp(s.p0, s.p1);
            (
                PathSeg::Line(Line::new(s.p0, m)),
                PathSeg::Line(Line::new(m, s.p1)),
            )
        }
        PathSeg::Quad(s) => {
            let (a, b) = (lerp(s.p0, s.p1), lerp(s.p1, s.p2));
            let m = lerp(a, b);
            (
                PathSeg::Quad(QuadBez::new(s.p0, a, m)),
                PathSeg::Quad(QuadBez::new(m, b, s.p2)),
            )
        }
        PathSeg::Cubic(s) => {
            let (a, b, c) = (lerp(s.p0, s.p1), lerp(s.p1, s.p2), lerp(s.p2, s.p3));
            let (d, e) = (lerp(a, b), lerp(b, c));
            let m = lerp(d, e);
            (
                PathSeg::Cubic(CubicBez::new(s.p0, a, d, m)),
                PathSeg::Cubic(CubicBez::new(m, e, c, s.p3)),
            )
        }
    }
}

fn portion(seg: PathSeg, start: f64, end: f64) -> PathSeg {
    if start == 0.0 && end == 1.0 {
        return seg;
    }
    let (left, _) = split(seg, end);
    if start == 0.0 {
        left
    } else {
        split(left, start / end).1
    }
}

struct ArcSegment {
    seg: PathSeg,
    from: f64,
    to: f64,
}

fn at(segments: &[ArcSegment], distance: f64) -> (Point, Vec2) {
    let chosen = segments
        .iter()
        .find(|s| distance <= s.to)
        .unwrap_or_else(|| segments.last().unwrap());
    let t = parameter(
        chosen.seg,
        (distance - chosen.from).clamp(0.0, chosen.to - chosen.from),
        chosen.to - chosen.from,
    );
    (point(chosen.seg, t), tangent(chosen.seg, t))
}

fn emit(segments: &[ArcSegment], from: f64, to: f64, path: &mut BezPath, move_to: bool) {
    if move_to {
        path.move_to(at(segments, from).0);
    }
    for s in segments {
        let (a, b) = (from.max(s.from), to.min(s.to));
        if b <= a {
            continue;
        }
        let (ta, tb) = (
            parameter(s.seg, a - s.from, s.to - s.from),
            parameter(s.seg, b - s.from, s.to - s.from),
        );
        path.push(portion(s.seg, ta, tb).as_path_el());
    }
}

fn push_span(spans: &mut Vec<(f64, f64)>, from: f64, to: f64) {
    if let Some(last) = spans.last_mut()
        && (last.1 - from).abs() <= 1e-9
    {
        last.1 = to;
    } else {
        spans.push((from, to));
    }
}

/// Partition each subpath by its own dash phase (SVG/Canvas rule). The path
/// and zero-length painted marks feed the independently constructed stroke
/// region; closed dashes crossing the seam stay joined, without seam caps.
pub(super) fn partition(path: &BezPath, offset: f64, pattern: &[f64]) -> Dashes {
    let mut normalized = pattern.to_vec();
    if normalized.len() % 2 == 1 {
        normalized.extend_from_slice(pattern);
    }
    let period: f64 = normalized.iter().sum();
    let mut out = Dashes {
        path: BezPath::new(),
        dots: Vec::new(),
    };
    for subpath in subpaths(path) {
        let mut segments = Vec::new();
        let mut end = 0.0;
        for seg in subpath.segments {
            let len = length(seg, 1.0);
            if len > 1e-12 {
                segments.push(ArcSegment {
                    seg,
                    from: end,
                    to: end + len,
                });
                end += len;
            }
        }
        if segments.is_empty() {
            continue;
        }
        let mut phase = offset.rem_euclid(period);
        let mut index = 0;
        while phase > normalized[index] {
            phase -= normalized[index];
            index = (index + 1) % normalized.len();
        }
        let mut position = 0.0;
        let mut spans = Vec::new();
        while position < end {
            let remaining = normalized[index] - phase;
            if remaining <= 1e-12 {
                if normalized[index] == 0.0 && index % 2 == 0 {
                    out.dots.push(at(&segments, position));
                }
            } else {
                let next = (position + remaining).min(end);
                if index % 2 == 0 {
                    push_span(&mut spans, position, next);
                }
                position = next;
            }
            index = (index + 1) % normalized.len();
            phase = 0.0;
        }
        // A zero painted interval exactly at the path end still contributes
        // its cap-only mark. Positive intervals beyond the end do not.
        if normalized[index] == 0.0 && index % 2 == 0 {
            out.dots.push(at(&segments, end));
        }
        if spans.is_empty() {
            continue;
        }
        if subpath.closed && spans.len() == 1 && spans[0].0 == 0.0 && spans[0].1 == end {
            emit(&segments, 0.0, end, &mut out.path, true);
            out.path.close_path();
        } else if subpath.closed
            && spans.len() > 1
            && spans[0].0 == 0.0
            && spans.last().unwrap().1 == end
        {
            let last = spans.pop().unwrap();
            let first = spans.remove(0);
            emit(&segments, last.0, end, &mut out.path, true);
            emit(&segments, 0.0, first.1, &mut out.path, false);
            for (a, b) in spans {
                emit(&segments, a, b, &mut out.path, true);
            }
        } else {
            for (a, b) in spans {
                emit(&segments, a, b, &mut out.path, true);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straight_intervals_odd_cycles_and_signed_offsets() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((45.0, 0.0));
        let a = partition(&line, -5.0, &[10.0, 5.0, 5.0]);
        let b = partition(&line, 35.0, &[10.0, 5.0, 5.0, 10.0, 5.0, 5.0]);
        assert_eq!(a.path, b.path);
        assert_eq!(a.dots.len(), 0);
        let spans: Vec<_> = a
            .path
            .segments()
            .map(|s| (s.start().x, s.end().x))
            .collect();
        assert_eq!(spans, [(5.0, 15.0), (20.0, 25.0), (35.0, 40.0)]);
    }

    #[test]
    fn zero_intervals_place_caps_and_zero_gaps_do_not_split() {
        let mut line = BezPath::new();
        line.move_to((0.0, 0.0));
        line.line_to((20.0, 0.0));
        let dots = partition(&line, 0.0, &[0.0, 10.0]);
        assert!(dots.path.elements().is_empty());
        assert_eq!(
            dots.dots.iter().map(|d| d.0.x).collect::<Vec<_>>(),
            [0.0, 10.0, 20.0]
        );
        let uncut = partition(&line, 0.0, &[10.0, 0.0]);
        assert_eq!(
            uncut.path.elements(),
            &[
                PathEl::MoveTo((0.0, 0.0).into()),
                PathEl::LineTo((20.0, 0.0).into())
            ]
        );
    }

    #[test]
    fn a_closed_dash_crossing_the_seam_has_no_caps_there() {
        let mut square = BezPath::new();
        square.move_to((0.0, 0.0));
        square.line_to((4.0, 0.0));
        square.line_to((4.0, 4.0));
        square.line_to((0.0, 4.0));
        square.close_path();
        let pieces = partition(&square, 0.0, &[5.0, 1.0]);
        assert_eq!(
            pieces
                .path
                .elements()
                .iter()
                .filter(|e| matches!(e, PathEl::MoveTo(_)))
                .count(),
            2
        );
        assert_eq!(pieces.path.segments().count(), 5);
        assert!(
            !pieces
                .path
                .elements()
                .iter()
                .any(|e| matches!(e, PathEl::ClosePath))
        );
    }

    #[test]
    fn curved_cut_is_by_arc_length_not_curve_parameter() {
        let mut curve = BezPath::new();
        curve.move_to((0.0, 0.0));
        curve.curve_to((0.0, 140.0), (30.0, 100.0), (100.0, 100.0));
        let seg = curve.segments().next().unwrap();
        let half = length(seg, 1.0) * 0.5;
        let pieces = partition(&curve, 0.0, &[half, half]);
        let end = pieces.path.segments().last().unwrap().end();
        // Independently tabulated by fine trapezoidal integration of the
        // cubic speed and linear interpolation of the half-length crossing.
        assert!((end.x - 12.154_753).abs() < 0.002, "{end:?}");
        assert!((end.y - 91.521_377).abs() < 0.002, "{end:?}");
        assert!((end - point(seg, 0.5)).hypot() > 2.0);
    }
}
