//! `SkRRect` and the paths Skia builds for rectangles, ovals, and rounded
//! rectangles (`SkPathRawShapes`, `SkPath_*PointIterator`), in `f32` as the
//! pinned Skia computes them. Blink's `<rect>`, `<circle>`, `<ellipse>` and
//! `clip-path` geometry reach Graphite through these.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::skpath::{Verb, Mat, P, SkPath};

/// `SkRRect::Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RRectType {
    Empty,
    Rect,
    Oval,
    Simple,
    NinePatch,
    Complex,
}

/// `SkRRect`: a rectangle `[l, t, r, b]` and the radii of its corners
/// (upper-left, upper-right, lower-right, lower-left).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SkRRect {
    pub rect: [f32; 4],
    pub radii: [[f32; 2]; 4],
    pub ty: RRectType,
}

const NEARLY_ZERO: f32 = 1.0 / 4096.0;

#[inline]
fn nearly_equal(a: f32, b: f32) -> bool {
    (a - b).abs() <= NEARLY_ZERO
}

impl SkRRect {
    pub fn width(&self) -> f32 {
        self.rect[2] - self.rect[0]
    }

    pub fn height(&self) -> f32 {
        self.rect[3] - self.rect[1]
    }

    /// `SkRRect::MakeRect`.
    pub fn make_rect(rect: [f32; 4]) -> Self {
        let rect = sort(rect);
        if rect[2] - rect[0] <= 0.0 || rect[3] - rect[1] <= 0.0 {
            return Self { rect, radii: [[0.0; 2]; 4], ty: RRectType::Empty };
        }
        Self { rect, radii: [[0.0; 2]; 4], ty: RRectType::Rect }
    }

    /// `SkRRect::MakeOval`.
    pub fn make_oval(rect: [f32; 4]) -> Self {
        let rect = sort(rect);
        if rect[2] - rect[0] <= 0.0 || rect[3] - rect[1] <= 0.0 {
            return Self::make_rect(rect);
        }
        let rx = half_width(rect);
        let ry = half_height(rect);
        if rx == 0.0 || ry == 0.0 {
            return Self::make_rect(rect);
        }
        Self { rect, radii: [[rx, ry]; 4], ty: RRectType::Oval }
    }

    /// `SkRRect::setRectXY`.
    pub fn make_rect_xy(rect: [f32; 4], rx: f32, ry: f32) -> Self {
        let rect = sort(rect);
        if rect[2] - rect[0] <= 0.0 || rect[3] - rect[1] <= 0.0 {
            return Self::make_rect(rect);
        }
        if !(rx.is_finite() && ry.is_finite()) {
            return Self::make_rect(rect);
        }
        let (mut rx, mut ry) = (rx, ry);
        if rx <= 0.0 || ry <= 0.0 {
            return Self::make_rect(rect);
        }
        if rect[2] - rect[0] < rx + rx || rect[3] - rect[1] < ry + ry {
            let scale = ((rect[2] - rect[0]) / (rx + rx)).min((rect[3] - rect[1]) / (ry + ry));
            rx *= scale;
            ry *= scale;
        }
        if rx <= 0.0 || ry <= 0.0 {
            return Self::make_rect(rect);
        }
        let mut out = Self { rect, radii: [[rx, ry]; 4], ty: RRectType::Simple };
        if nearly_equal(rx, half_width(rect)) && nearly_equal(ry, half_height(rect)) {
            out.ty = RRectType::Oval;
            out.radii = [[half_width(rect), half_height(rect)]; 4];
        }
        out
    }

    /// `SkRRect::setRectRadii`.
    pub fn make_rect_radii(rect: [f32; 4], radii: [[f32; 2]; 4]) -> Self {
        let rect = sort(rect);
        if rect[2] - rect[0] <= 0.0 || rect[3] - rect[1] <= 0.0 {
            return Self::make_rect(rect);
        }
        if !radii.iter().all(|r| r[0].is_finite() && r[1].is_finite()) {
            return Self::make_rect(rect);
        }
        let mut out = Self { rect, radii, ty: RRectType::Complex };
        for r in &mut out.radii {
            r[0] = r[0].max(0.0);
            r[1] = r[1].max(0.0);
        }
        if out.scale_radii() {
            out.compute_type();
        }
        out
    }

    /// `SkRRect::scaleRadii`: the radii of each axis scaled by the smallest
    /// factor that keeps adjacent corners from overlapping (the SVG and CSS
    /// rule), `clamp_to_zero`d, and `clamp_radius`ed.
    fn scale_radii(&mut self) -> bool {
        let mut scale = 1.0f32;
        let r = self.rect;
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        // Flattened: UL(x 0, y 1), UR(2, 3), LR(4, 5), LL(6, 7).
        let mut f = [
            self.radii[0][0], self.radii[0][1], self.radii[1][0], self.radii[1][1],
            self.radii[2][0], self.radii[2][1], self.radii[3][0], self.radii[3][1],
        ];
        scale = compute_min_scale(f[0], f[2], w, scale);
        scale = compute_min_scale(f[3], f[5], h, scale);
        scale = compute_min_scale(f[4], f[6], w, scale);
        scale = compute_min_scale(f[7], f[1], h, scale);
        for (a, b) in [(0, 2), (3, 5), (4, 6), (7, 1)] {
            if f[a] + f[b] == 0.0 {
                f[a] = 0.0;
                f[b] = 0.0;
            }
        }
        if scale < 1.0 {
            adjust_radii(w, scale, &mut f, 0, 2);
            adjust_radii(h, scale, &mut f, 3, 5);
            adjust_radii(w, scale, &mut f, 4, 6);
            adjust_radii(h, scale, &mut f, 7, 1);
        }
        for v in &mut f {
            *v = clamp_radius(*v);
        }
        self.radii = [[f[0], f[1]], [f[2], f[3]], [f[4], f[5]], [f[6], f[7]]];
        true
    }

    /// `SkRRect::computeType`.
    fn compute_type(&mut self) {
        let r = self.rect;
        if r[2] - r[0] <= 0.0 || r[3] - r[1] <= 0.0 {
            self.ty = RRectType::Empty;
            return;
        }
        let mut all_corners_square = true;
        let mut all_radii_equal = true;
        for i in 0..4 {
            if self.radii[i][0] != 0.0 || self.radii[i][1] != 0.0 {
                all_corners_square = false;
            }
            if self.radii[i][0] != self.radii[0][0] || self.radii[i][1] != self.radii[0][1] {
                all_radii_equal = false;
            }
        }
        if all_corners_square {
            self.ty = RRectType::Rect;
            return;
        }
        if all_radii_equal {
            if nearly_equal(self.radii[0][0], half_width(r)) && nearly_equal(self.radii[0][1], half_height(r)) {
                self.ty = RRectType::Oval;
            } else {
                self.ty = RRectType::Simple;
            }
            return;
        }
        let patches_corners = self.radii[0][0] == self.radii[3][0]
            && self.radii[0][1] == self.radii[1][1]
            && self.radii[1][0] == self.radii[2][0]
            && self.radii[2][1] == self.radii[3][1];
        self.ty = if patches_corners { RRectType::NinePatch } else { RRectType::Complex };
    }

    pub fn is_rect(&self) -> bool {
        matches!(self.ty, RRectType::Rect)
    }

    pub fn is_oval(&self) -> bool {
        matches!(self.ty, RRectType::Oval)
    }

    pub fn is_simple(&self) -> bool {
        matches!(self.ty, RRectType::Simple)
    }

    pub fn is_empty(&self) -> bool {
        matches!(self.ty, RRectType::Empty)
    }

    /// `SkRRect::transform` (the current, non-legacy implementation): the
    /// mapped rect for rects and ovals (an oval's radii are rebuilt from the
    /// new rect, so its extreme points stay on the rect); other round rects
    /// map their contour points and deduce the radii from the mapped conics
    /// (`SkPathPriv::DeduceRRectFromContour`).
    pub fn transform(&self, m: &Mat) -> Option<SkRRect> {
        if *m == Mat::IDENTITY {
            return Some(*self);
        }
        let axis_aligned = (m.kx == 0.0 && m.ky == 0.0) || (m.sx == 0.0 && m.sy == 0.0);
        if !axis_aligned {
            return None;
        }
        let new_rect = sort(map_rect(m, self.rect));
        if !new_rect.iter().all(|v| v.is_finite()) {
            return None;
        }
        match self.ty {
            RRectType::Empty => return Some(Self::make_rect([0.0; 4])),
            RRectType::Rect => return Some(Self::make_rect(new_rect)),
            RRectType::Oval => return Some(Self::make_oval(new_rect)),
            _ => {}
        }
        let path = rrect_path(self, true, 0);
        let pts: Vec<P> = path.pts.iter().map(|p| m.map(*p)).collect();
        let mut radii = [[0.0f32; 2]; 4];
        let mut pi = 0usize;
        for v in &path.verbs {
            match v {
                Verb::Move | Verb::Line => pi += 1,
                Verb::Conic(_) => {
                    let (prev, ctrl, end) = (pts[pi - 1], pts[pi], pts[pi + 1]);
                    let v1_0 = [ctrl[0] - prev[0], ctrl[1] - prev[1]];
                    let v2_1 = [end[0] - ctrl[0], end[1] - ctrl[1]];
                    let dxdy = if v1_0[0] != 0.0 {
                        [v1_0[0].abs(), v2_1[1].abs()]
                    } else if v1_0[1] == 0.0 {
                        [v2_1[0].abs(), v2_1[1].abs()]
                    } else {
                        [v2_1[0].abs(), v1_0[1].abs()]
                    };
                    let corner = if ctrl[0] == new_rect[0] {
                        if ctrl[1] == new_rect[1] { 0 } else { 3 }
                    } else if ctrl[1] == new_rect[1] {
                        1
                    } else {
                        2
                    };
                    radii[corner] = dxdy;
                    pi += 2;
                }
                Verb::Quad | Verb::Cubic => return None,
                Verb::Close => {}
            }
        }
        Some(Self::make_rect_radii(new_rect, radii))
    }


    /// `SkRRectPriv::InnerBounds`: the largest axis-aligned rectangle inside.
    pub fn inner_bounds(&self) -> [f32; 4] {
        if self.is_empty() || self.is_rect() {
            return self.rect;
        }
        let mut inner_bounds = self.rect;
        let (ul, ur, lr, ll) = (self.radii[0], self.radii[1], self.radii[2], self.radii[3]);
        let ins_x = ul[0].max(ll[0]).max(ur[0].max(lr[0]));
        let ins_y = ul[1].max(ur[1]).max(ll[1].max(lr[1]));
        inner_bounds[0] += ins_x;
        inner_bounds[1] += ins_y;
        inner_bounds[2] -= ins_x;
        inner_bounds[3] -= ins_y;
        if inner_bounds[2] - inner_bounds[0] <= 0.0 || inner_bounds[3] - inner_bounds[1] <= 0.0 {
            return [0.0; 4];
        }
        inner_bounds
    }

    /// `SkRRectPriv::IsRelativelyCircular`.
    pub fn is_relatively_circular(rx: f32, ry: f32, tolerance: f32) -> bool {
        (rx - ry).abs() <= tolerance * rx.max(ry)
    }

    /// `SkRRectPriv::AllCornersRelativelyCircular`.
    pub fn all_corners_relatively_circular(&self, tolerance: f32) -> bool {
        self.radii.iter().all(|r| Self::is_relatively_circular(r[0], r[1], tolerance))
    }

    /// The rounded rectangle as `SkPathBuilder::addRRect(rrect, dir, index)`
    /// writes it (`SkPathRawShapes::RRect`, or the oval/rect simplification
    /// `SkPathPriv::SimplifyRRect` applies).
    pub fn to_path(&self, cw: bool, index: u32) -> SkPath {
        if self.is_rect() || self.is_empty() {
            return rect_path(self.rect, cw, (index + 1) / 2);
        }
        if self.is_oval() {
            return oval_path(self.rect, cw, index / 2);
        }
        rrect_path(self, cw, index)
    }
}

#[inline]
fn sort(r: [f32; 4]) -> [f32; 4] {
    [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])]
}

#[inline]
fn half_width(r: [f32; 4]) -> f32 {
    // SkRectPriv::HalfWidth: (r - l) / 2 in float.
    0.5 * (r[2] - r[0])
}

#[inline]
fn half_height(r: [f32; 4]) -> f32 {
    0.5 * (r[3] - r[1])
}

fn compute_min_scale(rad1: f32, rad2: f32, limit: f32, cur_min: f32) -> f32 {
    if (rad1 + rad2) > limit {
        let temp = limit / (rad1 + rad2);
        cur_min.min(temp)
    } else {
        cur_min
    }
}

fn clamp_radius(r: f32) -> f32 {
    if r < 0.0 || r.is_nan() {
        0.0
    } else {
        r
    }
}

/// `SkRRect::AdjustRadii`.
fn adjust_radii(limit: f32, scale: f32, f: &mut [f32; 8], ia: usize, ib: usize) {
    f[ia] *= scale;
    f[ib] *= scale;
    if f[ia] + f[ib] > limit {
        let (imin, imax) = if f[ia] < f[ib] { (ia, ib) } else { (ib, ia) };
        let new_min = f64::from(limit) - f64::from(f[imax]);
        f[imin] = new_min as f32;
        if f64::from(f[imin]) > new_min {
            // The subtraction rounded up; take one ulp off.
            f[imin] = f32::from_bits(f[imin].to_bits() - 1);
        }
        if f[ia] + f[ib] > limit {
            let d = f[imax] - (limit - f[imin]);
            f[imax] -= d;
        }
    }
}

fn map_rect(m: &Mat, r: [f32; 4]) -> [f32; 4] {
    let a = m.map([r[0], r[1]]);
    let b = m.map([r[2], r[3]]);
    [a[0], a[1], b[0], b[1]]
}

/// `SkRect::centerX` (`sk_float_midpoint`, computed in double).
#[inline]
fn center(a: f32, b: f32) -> f32 {
    ((f64::from(a) + f64::from(b)) * 0.5) as f32
}

/// `SkPath_PointIterator<N>`.
struct PointIter<const N: usize> {
    pts: [P; N],
    current: usize,
    advance: usize,
}

impl<const N: usize> PointIter<N> {
    fn new(pts: [P; N], cw: bool, start: u32) -> Self {
        Self { pts, current: start as usize % N, advance: if cw { 1 } else { N - 1 } }
    }

    fn current(&self) -> P {
        self.pts[self.current]
    }

    fn next(&mut self) -> P {
        self.current = (self.current + self.advance) % N;
        self.current()
    }
}

fn rect_points(r: [f32; 4]) -> [P; 4] {
    [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]]
}

fn oval_points(r: [f32; 4]) -> [P; 4] {
    let cx = center(r[0], r[2]);
    let cy = center(r[1], r[3]);
    [[cx, r[1]], [r[2], cy], [cx, r[3]], [r[0], cy]]
}

fn rrect_points(rr: &SkRRect) -> [P; 8] {
    let [l, t, r, b] = rr.rect;
    let rad = rr.radii;
    [
        [l + rad[0][0], t],
        [r - rad[1][0], t],
        [r, t + rad[1][1]],
        [r, b - rad[2][1]],
        [r - rad[2][0], b],
        [l + rad[3][0], b],
        [l, b - rad[3][1]],
        [l, t + rad[0][1]],
    ]
}

const ROOT2_OVER2: f32 = core::f32::consts::FRAC_1_SQRT_2;

/// `SkPathBuilder::addRect(rect, dir, index)`.
pub fn rect_path(r: [f32; 4], cw: bool, index: u32) -> SkPath {
    let mut it = PointIter::new(rect_points(r), cw, index);
    let mut p = SkPath::new();
    p.move_to(it.current());
    p.line_to(it.next());
    p.line_to(it.next());
    p.line_to(it.next());
    p.close();
    p.mark_rect(true);
    p
}

/// `SkPathBuilder::addOval(oval, dir, index)`: four quarter-circle conics.
pub fn oval_path(r: [f32; 4], cw: bool, index: u32) -> SkPath {
    let mut oval = PointIter::new(oval_points(r), cw, index);
    let mut rect = PointIter::new(rect_points(r), cw, index + u32::from(!cw));
    let mut p = SkPath::new();
    p.move_to(oval.current());
    for _ in 0..4 {
        let c = rect.next();
        let e = oval.next();
        p.conic_to(c, e, ROOT2_OVER2);
    }
    p.close();
    p.mark_oval(r);
    p
}

/// `SkPathBuilder::addRRect(rrect, dir, index)` for a genuine rounded
/// rectangle.
pub fn rrect_path(rr: &SkRRect, cw: bool, index: u32) -> SkPath {
    let starts_with_conic = (index & 1 == 1) == cw;
    let mut rrect = PointIter::new(rrect_points(rr), cw, index);
    let rect_start = index / 2 + u32::from(!cw);
    let mut rect = PointIter::new(rect_points(rr.rect), cw, rect_start);
    let mut p = SkPath::new();
    p.move_to(rrect.current());
    if starts_with_conic {
        for _ in 0..3 {
            let c = rect.next();
            let e = rrect.next();
            p.conic_to(c, e, ROOT2_OVER2);
            p.line_to(rrect.next());
        }
        let c = rect.next();
        let e = rrect.next();
        p.conic_to(c, e, ROOT2_OVER2);
    } else {
        for _ in 0..4 {
            p.line_to(rrect.next());
            let c = rect.next();
            let e = rrect.next();
            p.conic_to(c, e, ROOT2_OVER2);
        }
    }
    p.close();
    p.mark_rrect(*rr);
    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oval_starts_at_three_oclock_for_blink() {
        // Blink's PathBuilder::AddEllipse uses addOval(rect, kCW, 1).
        let p = oval_path([10.0, 20.0, 50.0, 40.0], true, 1);
        assert_eq!(p.pts[0], [50.0, 30.0]);
        assert_eq!(p.verbs.len(), 6);
        assert!(p.is_oval().is_some());
    }

    #[test]
    fn rrect_with_rx_starts_after_the_corner() {
        let rr = SkRRect::make_rect_xy([0.0, 0.0, 100.0, 60.0], 12.0, 12.0);
        assert_eq!(rr.ty, RRectType::Simple);
        let p = rrect_path(&rr, true, 0);
        assert_eq!(p.pts[0], [12.0, 0.0]);
        assert_eq!(p.pts[1], [88.0, 0.0]);
        assert_eq!(p.verbs.len(), 10);
        assert!(p.is_rrect().is_some());
    }

    #[test]
    fn radii_scale_when_too_large() {
        let rr = SkRRect::make_rect_radii([0.0, 0.0, 10.0, 10.0], [[8.0, 8.0]; 4]);
        assert_eq!(rr.radii[0], [5.0, 5.0]);
        assert_eq!(rr.ty, RRectType::Oval);
    }

    #[test]
    fn inner_bounds_inset_by_the_largest_radii() {
        let rr = SkRRect::make_rect_xy([0.0, 0.0, 100.0, 60.0], 12.0, 8.0);
        assert_eq!(rr.inner_bounds(), [12.0, 8.0, 88.0, 52.0]);
    }
}
