//! A path builder with Canvas2D path semantics, producing a kurbo [`BezPath`].
//!
//! The legacy geometry is defined by the Canvas2D calls it makes (`moveTo`,
//! `arcTo`, `ellipse`, `rect`, SVG path data through `Path2D`). This builder
//! reproduces the HTML specification's behavior for those calls, including
//! ignoring calls with non-finite arguments and the implicit `moveTo` of a
//! `lineTo` on an empty path, so the ports read like the TypeScript they
//! replace.
//!
//! Arcs become cubic Béziers with at most [`MAX_ARC_SEGMENT`] radians per
//! segment (radial error below 7e-8 of the radius), computed with `libm` so
//! every target produces the same bits.

use core::f64::consts::{PI, TAU};

use kurbo::{BezPath, PathEl, Point, Vec2};

/// Largest arc sweep, in radians, approximated by one cubic segment.
pub const MAX_ARC_SEGMENT: f64 = PI / 8.0;

/// Builds a [`BezPath`] with Canvas2D path semantics.
#[derive(Debug, Clone, Default)]
pub struct CanvasPath {
    path: BezPath,
    /// Current point, if the path has a subpath.
    current: Option<Point>,
    /// First point of the current subpath.
    start: Point,
}

fn finite(values: &[f64]) -> bool {
    values.iter().all(|v| v.is_finite())
}

impl CanvasPath {
    pub fn new() -> Self {
        Self::default()
    }

    /// The finished path.
    pub fn finish(self) -> BezPath {
        self.path
    }

    pub fn current_point(&self) -> Option<Point> {
        self.current
    }

    /// `ctx.moveTo(x, y)`.
    pub fn move_to(&mut self, x: f64, y: f64) {
        if !finite(&[x, y]) {
            return;
        }
        let p = Point::new(x, y);
        // A move after a move replaces the empty subpath.
        if let Some(PathEl::MoveTo(_)) = self.path.elements().last() {
            self.path.truncate(self.path.elements().len() - 1);
        }
        self.path.move_to(p);
        self.current = Some(p);
        self.start = p;
    }

    fn ensure_subpath(&mut self, x: f64, y: f64) {
        if self.current.is_none() {
            self.move_to(x, y);
        }
    }

    /// `ctx.lineTo(x, y)`.
    pub fn line_to(&mut self, x: f64, y: f64) {
        if !finite(&[x, y]) {
            return;
        }
        if self.current.is_none() {
            self.move_to(x, y);
            return;
        }
        let p = Point::new(x, y);
        self.path.line_to(p);
        self.current = Some(p);
    }

    /// `ctx.bezierCurveTo(...)`.
    pub fn bezier_curve_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, x: f64, y: f64) {
        if !finite(&[x1, y1, x2, y2, x, y]) {
            return;
        }
        self.ensure_subpath(x1, y1);
        let p = Point::new(x, y);
        self.path
            .curve_to(Point::new(x1, y1), Point::new(x2, y2), p);
        self.current = Some(p);
    }

    /// `ctx.closePath()`.
    pub fn close_path(&mut self) {
        if self.current.is_none() {
            return;
        }
        if !matches!(self.path.elements().last(), Some(PathEl::ClosePath)) {
            self.path.close_path();
        }
        // The next subpath starts at the closed subpath's first point.
        self.current = Some(self.start);
    }

    /// `ctx.rect(x, y, w, h)`: a closed subpath, then a new subpath at (x, y).
    pub fn rect(&mut self, x: f64, y: f64, w: f64, h: f64) {
        if !finite(&[x, y, w, h]) {
            return;
        }
        self.move_to(x, y);
        self.line_to(x + w, y);
        self.line_to(x + w, y + h);
        self.line_to(x, y + h);
        self.close_path();
    }

    /// `ctx.arcTo(x1, y1, x2, y2, radius)`. A negative radius throws an
    /// `IndexSizeError` in the browser; here it returns `Err`.
    pub fn arc_to(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, radius: f64) -> Result<(), NegativeRadius> {
        if !finite(&[x1, y1, x2, y2, radius]) {
            return Ok(());
        }
        if radius < 0.0 {
            return Err(NegativeRadius);
        }
        self.ensure_subpath(x1, y1);
        let p0 = self.current.expect("subpath exists");
        let p1 = Point::new(x1, y1);
        let p2 = Point::new(x2, y2);
        // Degenerate cases from the specification: a straight line to p1.
        let d0 = p0 - p1;
        let d2 = p2 - p1;
        let cross = d0.x * d2.y - d0.y * d2.x;
        if p0 == p1 || p1 == p2 || radius == 0.0 || cross == 0.0 {
            self.line_to(x1, y1);
            return Ok(());
        }
        let l0 = libm::hypot(d0.x, d0.y);
        let l2 = libm::hypot(d2.x, d2.y);
        let u0 = d0 / l0;
        let u2 = d2 / l2;
        // With phi the angle at p1 between the legs, the tangent points are
        // r / tan(phi / 2) = r (1 + cos phi) / sin phi from p1, and the center
        // is sqrt(r^2 + that^2) from p1 along the bisector.
        let cos_phi = u0.dot(u2);
        let sin_phi = (u0.x * u2.y - u0.y * u2.x).abs();
        let tangent = radius * (1.0 + cos_phi) / sin_phi;
        let t0 = p1 + u0 * tangent;
        let t2 = p1 + u2 * tangent;
        let bisector = (u0 + u2).normalize();
        let center = p1 + bisector * libm::sqrt(radius * radius + tangent * tangent);
        let a0 = libm::atan2(t0.y - center.y, t0.x - center.x);
        let a2 = libm::atan2(t2.y - center.y, t2.x - center.x);
        // The arc between the tangent points always spans less than half a
        // turn (pi minus the corner angle), so it is the short way round.
        let mut sweep = a2 - a0;
        if sweep > PI {
            sweep -= TAU;
        } else if sweep < -PI {
            sweep += TAU;
        }
        self.line_to(t0.x, t0.y);
        append_arc(&mut self.path, center, Vec2::new(radius, radius), a0, sweep, t2);
        self.current = Some(t2);
        Ok(())
    }

    /// `ctx.ellipse(x, y, rx, ry, rotation, start, end)` (clockwise). Negative
    /// radii throw in the browser; here they return `Err`.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(
        &mut self,
        x: f64,
        y: f64,
        rx: f64,
        ry: f64,
        rotation: f64,
        start: f64,
        end: f64,
    ) -> Result<(), NegativeRadius> {
        if !finite(&[x, y, rx, ry, rotation, start, end]) {
            return Ok(());
        }
        if rx < 0.0 || ry < 0.0 {
            return Err(NegativeRadius);
        }
        // Canonicalize the angles like Blink: a clockwise sweep of at least a
        // full turn draws the whole ellipse once from `start`.
        let sweep = if end - start >= TAU {
            TAU
        } else {
            let mut s = libm::fmod(end - start, TAU);
            if s < 0.0 {
                s += TAU;
            }
            s
        };
        let (rs, rc) = (libm::sin(rotation), libm::cos(rotation));
        let at = |angle: f64| {
            let (s, c) = (libm::sin(angle), libm::cos(angle));
            let (px, py) = (rx * c, ry * s);
            Point::new(x + px * rc - py * rs, y + px * rs + py * rc)
        };
        let first = at(start);
        if self.current.is_some() {
            self.line_to(first.x, first.y);
        } else {
            self.move_to(first.x, first.y);
        }
        let last = if sweep == TAU { first } else { at(start + sweep) };
        append_rotated_arc(&mut self.path, Point::new(x, y), Vec2::new(rx, ry), rotation, start, sweep, last);
        self.current = Some(last);
        Ok(())
    }

    /// An SVG elliptical arc command with no x-axis rotation (`a`/`A` as used
    /// by the legacy path data), from the current point to `to`.
    pub fn svg_arc(&mut self, rx: f64, ry: f64, large_arc: bool, sweep_flag: bool, to: Point) {
        let Some(from) = self.current else {
            return;
        };
        if from == to {
            return;
        }
        let (mut rx, mut ry) = (rx.abs(), ry.abs());
        if rx == 0.0 || ry == 0.0 {
            self.line_to(to.x, to.y);
            return;
        }
        // SVG 1.1 implementation notes F.6.5 with phi = 0.
        let x1p = (from.x - to.x) / 2.0;
        let y1p = (from.y - to.y) / 2.0;
        let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
        if lambda > 1.0 {
            let s = libm::sqrt(lambda);
            rx *= s;
            ry *= s;
        }
        let num = rx * rx * ry * ry - rx * rx * y1p * y1p - ry * ry * x1p * x1p;
        let den = rx * rx * y1p * y1p + ry * ry * x1p * x1p;
        let mut coef = if den == 0.0 { 0.0 } else { libm::sqrt((num / den).max(0.0)) };
        if large_arc == sweep_flag {
            coef = -coef;
        }
        let cxp = coef * rx * y1p / ry;
        let cyp = -coef * ry * x1p / rx;
        let center = Point::new(cxp + (from.x + to.x) / 2.0, cyp + (from.y + to.y) / 2.0);
        let ux = (x1p - cxp) / rx;
        let uy = (y1p - cyp) / ry;
        let vx = (-x1p - cxp) / rx;
        let vy = (-y1p - cyp) / ry;
        let start = libm::atan2(uy, ux);
        let mut sweep = libm::atan2(ux * vy - uy * vx, ux * vx + uy * vy);
        if !sweep_flag && sweep > 0.0 {
            sweep -= TAU;
        } else if sweep_flag && sweep < 0.0 {
            sweep += TAU;
        }
        append_arc(&mut self.path, center, Vec2::new(rx, ry), start, sweep, to);
        self.current = Some(to);
    }
}

/// `arcTo` or `ellipse` with a negative radius (`IndexSizeError` in Canvas2D).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegativeRadius;

impl core::fmt::Display for NegativeRadius {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("negative radius (Canvas2D IndexSizeError)")
    }
}

impl std::error::Error for NegativeRadius {}

/// Appends an axis-aligned elliptical arc as cubics, ending exactly at `end`.
fn append_arc(path: &mut BezPath, center: Point, radii: Vec2, start: f64, sweep: f64, end: Point) {
    append_rotated_arc(path, center, radii, 0.0, start, sweep, end);
}

/// Appends an elliptical arc (parametric angles, rotated by `rotation`) as
/// cubic segments of at most [`MAX_ARC_SEGMENT`], ending exactly at `end`.
fn append_rotated_arc(
    path: &mut BezPath,
    center: Point,
    radii: Vec2,
    rotation: f64,
    start: f64,
    sweep: f64,
    end: Point,
) {
    if sweep == 0.0 {
        return;
    }
    let n = libm::ceil(sweep.abs() / MAX_ARC_SEGMENT).max(1.0) as usize;
    let step = sweep / n as f64;
    // Control-point distance for a circular arc of `step` radians.
    let k = 4.0 / 3.0 * libm::tan(step / 4.0);
    let (rs, rc) = (libm::sin(rotation), libm::cos(rotation));
    let map = |px: f64, py: f64| {
        let (px, py) = (px * radii.x, py * radii.y);
        Point::new(center.x + px * rc - py * rs, center.y + px * rs + py * rc)
    };
    let (mut s0, mut c0) = (libm::sin(start), libm::cos(start));
    for i in 0..n {
        let a1 = if i + 1 == n { start + sweep } else { start + step * (i + 1) as f64 };
        let (s1, c1) = (libm::sin(a1), libm::cos(a1));
        let cp1 = map(c0 - k * s0, s0 + k * c0);
        let cp2 = map(c1 + k * s1, s1 - k * c1);
        let p = if i + 1 == n { end } else { map(c1, s1) };
        path.curve_to(cp1, cp2, p);
        s0 = s1;
        c0 = c1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{ParamCurve, Shape};

    #[test]
    fn line_without_subpath_moves() {
        let mut p = CanvasPath::new();
        p.line_to(1.0, 2.0);
        p.line_to(3.0, 4.0);
        assert_eq!(
            p.finish().elements(),
            &[PathEl::MoveTo(Point::new(1.0, 2.0)), PathEl::LineTo(Point::new(3.0, 4.0))]
        );
    }

    #[test]
    fn non_finite_calls_are_ignored() {
        let mut p = CanvasPath::new();
        p.move_to(f64::NAN, 0.0);
        p.line_to(f64::INFINITY, 0.0);
        assert!(p.finish().elements().is_empty());
    }

    #[test]
    fn full_circle_area() {
        let mut p = CanvasPath::new();
        p.ellipse(10.0, 20.0, 5.0, 5.0, 0.0, 0.0, 7.0).unwrap();
        let path = p.finish();
        assert!((path.area() - PI * 25.0).abs() < 1e-5);
        let b = path.bounding_box();
        assert!((b.x0 - 5.0).abs() < 1e-6 && (b.x1 - 15.0).abs() < 1e-6);
        assert!((b.y0 - 15.0).abs() < 1e-6 && (b.y1 - 25.0).abs() < 1e-6);
    }

    #[test]
    fn arc_to_rounds_a_square_corner() {
        let mut p = CanvasPath::new();
        p.move_to(0.0, 0.0);
        p.arc_to(10.0, 0.0, 10.0, 10.0, 4.0).unwrap();
        let path = p.finish();
        let els = path.elements();
        assert_eq!(els[1], PathEl::LineTo(Point::new(6.0, 0.0)));
        let PathEl::CurveTo(_, _, end) = *els.last().unwrap() else { panic!() };
        assert!((end - Point::new(10.0, 4.0)).hypot() < 1e-12);
        // Every arc sample is on the circle of radius 4 around (6, 4).
        for seg in path.segments().skip(1) {
            for t in [0.25, 0.5, 0.75] {
                let r = (seg.eval(t) - Point::new(6.0, 4.0)).hypot();
                assert!((r - 4.0).abs() < 1e-6, "{r}");
            }
        }
    }

    #[test]
    fn arc_to_negative_radius_errors() {
        let mut p = CanvasPath::new();
        p.move_to(0.0, 0.0);
        assert_eq!(p.arc_to(1.0, 0.0, 1.0, 1.0, -1.0), Err(NegativeRadius));
    }

    #[test]
    fn svg_quarter_arc() {
        let mut p = CanvasPath::new();
        p.move_to(0.0, 0.0);
        p.svg_arc(10.0, 10.0, false, true, Point::new(10.0, 10.0));
        let path = p.finish();
        // Sweep flag 1 turns clockwise on screen, so the center is (0, 10).
        let last = path.segments().last().unwrap().eval(1.0);
        assert!((last - Point::new(10.0, 10.0)).hypot() < 1e-12);
        for seg in path.segments() {
            let r = (seg.eval(0.5) - Point::new(0.0, 10.0)).hypot();
            assert!((r - 10.0).abs() < 1e-6, "{r}");
        }
    }
}
