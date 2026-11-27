use crate::{Error, Result};
use kurbo::{Affine, BezPath, ParamCurveNearest, PathEl, Point, Shape, Vec2};
use pm_vector::canvas::CanvasPath;

/// Current default path in device space. Path construction captures the CTM;
/// save/restore does not save this object (HTML drawing state model).
#[derive(Debug, Clone, Default)]
pub struct Path {
    pub(crate) data: BezPath,
    current: Option<Point>,
    start: Point,
}
impl Path {
    pub fn elements(&self) -> &[PathEl] {
        self.data.elements()
    }
    pub fn clear(&mut self) {
        *self = Self::default();
    }
    pub fn move_to(&mut self, m: Affine, x: f64, y: f64) {
        let p = m * Point::new(x, y);
        if !p.is_finite() {
            return;
        }
        if matches!(self.data.elements().last(), Some(PathEl::MoveTo(_))) {
            self.data.truncate(self.data.elements().len() - 1);
        }
        self.data.move_to(p);
        self.current = Some(p);
        self.start = p;
    }
    pub fn line_to(&mut self, m: Affine, x: f64, y: f64) {
        let p = m * Point::new(x, y);
        if !p.is_finite() {
            return;
        }
        if self.current.is_none() {
            self.move_to(m, x, y);
        } else {
            self.data.line_to(p);
            self.current = Some(p);
        }
    }
    pub fn quadratic_curve_to(&mut self, m: Affine, c: Point, p: Point) {
        if !c.is_finite() || !p.is_finite() || !(m * c).is_finite() || !(m * p).is_finite() {
            return;
        }
        if self.current.is_none() {
            self.move_to(m, c.x, c.y);
        }
        self.data.quad_to(m * c, m * p);
        self.current = Some(m * p);
    }
    pub fn bezier_curve_to(&mut self, m: Affine, c1: Point, c2: Point, p: Point) {
        if ![c1, c2, p]
            .iter()
            .all(|p| p.is_finite() && (m * *p).is_finite())
        {
            return;
        }
        if self.current.is_none() {
            self.move_to(m, c1.x, c1.y);
        }
        self.data.curve_to(m * c1, m * c2, m * p);
        self.current = Some(m * p);
    }
    pub fn close(&mut self) {
        if self.current.is_some() {
            self.data.close_path();
            self.current = Some(self.start);
        }
    }
    pub fn rect(&mut self, m: Affine, r: kurbo::Rect) {
        if !finite(&[r.x0, r.y0, r.x1, r.y1]) {
            return;
        }
        self.move_to(m, r.x0, r.y0);
        self.line_to(m, r.x1, r.y0);
        self.line_to(m, r.x1, r.y1);
        self.line_to(m, r.x0, r.y1);
        self.close();
    }
    /// Counterclockwise arcs reverse a clockwise native arc, without changing its locus.
    #[allow(clippy::too_many_arguments)]
    pub fn ellipse(
        &mut self,
        m: Affine,
        center: Point,
        radii: Vec2,
        rotation: f64,
        start: f64,
        end: f64,
        ccw: bool,
    ) -> Result<()> {
        if !finite(&[center.x, center.y, radii.x, radii.y, rotation, start, end]) {
            return Ok(());
        }
        if radii.x < 0. || radii.y < 0. {
            return Err(Error::IndexSize);
        }
        let mut native = CanvasPath::new();
        if ccw {
            let sweep = if start - end >= std::f64::consts::TAU {
                std::f64::consts::TAU
            } else {
                (start - end).rem_euclid(std::f64::consts::TAU)
            };
            native
                .ellipse(
                    center.x,
                    center.y,
                    radii.x,
                    radii.y,
                    rotation,
                    start - sweep,
                    start,
                )
                .map_err(|_| Error::IndexSize)?;
            self.append(m, &native.finish().reverse_subpaths(), true);
        } else {
            native
                .ellipse(center.x, center.y, radii.x, radii.y, rotation, start, end)
                .map_err(|_| Error::IndexSize)?;
            self.append(m, &native.finish(), true);
        }
        Ok(())
    }
    pub fn arc_to(&mut self, m: Affine, p1: Point, p2: Point, radius: f64) -> Result<()> {
        if !finite(&[p1.x, p1.y, p2.x, p2.y, radius]) {
            return Ok(());
        }
        if radius < 0. {
            return Err(Error::IndexSize);
        }
        let mut native = CanvasPath::new();
        if let Some(p) = self.current {
            if m.determinant() == 0. {
                return Ok(());
            }
            let p = m.inverse() * p;
            native.move_to(p.x, p.y);
        }
        native
            .arc_to(p1.x, p1.y, p2.x, p2.y, radius)
            .map_err(|_| Error::IndexSize)?;
        self.append(m, &native.finish(), true);
        Ok(())
    }
    /// CSS corner radii order: TL, TR, BR, BL; 1–4 radii, scaled together to fit.
    pub fn round_rect(
        &mut self,
        m: Affine,
        x: f64,
        y: f64,
        w: f64,
        h: f64,
        radii: &[Vec2],
    ) -> Result<()> {
        if radii.is_empty() || radii.len() > 4 {
            return Err(Error::Range);
        }
        if !finite(&[x, y, w, h]) || radii.iter().any(|r| !r.is_finite()) {
            return Ok(());
        }
        if radii.iter().any(|r| r.x < 0. || r.y < 0.) {
            return Err(Error::Range);
        }
        let mut r = match radii {
            [a] => [*a; 4],
            [a, b] => [*a, *b, *a, *b],
            [a, b, c] => [*a, *b, *c, *b],
            [a, b, c, d] => [*a, *b, *c, *d],
            _ => unreachable!(),
        };
        let mut s = 1.0f64;
        for (extent, sum) in [
            (w.abs(), r[0].x + r[1].x),
            (w.abs(), r[2].x + r[3].x),
            (h.abs(), r[0].y + r[3].y),
            (h.abs(), r[1].y + r[2].y),
        ] {
            if sum > 0. {
                s = s.min(extent / sum);
            }
        }
        for a in &mut r {
            *a *= s;
        }
        // Build in signed local coordinates; reflection preserves winding and corner assignment.
        let t = m
            * Affine::translate((x, y))
            * Affine::scale_non_uniform(
                if w < 0. { -1. } else { 1. },
                if h < 0. { -1. } else { 1. },
            );
        let (w, h) = (w.abs(), h.abs());
        self.move_to(t, r[0].x, 0.);
        self.line_to(t, w - r[1].x, 0.);
        self.ellipse(
            t,
            Point::new(w - r[1].x, r[1].y),
            r[1],
            0.,
            -std::f64::consts::FRAC_PI_2,
            0.,
            false,
        )?;
        self.line_to(t, w, h - r[2].y);
        self.ellipse(
            t,
            Point::new(w - r[2].x, h - r[2].y),
            r[2],
            0.,
            0.,
            std::f64::consts::FRAC_PI_2,
            false,
        )?;
        self.line_to(t, r[3].x, h);
        self.ellipse(
            t,
            Point::new(r[3].x, h - r[3].y),
            r[3],
            0.,
            std::f64::consts::FRAC_PI_2,
            std::f64::consts::PI,
            false,
        )?;
        self.line_to(t, 0., r[0].y);
        self.ellipse(
            t,
            Point::new(r[0].x, r[0].y),
            r[0],
            0.,
            std::f64::consts::PI,
            std::f64::consts::PI * 1.5,
            false,
        )?;
        self.close();
        self.move_to(t, 0., 0.);
        Ok(())
    }
    fn append(&mut self, m: Affine, path: &BezPath, connect: bool) {
        for (i, el) in path.iter().enumerate() {
            match el {
                PathEl::MoveTo(p) if i == 0 && connect && self.current.is_some() => {
                    self.line_to(m, p.x, p.y)
                }
                PathEl::MoveTo(p) => self.move_to(m, p.x, p.y),
                PathEl::LineTo(p) => self.line_to(m, p.x, p.y),
                PathEl::QuadTo(c, p) => self.quadratic_curve_to(m, c, p),
                PathEl::CurveTo(a, b, p) => self.bezier_curve_to(m, a, b, p),
                PathEl::ClosePath => self.close(),
            }
        }
    }
    pub fn contains(&self, p: Point, rule: crate::FillRule) -> bool {
        if !p.is_finite() {
            return false;
        }
        if self
            .data
            .segments()
            .any(|s| s.nearest(p, 1e-9).distance_sq <= 1e-18)
        {
            return true;
        }
        let winding = self.data.winding(p);
        match rule {
            crate::FillRule::NonZero => winding != 0,
            crate::FillRule::EvenOdd => winding % 2 != 0,
        }
    }
}
pub(crate) fn finite(v: &[f64]) -> bool {
    v.iter().all(|x| x.is_finite())
}
