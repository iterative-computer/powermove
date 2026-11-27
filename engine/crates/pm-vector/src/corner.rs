//! Rounded-rectangle corners: per-corner radii and Figma-style smoothing.
//!
//! A port of `legacy/gl/corner-geometry.ts`. Content keeps `radius` as the
//! uniform value; `independentCorners` switches to `radiusTL`/`TR`/`BR`/`BL`,
//! and `smoothing` is a percentage (0 is a circular arc, 100 fully smoothed).
//! [`rounded_rect_svg`] produces the same SVG path data as `roundedRectPath`
//! byte for byte, and [`rounded_rect_path`] the geometry Canvas2D draws from it.

use kurbo::{BezPath, Point};

use crate::canvas::CanvasPath;
use crate::js;

/// Radii in the order top-left, top-right, bottom-right, bottom-left.
pub type CornerRadii = [f64; 4];

/// The iOS app-icon smoothing preset (percent).
pub const IOS_SMOOTHING: f64 = 60.0;

/// The corner-related fields of shape content, as numbers. Missing fields are
/// NaN (what `Number(undefined)` gives); a missing per-corner radius is `None`
/// (`null` or `undefined`, which falls back to the uniform radius).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CornerContent {
    pub w: f64,
    pub h: f64,
    pub radius: f64,
    pub independent_corners: bool,
    /// `radiusTL`, `radiusTR`, `radiusBR`, `radiusBL`.
    pub corner_radii: [Option<f64>; 4],
    pub smoothing: f64,
}

impl Default for CornerContent {
    fn default() -> Self {
        Self {
            w: f64::NAN,
            h: f64::NAN,
            radius: f64::NAN,
            independent_corners: false,
            corner_radii: [None; 4],
            smoothing: f64::NAN,
        }
    }
}

/// `number()` in corner-geometry.ts: finite values clamped at 0, else 0.
fn non_negative(value: f64) -> f64 {
    if value.is_finite() {
        js::max(0.0, value)
    } else {
        0.0
    }
}

/// `cornerRadii(d)`.
pub fn corner_radii(d: &CornerContent) -> CornerRadii {
    let uniform = non_negative(d.radius);
    if !d.independent_corners {
        return [uniform; 4];
    }
    d.corner_radii.map(|r| r.map_or(uniform, non_negative))
}

/// `cornerSmoothing(d)`: smoothing as a 0..1 fraction.
pub fn corner_smoothing(d: &CornerContent) -> f64 {
    let percent = d.smoothing;
    if percent.is_finite() {
        js::min(1.0, js::max(0.0, percent / 100.0))
    } else {
        0.0
    }
}

/// `isUniformCircular(d)`: no smoothing and four equal radii.
pub fn is_uniform_circular(d: &CornerContent) -> bool {
    let [a, b, c, e] = corner_radii(d);
    corner_smoothing(d) == 0.0 && a == b && b == c && c == e
}

/// Each radius limited to half the shorter side, like Figma.
fn clamped_radii(d: &CornerContent) -> CornerRadii {
    let limit = js::max(0.0, js::min(js::or0(d.w), js::or0(d.h)) / 2.0);
    corner_radii(d).map(|r| js::min(r, limit))
}

/// `cornerInset(d)`: distance from the bounding box inside which corner
/// geometry can differ from a flat fill.
pub fn corner_inset(d: &CornerContent) -> f64 {
    let smoothing = corner_smoothing(d);
    let radii = clamped_radii(d);
    let largest = radii.iter().copied().fold(f64::NEG_INFINITY, js::max);
    let reach = largest * (1.0 + smoothing);
    js::max(
        0.0,
        js::min(js::min(reach, js::or0(d.w) / 2.0), js::or0(d.h) / 2.0),
    )
}

#[derive(Debug, Clone, Copy, Default)]
struct CornerParams {
    r: f64,
    a: f64,
    b: f64,
    c: f64,
    d: f64,
    p: f64,
    arc: f64,
}

fn corner_params(r: f64, smoothing: f64, budget: f64) -> CornerParams {
    if r <= 0.0 {
        return CornerParams::default();
    }
    let s = js::min(smoothing, js::max(0.0, budget / r - 1.0));
    let p = js::min((1.0 + s) * r, budget);
    let arc_measure = 90.0 * (1.0 - s);
    let arc = js::sin(js::radians(arc_measure / 2.0)) * r * core::f64::consts::SQRT_2;
    let alpha = (90.0 - arc_measure) / 2.0;
    let p3p4 = r * js::tan(js::radians(alpha / 2.0));
    let beta = 45.0 * s;
    let c = p3p4 * js::cos(js::radians(beta));
    let d = c * js::tan(js::radians(beta));
    let b = (p - arc - c - d) / 3.0;
    CornerParams {
        r,
        a: 2.0 * b,
        b,
        c,
        d,
        p,
        arc,
    }
}

/// Room each corner may use along its edges, shared proportionally with its neighbours.
fn budgets(radii: CornerRadii, w: f64, h: f64) -> [f64; 4] {
    let [tl, tr, br, bl] = radii;
    let share = |r: f64, neighbour: f64, side: f64| {
        if r + neighbour == 0.0 {
            side
        } else {
            r / (r + neighbour) * side
        }
    };
    [
        js::min(share(tl, tr, w), share(tl, bl, h)),
        js::min(share(tr, tl, w), share(tr, br, h)),
        js::min(share(br, bl, w), share(br, tr, h)),
        js::min(share(bl, br, w), share(bl, tl, h)),
    ]
}

/// `f()`: four decimals, `Math.round` semantics.
fn f(value: f64) -> f64 {
    js::round(value * 1e4) / 1e4
}

/// One command of the rounded-rectangle path data, with its numbers already
/// rounded by `f()`. Lower-case commands are relative.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Command {
    MoveAbs(f64, f64),
    LineAbs(f64, f64),
    LineRel(f64, f64),
    CurveRel([f64; 6]),
    /// `a r r 0 0 1 dx dy`.
    ArcRel {
        r: f64,
        dx: f64,
        dy: f64,
    },
    Close,
}

fn commands(d: &CornerContent) -> Vec<Command> {
    use Command::*;
    let w = js::or0(d.w);
    let h = js::or0(d.h);
    let radii = clamped_radii(d);
    let smoothing = corner_smoothing(d);
    let budget = budgets(radii, w, h);
    let [tl, tr, br, bl] = [0, 1, 2, 3].map(|i| corner_params(radii[i], smoothing, budget[i]));
    let curve = |v: [f64; 6]| CurveRel(v.map(f));
    let arc = |k: &CornerParams, x: f64, y: f64| ArcRel {
        r: f(k.r),
        dx: f(x),
        dy: f(y),
    };
    let mut out = vec![MoveAbs(f(w - tr.p), 0.0)];
    if js::truthy(tr.r) {
        out.push(curve([
            tr.a,
            0.0,
            tr.a + tr.b,
            0.0,
            tr.a + tr.b + tr.c,
            tr.d,
        ]));
        out.push(arc(&tr, tr.arc, tr.arc));
        out.push(curve([
            tr.d,
            tr.c,
            tr.d,
            tr.b + tr.c,
            tr.d,
            tr.a + tr.b + tr.c,
        ]));
    } else {
        out.push(LineRel(f(tr.p), 0.0));
    }
    out.push(LineAbs(f(w), f(h - br.p)));
    if js::truthy(br.r) {
        out.push(curve([
            0.0,
            br.a,
            0.0,
            br.a + br.b,
            -br.d,
            br.a + br.b + br.c,
        ]));
        out.push(arc(&br, -br.arc, br.arc));
        out.push(curve([
            -br.c,
            br.d,
            -(br.b + br.c),
            br.d,
            -(br.a + br.b + br.c),
            br.d,
        ]));
    } else {
        out.push(LineRel(0.0, f(br.p)));
    }
    out.push(LineAbs(f(bl.p), f(h)));
    if js::truthy(bl.r) {
        out.push(curve([
            -bl.a,
            0.0,
            -(bl.a + bl.b),
            0.0,
            -(bl.a + bl.b + bl.c),
            -bl.d,
        ]));
        out.push(arc(&bl, -bl.arc, -bl.arc));
        out.push(curve([
            -bl.d,
            -bl.c,
            -bl.d,
            -(bl.b + bl.c),
            -bl.d,
            -(bl.a + bl.b + bl.c),
        ]));
    } else {
        out.push(LineRel(f(-bl.p), 0.0));
    }
    out.push(LineAbs(0.0, f(tl.p)));
    if js::truthy(tl.r) {
        out.push(curve([
            0.0,
            -tl.a,
            0.0,
            -(tl.a + tl.b),
            tl.d,
            -(tl.a + tl.b + tl.c),
        ]));
        out.push(arc(&tl, tl.arc, -tl.arc));
        out.push(curve([
            tl.c,
            -tl.d,
            tl.b + tl.c,
            -tl.d,
            tl.a + tl.b + tl.c,
            -tl.d,
        ]));
    } else {
        out.push(LineRel(0.0, f(-tl.p)));
    }
    out.push(Close);
    out
}

/// `roundedRectPath(d)`: SVG path data for the outline with its origin at the
/// top-left corner, identical to the TypeScript output.
pub fn rounded_rect_svg(d: &CornerContent) -> String {
    let n = js::number_to_string;
    commands(d)
        .iter()
        .map(|c| match *c {
            Command::MoveAbs(x, y) => format!("M {} {}", n(x), n(y)),
            Command::LineAbs(x, y) => format!("L {} {}", n(x), n(y)),
            Command::LineRel(x, y) => format!("l {} {}", n(x), n(y)),
            Command::CurveRel(v) => format!("c {}", v.map(n).join(" ")),
            Command::ArcRel { r, dx, dy } => {
                format!("a {} {} 0 0 1 {} {}", n(r), n(r), n(dx), n(dy))
            }
            Command::Close => "Z".to_owned(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The geometry `new Path2D(roundedRectPath(d))` describes, as a closed path
/// with its origin at the top-left corner.
pub fn rounded_rect_path(d: &CornerContent) -> BezPath {
    let mut path = CanvasPath::new();
    let mut at = Point::ZERO;
    for command in commands(d) {
        match command {
            Command::MoveAbs(x, y) => {
                at = Point::new(x, y);
                path.move_to(x, y);
            }
            Command::LineAbs(x, y) => {
                at = Point::new(x, y);
                path.line_to(x, y);
            }
            Command::LineRel(dx, dy) => {
                at = Point::new(at.x + dx, at.y + dy);
                path.line_to(at.x, at.y);
            }
            Command::CurveRel([x1, y1, x2, y2, x, y]) => {
                let o = at;
                at = Point::new(o.x + x, o.y + y);
                path.bezier_curve_to(o.x + x1, o.y + y1, o.x + x2, o.y + y2, at.x, at.y);
            }
            Command::ArcRel { r, dx, dy } => {
                at = Point::new(at.x + dx, at.y + dy);
                path.svg_arc(r, r, false, true, at);
            }
            Command::Close => path.close_path(),
        }
    }
    path.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Shape;

    fn content(w: f64, h: f64, radius: f64, smoothing: f64) -> CornerContent {
        CornerContent {
            w,
            h,
            radius,
            smoothing,
            ..CornerContent::default()
        }
    }

    #[test]
    fn plain_rounded_rect_matches_the_typescript_string() {
        // From corner-geometry.ts under bun: roundedRectPath({w:100,h:50,radius:10,smoothing:0}).
        assert_eq!(
            rounded_rect_svg(&content(100.0, 50.0, 10.0, 0.0)),
            "M 90 0 c 0 0 0 0 0 0 a 10 10 0 0 1 10 10 c 0 0 0 0 0 0 L 100 40 c 0 0 0 0 0 0 a 10 10 0 0 1 -10 10 c 0 0 0 0 0 0 L 10 50 c 0 0 0 0 0 0 a 10 10 0 0 1 -10 -10 c 0 0 0 0 0 0 L 0 10 c 0 0 0 0 0 0 a 10 10 0 0 1 10 -10 c 0 0 0 0 0 0 Z"
        );
    }

    #[test]
    fn rounded_rect_area_matches_the_analytic_area() {
        let path = rounded_rect_path(&content(100.0, 50.0, 10.0, 0.0));
        let expected = 100.0 * 50.0 - (4.0 - core::f64::consts::PI) * 100.0;
        assert!((path.area() - expected).abs() < 1e-3, "{}", path.area());
    }

    #[test]
    fn smoothing_reaches_further_along_the_edges() {
        let d = content(200.0, 200.0, 40.0, IOS_SMOOTHING);
        assert_eq!(corner_smoothing(&d), 0.6);
        assert!(!is_uniform_circular(&d));
        assert!((corner_inset(&d) - 64.0).abs() < 1e-12);
        let path = rounded_rect_path(&d);
        assert!(path.area() < 200.0 * 200.0);
        let b = path.bounding_box();
        assert!(
            b.x0.abs() < 1e-9
                && b.y0.abs() < 1e-9
                && (b.x1 - 200.0).abs() < 1e-9
                && (b.y1 - 200.0).abs() < 1e-9
        );
        // The top edge starts its curve at (1 + s) r from the corner.
        assert_eq!(
            path.elements()[0],
            kurbo::PathEl::MoveTo(Point::new(136.0, 0.0))
        );
    }

    #[test]
    fn independent_corners_fall_back_to_the_uniform_radius() {
        let d = CornerContent {
            radius: 8.0,
            independent_corners: true,
            corner_radii: [Some(2.0), None, Some(-4.0), Some(f64::NAN)],
            ..content(10.0, 10.0, 8.0, 0.0)
        };
        assert_eq!(corner_radii(&d), [2.0, 8.0, 0.0, 0.0]);
    }
}
