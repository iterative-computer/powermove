//! Skia's path model as Blink builds it for Canvas2D (Skia m153
//! `SkPathBuilder`, `SkConic::BuildUnitArc`, `SkMatrix` point mapping,
//! `SkPathPriv::IsRectContour`), in `f32` with the same operation order, so
//! the points the GPU later tessellates are the bits Chromium hands it.
//!
//! Portions are derived from Skia, Copyright Google LLC and The Android Open
//! Source Project, used under the BSD 3-Clause license.

/// A point.
pub type P = [f32; 2];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verb {
    Move,
    Line,
    Quad,
    /// A conic with its weight.
    Conic(f32),
    Cubic,
    Close,
}

/// A path: verbs and points (each verb after the first `Move` consumes 1, 2,
/// or 3 points; `Close` none).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkPath {
    pub verbs: Vec<Verb>,
    pub pts: Vec<P>,
    last_move: Option<usize>,
    /// What `SkPathBuilder::addOval`/`addRRect`/`addRect` recorded
    /// (`SkPathIsAType`): Skia recognizes these shapes later without
    /// inspecting the verbs.
    is_a: IsA,
}

/// `SkPathIsAType` with the shape it records.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub enum IsA {
    #[default]
    General,
    Oval([f32; 4]),
    RRect(super::skrrect::SkRRect),
    Rect,
}

/// `SK_ScalarNearlyZero`.
const NEARLY_ZERO: f32 = 1.0 / 4096.0;
/// `SK_ScalarSinCosNearlyZero`.
const SIN_COS_NEARLY_ZERO: f32 = 1.0 / 65536.0;
const ROOT2_OVER2: f32 = core::f32::consts::FRAC_1_SQRT_2;

#[inline]
fn nearly_zero(x: f32, tol: f32) -> bool {
    x.abs() <= tol
}

#[inline]
fn sin_snap(r: f32) -> f32 {
    let v = libm::sinf(r);
    if nearly_zero(v, SIN_COS_NEARLY_ZERO) { 0.0 } else { v }
}

#[inline]
fn cos_snap(r: f32) -> f32 {
    let v = libm::cosf(r);
    if nearly_zero(v, SIN_COS_NEARLY_ZERO) { 0.0 } else { v }
}

/// `SkScalarNearlyEqual` (tolerance `SK_ScalarNearlyZero`).
#[inline]
fn nearly_equal(a: f32, b: f32) -> bool {
    (a - b).abs() <= NEARLY_ZERO
}

/// An affine `SkMatrix` (`[sx kx tx; ky sy ty]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat {
    pub sx: f32,
    pub kx: f32,
    pub tx: f32,
    pub ky: f32,
    pub sy: f32,
    pub ty: f32,
}

impl Mat {
    pub const IDENTITY: Mat = Mat { sx: 1.0, kx: 0.0, tx: 0.0, ky: 0.0, sy: 1.0, ty: 0.0 };

    pub fn scale_translate(sx: f32, sy: f32, tx: f32, ty: f32) -> Mat {
        Mat { sx, kx: 0.0, tx, ky: 0.0, sy, ty }
    }

    fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }

    fn scale_translate_only(&self) -> bool {
        self.kx == 0.0 && self.ky == 0.0
    }

    /// `mapPoints` (Scale_pts / Affine_vpts: `src·scale + swz·skew + trans`).
    pub fn map(&self, p: P) -> P {
        if self.scale_translate_only() {
            [p[0] * self.sx + self.tx, p[1] * self.sy + self.ty]
        } else {
            [
                p[0] * self.sx + p[1] * self.kx + self.tx,
                p[1] * self.sy + p[0] * self.ky + self.ty,
            ]
        }
    }

    /// `setConcat(a, b)` = a · b.
    pub fn concat(a: &Mat, b: &Mat) -> Mat {
        if a.is_identity() {
            return *b;
        }
        if b.is_identity() {
            return *a;
        }
        if a.scale_translate_only() && b.scale_translate_only() {
            return Mat::scale_translate(
                a.sx * b.sx,
                a.sy * b.sy,
                a.sx * b.tx + a.tx,
                a.sy * b.ty + a.ty,
            );
        }
        let mam = |a: f32, b: f32, c: f32, d: f32| (f64::from(a) * f64::from(b) + f64::from(c) * f64::from(d)) as f32;
        Mat {
            sx: mam(a.sx, b.sx, a.kx, b.ky),
            kx: mam(a.sx, b.kx, a.kx, b.sy),
            tx: mam(a.sx, b.tx, a.kx, b.ty) + a.tx,
            ky: mam(a.ky, b.sx, a.sy, b.ky),
            sy: mam(a.ky, b.kx, a.sy, b.sy),
            ty: mam(a.ky, b.tx, a.sy, b.ty) + a.ty,
        }
    }
}

/// `SkPoint::setLength` (computed through doubles).
fn set_length(v: P, length: f32) -> P {
    let (xx, yy) = (f64::from(v[0]), f64::from(v[1]));
    let dmag = (xx * xx + yy * yy).sqrt();
    let dscale = f64::from(length) / dmag;
    let x = (f64::from(v[0]) * dscale) as f32;
    let y = (f64::from(v[1]) * dscale) as f32;
    if !(x.is_finite() && y.is_finite()) || (x == 0.0 && y == 0.0) {
        return [0.0, 0.0];
    }
    [x, y]
}

/// One conic of an arc.
#[derive(Debug, Clone, Copy)]
struct Conic {
    pts: [P; 3],
    w: f32,
}

/// `SkConic::BuildUnitArc`.
fn build_unit_arc(u_start: P, u_stop: P, cw: bool, user: &Mat) -> Vec<Conic> {
    let x = u_start[0] * u_stop[0] + u_start[1] * u_stop[1];
    let mut y = u_start[0] * u_stop[1] - u_start[1] * u_stop[0];
    let abs_y = y.abs();
    if abs_y <= NEARLY_ZERO && x > 0.0 && ((y >= 0.0 && cw) || (y <= 0.0 && !cw)) {
        return Vec::new();
    }
    if !cw {
        y = -y;
    }
    let mut quadrant = 0usize;
    if y == 0.0 {
        quadrant = 2;
    } else if x == 0.0 {
        quadrant = if y > 0.0 { 1 } else { 3 };
    } else {
        if y < 0.0 {
            quadrant += 2;
        }
        if (x < 0.0) != (y < 0.0) {
            quadrant += 1;
        }
    }
    const Q: [P; 8] = [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0], [-1.0, 0.0], [-1.0, -1.0], [0.0, -1.0], [1.0, -1.0]];
    let mut out: Vec<Conic> = (0..quadrant)
        .map(|i| Conic { pts: [Q[i * 2], Q[i * 2 + 1], Q[(i * 2 + 2) % 8]], w: ROOT2_OVER2 })
        .collect();
    let final_p = [x, y];
    let last_q = Q[quadrant * 2];
    let dot = last_q[0] * final_p[0] + last_q[1] * final_p[1];
    if dot.is_nan() {
        return Vec::new();
    }
    if dot < 1.0 {
        let off = [last_q[0] + x, last_q[1] + y];
        let cos_half = ((1.0 + dot) / 2.0).sqrt();
        let off = set_length(off, 1.0 / cos_half);
        // `SkPointPriv::EqualsWithinTolerance(lastQ, offCurve)`.
        let within = (off[0] - last_q[0]).abs() <= NEARLY_ZERO && (off[1] - last_q[1]).abs() <= NEARLY_ZERO;
        if !within {
            out.push(Conic { pts: [last_q, off, final_p], w: cos_half });
        }
    }
    // Rotate by the start vector (setSinCos), mirror for counter-clockwise,
    // then the user matrix.
    let mut m = Mat { sx: u_start[0], kx: -u_start[1], tx: 0.0, ky: u_start[1], sy: u_start[0], ty: 0.0 };
    if !cw {
        // preScale(1, -1).
        m.kx = -m.kx;
        m.sy = -m.sy;
    }
    let m = Mat::concat(user, &m);
    for c in &mut out {
        for p in &mut c.pts {
            *p = m.map(*p);
        }
    }
    out
}

impl SkPath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.verbs.is_empty()
    }

    pub fn last_pt(&self) -> Option<P> {
        self.pts.last().copied()
    }

    /// `SkPath::isOval`: the oval's bounds when the path was built by
    /// `addOval` (and only that).
    pub fn is_oval(&self) -> Option<[f32; 4]> {
        match self.is_a {
            IsA::Oval(r) => Some(r),
            _ => None,
        }
    }

    /// `SkPath::isRRect`.
    pub fn is_rrect(&self) -> Option<super::skrrect::SkRRect> {
        match self.is_a {
            IsA::RRect(r) => Some(r),
            _ => None,
        }
    }

    pub(crate) fn mark_oval(&mut self, r: [f32; 4]) {
        self.is_a = IsA::Oval(r);
    }

    pub(crate) fn mark_rrect(&mut self, r: super::skrrect::SkRRect) {
        self.is_a = IsA::RRect(r);
    }

    pub(crate) fn mark_rect(&mut self, _r: bool) {
        self.is_a = IsA::Rect;
    }

    /// `SkPathBuilder::transform` for an affine matrix: every point mapped
    /// (`SkMatrix::mapPoints`), conic weights kept; the oval/rrect identity
    /// survives a rect-stays-rect transform as Skia keeps it.
    pub fn transform(&self, m: &Mat) -> SkPath {
        if *m == Mat::IDENTITY || self.verbs.is_empty() {
            return self.clone();
        }
        let mut out = self.clone();
        for p in &mut out.pts {
            *p = m.map(*p);
        }
        let rect_stays_rect = (m.kx == 0.0 && m.ky == 0.0) || (m.sx == 0.0 && m.sy == 0.0);
        out.is_a = match (rect_stays_rect, self.is_a) {
            (true, IsA::Oval(r)) => {
                let a = m.map([r[0], r[1]]);
                let b = m.map([r[2], r[3]]);
                IsA::Oval([a[0].min(b[0]), a[1].min(b[1]), a[0].max(b[0]), a[1].max(b[1])])
            }
            (true, IsA::RRect(rr)) => match rr.transform(m) {
                Some(t) => IsA::RRect(t),
                None => IsA::General,
            },
            (true, IsA::Rect) => IsA::Rect,
            _ => IsA::General,
        };
        out
    }

    pub fn move_to(&mut self, p: P) {
        if let Some(Verb::Move) = self.verbs.last() {
            *self.pts.last_mut().unwrap() = p;
        } else {
            self.last_move = Some(self.pts.len());
            self.pts.push(p);
            self.verbs.push(Verb::Move);
        }
    }

    /// `ensureMove`: a verb after `Close` (or on an empty path) starts at the
    /// last move point (or the origin).
    fn ensure_move(&mut self) {
        match self.verbs.last() {
            None => self.move_to([0.0, 0.0]),
            Some(Verb::Close) => {
                let p = self.last_move.map_or([0.0, 0.0], |i| self.pts[i]);
                self.move_to(p);
            }
            _ => {}
        }
    }

    pub fn line_to(&mut self, p: P) {
        self.ensure_move();
        self.pts.push(p);
        self.verbs.push(Verb::Line);
    }

    pub fn quad_to(&mut self, p1: P, p2: P) {
        self.ensure_move();
        self.pts.extend([p1, p2]);
        self.verbs.push(Verb::Quad);
    }

    pub fn conic_to(&mut self, p1: P, p2: P, w: f32) {
        self.ensure_move();
        if w <= 0.0 {
            self.line_to(p2);
            return;
        }
        self.pts.extend([p1, p2]);
        if w == 1.0 {
            self.verbs.push(Verb::Quad);
        } else if w.is_finite() {
            self.verbs.push(Verb::Conic(w));
        } else {
            self.verbs.push(Verb::Line);
            self.verbs.push(Verb::Line);
        }
    }

    pub fn cubic_to(&mut self, p1: P, p2: P, p3: P) {
        self.ensure_move();
        self.pts.extend([p1, p2, p3]);
        self.verbs.push(Verb::Cubic);
    }

    pub fn close(&mut self) {
        if !self.verbs.is_empty() && self.verbs.last() != Some(&Verb::Close) {
            self.ensure_move();
            self.verbs.push(Verb::Close);
        }
    }

    /// `arcTo(oval, startAngle, sweepAngle, forceMoveTo)` (degrees).
    pub fn arc_to_oval(&mut self, oval: [f32; 4], start: f32, sweep: f32, mut force_move: bool) {
        let [l, t, r, b] = oval;
        let (w, h) = (r - l, b - t);
        if w < 0.0 || h < 0.0 {
            return;
        }
        let start = libm::fmodf(start, 360.0);
        if self.verbs.is_empty() {
            force_move = true;
        }
        // arc_is_lone_point.
        let lone = if sweep == 0.0 && (start == 0.0 || start == 360.0) {
            Some([r, center(t, b)])
        } else if w == 0.0 && h == 0.0 {
            Some([r, t])
        } else {
            None
        };
        if let Some(p) = lone {
            if force_move { self.move_to(p) } else { self.line_to(p) }
            return;
        }
        // angles_to_unit_vectors.
        let start_rad = start * (core::f32::consts::PI / 180.0);
        let mut stop_rad = (start + sweep) * (core::f32::consts::PI / 180.0);
        let start_v = [cos_snap(start_rad), sin_snap(start_rad)];
        let mut stop_v = [cos_snap(stop_rad), sin_snap(stop_rad)];
        if start_v == stop_v {
            let sw = sweep.abs();
            if sw < 360.0 && sw > 359.0 {
                let delta = (1.0f32 / 512.0).copysign(sweep);
                loop {
                    stop_rad -= delta;
                    stop_v = [cos_snap(stop_rad), sin_snap(stop_rad)];
                    if start_v != stop_v {
                        break;
                    }
                }
            }
        }
        let cw = sweep > 0.0;
        let add_pt = |this: &mut Self, p: P| {
            if force_move {
                this.move_to(p);
            } else {
                let last = *this.pts.last().unwrap();
                if !(nearly_equal(last[0], p[0]) && nearly_equal(last[1], p[1])) {
                    this.line_to(p);
                }
            }
        };
        if start_v == stop_v {
            let end = (start + sweep) * (core::f32::consts::PI / 180.0);
            let (rx, ry) = (w / 2.0, h / 2.0);
            let p = [center(l, r) + rx * libm::cosf(end), center(t, b) + ry * libm::sinf(end)];
            add_pt(self, p);
            return;
        }
        // build_arc_conics.
        let user = Mat::scale_translate(w / 2.0, h / 2.0, center(l, r), center(t, b));
        let conics = build_unit_arc(start_v, stop_v, cw, &user);
        if conics.is_empty() {
            add_pt(self, user.map(stop_v));
        } else {
            add_pt(self, conics[0].pts[0]);
            for c in &conics {
                self.conic_to(c.pts[1], c.pts[2], c.w);
            }
        }
    }

    /// `arcTo(p1, p2, radius)` (the tangent arc of Canvas2D `arcTo`).
    pub fn arc_to_tangent(&mut self, p1: P, p2: P, radius: f32) {
        self.ensure_move();
        if radius == 0.0 {
            self.line_to(p1);
            return;
        }
        let start = *self.pts.last().unwrap();
        let norm = |x: f64, y: f64| {
            let l = (x * x + y * y).sqrt();
            (x / l, y / l)
        };
        let before = norm(f64::from(p1[0] - start[0]), f64::from(p1[1] - start[1]));
        let after = norm(f64::from(p2[0] - p1[0]), f64::from(p2[1] - p1[1]));
        let cosh = before.0 * after.0 + before.1 * after.1;
        let sinh = before.0 * after.1 - before.1 * after.0;
        let finite = |v: (f64, f64)| v.0.is_finite() && v.1.is_finite();
        if !finite(before) || !finite(after) || nearly_zero(sinh as f32, NEARLY_ZERO) {
            self.line_to(p1);
            return;
        }
        let dist = ((f64::from(radius) * (1.0 - cosh) / sinh) as f32).abs();
        let xx = p1[0] - dist * before.0 as f32;
        let yy = p1[1] - dist * before.1 as f32;
        let after_v = set_length([after.0 as f32, after.1 as f32], dist);
        self.line_to([xx, yy]);
        let weight = ((0.5 + cosh * 0.5) as f32).sqrt();
        self.conic_to(p1, [p1[0] + after_v[0], p1[1] + after_v[1]], weight);
    }

    /// The SVG elliptical arc `arcTo(radii, xAxisRotate, largeArc, sweep, end)`.
    pub fn arc_to_svg(&mut self, rad: P, angle: f32, large: bool, cw: bool, end: P) {
        self.ensure_move();
        let src = [*self.pts.last().unwrap(), end];
        if rad[0] == 0.0 || rad[1] == 0.0 || src[0] == src[1] {
            self.line_to(end);
            return;
        }
        let mut rx = rad[0].abs();
        let mut ry = rad[1].abs();
        let mid = [(src[0][0] - src[1][0]) * 0.5, (src[0][1] - src[1][1]) * 0.5];
        let rot = |deg: f32| {
            let r = deg * (core::f32::consts::PI / 180.0);
            (sin_snap(r), cos_snap(r))
        };
        // pointTransform.setRotate(-angle).
        let (s, c) = rot(-angle);
        let rotm = Mat { sx: c, kx: -s, tx: 0.0, ky: s, sy: c, ty: 0.0 };
        let tm = rotm.map(mid);
        let (sq_rx, sq_ry) = (rx * rx, ry * ry);
        let (sq_x, sq_y) = (tm[0] * tm[0], tm[1] * tm[1]);
        let mut radii_scale = sq_x / sq_rx + sq_y / sq_ry;
        if radii_scale > 1.0 {
            radii_scale = radii_scale.sqrt();
            rx *= radii_scale;
            ry *= radii_scale;
        }
        // setScale(1/rx, 1/ry).preRotate(-angle).
        let scale = Mat::scale_translate(1.0 / rx, 1.0 / ry, 0.0, 0.0);
        let pt = Mat::concat(&scale, &rotm);
        let mut unit = [pt.map(src[0]), pt.map(src[1])];
        let mut delta = [unit[1][0] - unit[0][0], unit[1][1] - unit[0][1]];
        let d = delta[0] * delta[0] + delta[1] * delta[1];
        let sfs = (1.0 / d - 0.25).max(0.0);
        let mut sf = sfs.sqrt();
        if cw == large {
            sf = -sf;
        }
        delta = [delta[0] * sf, delta[1] * sf];
        let mut cpt = [unit[0][0] + unit[1][0], unit[0][1] + unit[1][1]];
        cpt = [cpt[0] * 0.5, cpt[1] * 0.5];
        cpt = [cpt[0] - delta[1], cpt[1] + delta[0]];
        unit[0] = [unit[0][0] - cpt[0], unit[0][1] - cpt[1]];
        unit[1] = [unit[1][0] - cpt[0], unit[1][1] - cpt[1]];
        let theta1 = libm::atan2f(unit[0][1], unit[0][0]);
        let theta2 = libm::atan2f(unit[1][1], unit[1][0]);
        let mut theta_arc = theta2 - theta1;
        let pi = core::f32::consts::PI;
        if theta_arc < 0.0 && cw {
            theta_arc += pi * 2.0;
        } else if theta_arc > 0.0 && !cw {
            theta_arc -= pi * 2.0;
        }
        if theta_arc.abs() < pi / (1000.0 * 1000.0) {
            self.line_to(end);
            return;
        }
        // pointTransform.setRotate(angle).preScale(rx, ry).
        let (s, c) = rot(angle);
        let rotm = Mat { sx: c, kx: -s, tx: 0.0, ky: s, sy: c, ty: 0.0 };
        let pt = Mat::concat(&rotm, &Mat::scale_translate(rx, ry, 0.0, 0.0));
        let segments = libm::ceilf((theta_arc / (2.0 * pi / 3.0)).abs()) as i32;
        let theta_width = theta_arc / segments as f32;
        let t = libm::tanf(0.5 * theta_width);
        if !t.is_finite() {
            return;
        }
        let mut start_theta = theta1;
        let w = (0.5 + libm::cosf(theta_width) * 0.5).sqrt();
        let is_int = |v: f32| v == libm::floorf(v);
        let expect_ints = nearly_zero(pi / 2.0 - theta_width.abs(), NEARLY_ZERO)
            && is_int(rx)
            && is_int(ry)
            && is_int(end[0])
            && is_int(end[1]);
        for _ in 0..segments {
            let end_theta = start_theta + theta_width;
            let (se, ce) = (sin_snap(end_theta), cos_snap(end_theta));
            let mut u1 = [ce, se];
            u1 = [u1[0] + cpt[0], u1[1] + cpt[1]];
            let u0 = [u1[0] + t * se, u1[1] - t * ce];
            let mut mapped = [pt.map(u0), pt.map(u1)];
            if expect_ints {
                for p in &mut mapped {
                    *p = [round_scalar(p[0]), round_scalar(p[1])];
                }
            }
            self.conic_to(mapped[0], mapped[1], w);
            start_theta = end_theta;
        }
        *self.pts.last_mut().unwrap() = end;
    }

    /// The points of verb `i` (the previous point first), as Skia iterates.
    pub fn segments(&self) -> Vec<(Verb, Vec<P>)> {
        let mut out = Vec::new();
        let mut pi = 0usize;
        let mut last = [0.0f32; 2];
        let mut start = [0.0f32; 2];
        for v in &self.verbs {
            match v {
                Verb::Move => {
                    last = self.pts[pi];
                    start = last;
                    pi += 1;
                    out.push((*v, vec![last]));
                }
                Verb::Line => {
                    let p = self.pts[pi];
                    out.push((*v, vec![last, p]));
                    last = p;
                    pi += 1;
                }
                Verb::Quad | Verb::Conic(_) => {
                    let (a, b) = (self.pts[pi], self.pts[pi + 1]);
                    out.push((*v, vec![last, a, b]));
                    last = b;
                    pi += 2;
                }
                Verb::Cubic => {
                    let (a, b, c) = (self.pts[pi], self.pts[pi + 1], self.pts[pi + 2]);
                    out.push((*v, vec![last, a, b, c]));
                    last = c;
                    pi += 3;
                }
                Verb::Close => {
                    out.push((*v, vec![last, start]));
                    last = start;
                }
            }
        }
        out
    }

    /// `SkPath::isLine`: exactly a move and a line.
    pub fn is_line(&self) -> Option<[P; 2]> {
        (self.verbs.len() == 2 && self.verbs[0] == Verb::Move && self.verbs[1] == Verb::Line)
            .then(|| [self.pts[0], self.pts[1]])
    }

    /// `SkPath::isRect`: the rectangle and whether the contour is closed.
    pub fn is_rect(&self) -> Option<([f32; 4], bool)> {
        is_rect_contour(&self.pts, &self.verbs)
    }

    /// Whether `SkPathPriv::IsNestedFillRects` may accept the path, which
    /// makes Graphite's `Device::drawPath` draw it as `drawRect` or
    /// `drawDRRect` instead of a path. Conservative: true whenever the
    /// first two contours are rect contours and one contains the other
    /// (Skia also requires that nothing but empty contours follow, that the
    /// path has only lines, and accepts an unclosed first contour exactly
    /// as `is_rect` does).
    pub fn maybe_nested_fill_rects(&self) -> bool {
        let mut contours = Vec::new();
        let (mut vstart, mut pstart, mut pi) = (0usize, 0usize, 0usize);
        for (i, v) in self.verbs.iter().enumerate() {
            if *v == Verb::Move && i > vstart {
                contours.push((vstart..i, pstart..pi));
                vstart = i;
                pstart = pi;
            }
            pi += match v {
                Verb::Move | Verb::Line => 1,
                Verb::Quad | Verb::Conic(_) => 2,
                Verb::Cubic => 3,
                Verb::Close => 0,
            };
        }
        contours.push((vstart..self.verbs.len(), pstart..pi));
        let rect = |(v, p): &(std::ops::Range<usize>, std::ops::Range<usize>)| is_rect_contour(&self.pts[p.clone()], &self.verbs[v.clone()]).map(|r| r.0);
        let (Some(a), Some(b)) = (contours.first().and_then(rect), contours.get(1).and_then(rect)) else {
            return false;
        };
        let contains = |o: [f32; 4], i: [f32; 4]| o[0] <= i[0] && o[1] <= i[1] && o[2] >= i[2] && o[3] >= i[3];
        contains(a, b) || contains(b, a)
    }

    /// Bounds of the points (`computeBounds`), or `None` when empty.
    pub fn bounds(&self) -> Option<[f32; 4]> {
        let first = self.pts.first()?;
        let mut b = [first[0], first[1], first[0], first[1]];
        for p in &self.pts {
            b[0] = b[0].min(p[0]);
            b[1] = b[1].min(p[1]);
            b[2] = b[2].max(p[0]);
            b[3] = b[3].max(p[1]);
        }
        Some(b)
    }
}

#[inline]
fn center(a: f32, b: f32) -> f32 {
    // SkRect::centerX: sk_float_midpoint (computed in double).
    ((f64::from(a) + f64::from(b)) * 0.5) as f32
}

#[inline]
fn round_scalar(v: f32) -> f32 {
    // SkScalarRoundToScalar: floor(x + 0.5).
    libm::floorf(v + 0.5)
}

fn rect_make_dir(dx: f32, dy: f32) -> i32 {
    i32::from(dx != 0.0) | (i32::from(dx > 0.0 || dy > 0.0) << 1)
}

/// `SkPathPriv::IsRectContour(pts, verbs, segmentMask, allowPartial = false)`.
fn is_rect_contour(pts: &[P], verbs: &[Verb]) -> Option<([f32; 4], bool)> {
    if verbs.iter().any(|v| matches!(v, Verb::Quad | Verb::Conic(_) | Verb::Cubic))
        || !verbs.contains(&Verb::Line)
        || pts.len() < 4
        || verbs.len() < 4
    {
        return None;
    }
    // trivial_rect: move, line, line, line, close with four points.
    if pts.len() == 4 && verbs == [Verb::Move, Verb::Line, Verb::Line, Verb::Line, Verb::Close] {
        let v = |a: usize, b: usize| [pts[b][0] - pts[a][0], pts[b][1] - pts[a][1]];
        let (v0, v1, v2, v3) = (v(0, 1), v(1, 2), v(2, 3), v(3, 0));
        let ortho = |a: P, b: P| ((a[0] == 0.0) ^ (b[0] == 0.0)) & ((a[1] == 0.0) ^ (b[1] == 0.0));
        if ((v0[0] == 0.0) ^ (v0[1] == 0.0)) & ortho(v0, v1) & ortho(v1, v2) & ortho(v2, v3) {
            let r = sorted([pts[0][0], pts[0][1], pts[2][0], pts[2][1]]);
            return Some((r, true));
        }
        return None;
    }
    let mut corners = 0;
    let mut line_start = [0.0f32, 0.0];
    let mut first_pt: Option<usize> = None;
    let mut last_pt: Option<usize> = None;
    let mut first_corner = [0.0f32; 2];
    let mut third_corner = [0.0f32; 2];
    let mut directions = [-1i32; 5];
    let mut closed_or_moved = false;
    let mut auto_close = false;
    let mut pi = 0usize;
    for &verb in verbs {
        match verb {
            Verb::Close | Verb::Line => {
                if verb == Verb::Close {
                    auto_close = true;
                } else {
                    last_pt = Some(pi);
                }
                let line_end = if verb == Verb::Close {
                    pts[first_pt?]
                } else {
                    let p = pts[pi];
                    pi += 1;
                    p
                };
                let d = [line_end[0] - line_start[0], line_end[1] - line_start[1]];
                if d[0] != 0.0 && d[1] != 0.0 {
                    return None;
                }
                if !(d[0].is_finite() && d[1].is_finite()) {
                    return None;
                }
                if line_start == line_end {
                    continue;
                }
                let next = rect_make_dir(d[0], d[1]);
                if corners == 0 {
                    directions[0] = next;
                    corners = 1;
                    closed_or_moved = false;
                    line_start = line_end;
                    continue;
                }
                if closed_or_moved {
                    return None;
                }
                if auto_close && next == directions[0] {
                    continue;
                }
                closed_or_moved = auto_close;
                if directions[corners - 1] == next {
                    if corners == 3 && verb == Verb::Line {
                        third_corner = line_end;
                    }
                    line_start = line_end;
                    continue;
                }
                directions[corners] = next;
                corners += 1;
                match corners {
                    2 => first_corner = line_start,
                    3 => {
                        if (directions[0] ^ directions[2]) != 2 {
                            return None;
                        }
                        third_corner = line_end;
                    }
                    4 => {
                        if (directions[1] ^ directions[3]) != 2 {
                            return None;
                        }
                    }
                    _ => return None,
                }
                line_start = line_end;
            }
            Verb::Move => {
                if corners == 0 {
                    first_pt = Some(pi);
                } else {
                    let (f, l) = (pts[first_pt?], pts[last_pt?]);
                    if f[0] - l[0] != 0.0 && f[1] - l[1] != 0.0 {
                        return None;
                    }
                }
                line_start = pts[pi];
                pi += 1;
                closed_or_moved = true;
            }
            _ => return None,
        }
    }
    if !(3..=4).contains(&corners) {
        return None;
    }
    let (f, l) = (pts[first_pt?], pts[last_pt.unwrap_or(first_pt?)]);
    if f[0] - l[0] != 0.0 && f[1] - l[1] != 0.0 {
        return None;
    }
    let r = sorted([first_corner[0], first_corner[1], third_corner[0], third_corner[1]]);
    Some((r, auto_close))
}

fn sorted(r: [f32; 4]) -> [f32; 4] {
    [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_ellipse_is_four_quadrant_conics() {
        // Blink: arcTo(oval, 0, 180) then arcTo(oval, 180, 180).
        let mut p = SkPath::new();
        let oval = [4.0, 4.0, 124.0, 94.0];
        p.arc_to_oval(oval, 0.0, 180.0, false);
        p.arc_to_oval(oval, 180.0, 180.0, false);
        let conics = p.verbs.iter().filter(|v| matches!(v, Verb::Conic(_))).count();
        assert_eq!(conics, 4);
        assert_eq!(p.pts[0], [124.0, 49.0]);
        assert_eq!(*p.pts.last().unwrap(), [124.0, 49.0]);
    }

    #[test]
    fn tangent_arc_rounds_a_corner() {
        let mut p = SkPath::new();
        p.move_to([10.0, 0.0]);
        p.arc_to_tangent([100.0, 0.0], [100.0, 50.0], 10.0);
        assert_eq!(p.verbs.len(), 3);
        assert!(matches!(p.verbs[2], Verb::Conic(w) if (w - ROOT2_OVER2).abs() < 1e-6));
        assert_eq!(p.pts[1], [90.0, 0.0]);
        assert_eq!(p.pts[3], [100.0, 10.0]);
    }

    #[test]
    fn rects_are_detected() {
        let mut p = SkPath::new();
        p.move_to([0.0, 0.0]);
        for q in [[10.0, 0.0], [10.0, 5.0], [0.0, 5.0], [0.0, 0.0]] {
            p.line_to(q);
        }
        p.close();
        assert_eq!(p.is_rect(), Some(([0.0, 0.0, 10.0, 5.0], true)));
        let mut tri = SkPath::new();
        tri.move_to([0.0, 0.0]);
        tri.line_to([10.0, 0.0]);
        tri.line_to([5.0, 5.0]);
        tri.close();
        assert_eq!(tri.is_rect(), None);
        assert!(!tri.maybe_nested_fill_rects());
        let mut nested = SkPath::new();
        for r in [[5.0, 5.0, 95.0, 55.0], [25.0, 15.0, 75.0, 45.0]] {
            nested.move_to([r[0], r[1]]);
            nested.line_to([r[2], r[1]]);
            nested.line_to([r[2], r[3]]);
            nested.line_to([r[0], r[3]]);
            nested.close();
        }
        assert!(nested.maybe_nested_fill_rects());
        let mut apart = SkPath::new();
        for r in [[0.0, 0.0, 10.0, 10.0], [20.0, 0.0, 30.0, 10.0]] {
            apart.move_to([r[0], r[1]]);
            apart.line_to([r[2], r[1]]);
            apart.line_to([r[2], r[3]]);
            apart.line_to([r[0], r[3]]);
            apart.close();
        }
        assert!(!apart.maybe_nested_fill_rects());
    }
}
