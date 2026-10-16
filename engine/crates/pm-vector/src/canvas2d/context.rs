//! A Canvas2D context as Chromium 152 draws it on Skia Graphite (Metal):
//! Blink's state and paths, recorded paint operations replayed by a
//! Graphite device into a 4× multisampled RGBA8 target, resolved to the
//! bitmap the legacy compositor uploads.

use super::blink::{CanvasPath, Drawn, sat_f32};
use super::msaa::{MsaaTarget, Rule, SAMPLES_1X, SAMPLES_4X, fill_polygons};
use super::skpath::{Mat, P, SkPath};
use super::analytic;
use super::stroke::{self, Cap, StrokeParams};
use super::tess;
use crate::render::color::{CanvasColor, StyleSlot};

/// `lineJoin`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

/// `lineCap`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Butt,
    Round,
    Square,
}

/// A draw the emulation does not reproduce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unsupported(pub &'static str);

impl core::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Canvas2D emulation does not draw {}", self.0)
    }
}

impl std::error::Error for Unsupported {}

#[derive(Debug, Clone)]
struct State {
    fill: StyleSlot,
    stroke: StyleSlot,
    line_width: f32,
    join: LineJoin,
    cap: LineCap,
    miter_limit: f32,
    global_alpha: f64,
    /// Blink's `AffineTransform` `[a, b, c, d, e, f]` in double (decides
    /// whether a call changes the transform at all).
    ctm: [f64; 6],
    /// The playback canvas's matrix (`SkM44`, column-major, `f32`): Blink
    /// forwards each call to the recording canvas, so Skia composes this.
    m44: [f32; 16],
}

impl Default for State {
    fn default() -> Self {
        Self {
            fill: StyleSlot::default(),
            stroke: StyleSlot::default(),
            line_width: 1.0,
            join: LineJoin::Miter,
            cap: LineCap::Butt,
            miter_limit: 10.0,
            global_alpha: 1.0,
            ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            m44: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
}

/// One recorded draw, rasterized when the canvas is finished.
#[derive(Debug, Clone)]
enum Op {
    /// Closed polygons in window coordinates with the nonzero rule: the
    /// tessellated fill renderers (which require MSAA) or the non-AA bounds
    /// renderer (which does not).
    Polygons { polys: Vec<Vec<[f64; 2]>>, color: [f32; 4], msaa: bool },
    /// The union of triangles (the tessellated stroke renderer, MSAA).
    Triangles { tris: Vec<[P; 3]>, color: [f32; 4] },
    /// One `AnalyticRRectRenderStep` instance (no MSAA requirement).
    Analytic { inst: analytic::Instance, color: [f32; 4] },
}

/// A Canvas2D context on a `width` × `height` canvas.
pub struct Context {
    width: u32,
    height: u32,
    ops: Vec<Op>,
    state: State,
    stack: Vec<State>,
    pub path: CanvasPath,
}

fn pre_concat(m: [f64; 6], n: [f64; 6]) -> [f64; 6] {
    [
        m[0] * n[0] + m[2] * n[1],
        m[1] * n[0] + m[3] * n[1],
        m[0] * n[2] + m[2] * n[3],
        m[1] * n[2] + m[3] * n[3],
        m[0] * n[4] + m[2] * n[5] + m[4],
        m[1] * n[4] + m[3] * n[5] + m[5],
    ]
}

/// `SkM44::setConcat(a, b)`: each column `c0*r0 + (c1*r1 + (c2*r2 + c3*r3))`.
fn m44_concat(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        let r = &b[col * 4..col * 4 + 4];
        for i in 0..4 {
            out[col * 4 + i] = a[i] * r[0] + (a[4 + i] * r[1] + (a[8 + i] * r[2] + a[12 + i] * r[3]));
        }
    }
    out
}

impl Context {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            ops: Vec::new(),
            state: State::default(),
            stack: Vec::new(),
            path: CanvasPath::new(),
        }
    }

    pub fn save(&mut self) {
        self.stack.push(self.state.clone());
    }

    pub fn restore(&mut self) {
        if let Some(s) = self.stack.pop() {
            self.state = s;
        }
    }

    /// `scale(sx, sy)`: Blink's float arguments, `SkCanvas::scale`
    /// (`SkM44::preScale`).
    pub fn scale(&mut self, sx: f64, sy: f64) {
        if !(sx.is_finite() && sy.is_finite()) {
            return;
        }
        let (fx, fy) = (sat_f32(sx), sat_f32(sy));
        let mut m = self.state.ctm;
        m[0] *= f64::from(fx);
        m[1] *= f64::from(fx);
        m[2] *= f64::from(fy);
        m[3] *= f64::from(fy);
        if m == self.state.ctm {
            return;
        }
        self.state.ctm = m;
        if fx != 1.0 || fy != 1.0 {
            let k = &mut self.state.m44;
            for i in 0..4 {
                k[i] *= fx;
                k[4 + i] *= fy;
            }
        }
    }

    /// `translate(tx, ty)`: `SkM44::preTranslate`.
    pub fn translate(&mut self, tx: f64, ty: f64) {
        if !(tx.is_finite() && ty.is_finite()) {
            return;
        }
        let (fx, fy) = (sat_f32(tx), sat_f32(ty));
        let mut m = self.state.ctm;
        m[4] += f64::from(fx) * m[0] + f64::from(fy) * m[2];
        m[5] += f64::from(fx) * m[1] + f64::from(fy) * m[3];
        if m == self.state.ctm {
            return;
        }
        self.state.ctm = m;
        if fx != 0.0 || fy != 0.0 {
            let k = &mut self.state.m44;
            for i in 0..4 {
                // c0*x + (c1*y + (c2*z + c3)) with z = 0.
                k[12 + i] = k[i] * fx + (k[4 + i] * fy + (k[8 + i] * 0.0 + k[12 + i]));
            }
        }
    }

    /// `rotate(radians)`: degrees in float, `SkMatrix::setRotate`, then
    /// `SkM44::preConcat`.
    pub fn rotate(&mut self, radians: f64) {
        if !radians.is_finite() {
            return;
        }
        let (s, c) = (libm::sin(radians), libm::cos(radians));
        let m = pre_concat(self.state.ctm, [c, s, -s, c, 0.0, 0.0]);
        if m == self.state.ctm {
            return;
        }
        self.state.ctm = m;
        let degrees = sat_f32(radians * (180.0 / f64::from(core::f32::consts::PI)));
        let rad = degrees * (core::f32::consts::PI / 180.0);
        let snap = |v: f32| if v.abs() <= 1.0 / 65536.0 { 0.0 } else { v };
        let (sn, cs) = (snap(libm::sinf(rad)), snap(libm::cosf(rad)));
        if sn == 0.0 && cs == 1.0 {
            return;
        }
        // The rotation as a 4x4 (column-major): [cs sn 0 0], [-sn cs 0 0], ...
        let r = [cs, sn, 0.0, 0.0, -sn, cs, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
        self.state.m44 = m44_concat(&self.state.m44, &r);
    }

    /// `transform(a, b, c, d, e, f)`: float arguments, `SkM44::preConcat`.
    pub fn transform(&mut self, t: [f64; 6]) {
        if !t.iter().all(|v| v.is_finite()) {
            return;
        }
        let f = t.map(sat_f32);
        let m = pre_concat(self.state.ctm, f.map(f64::from));
        if m == self.state.ctm {
            return;
        }
        self.state.ctm = m;
        let r = [f[0], f[1], 0.0, 0.0, f[2], f[3], 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, f[4], f[5], 0.0, 1.0];
        if r != [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
            self.state.m44 = m44_concat(&self.state.m44, &r);
        }
    }

    pub fn set_fill_style(&mut self, s: &str) {
        self.state.fill.set(s);
    }

    pub fn set_stroke_style(&mut self, s: &str) {
        self.state.stroke.set(s);
    }

    /// `lineWidth = v` (non-finite and non-positive values are ignored).
    pub fn set_line_width(&mut self, v: f64) {
        if v.is_finite() && v > 0.0 {
            self.state.line_width = sat_f32(v);
        }
    }

    pub fn set_line_join(&mut self, j: LineJoin) {
        self.state.join = j;
    }

    pub fn set_line_cap(&mut self, c: LineCap) {
        self.state.cap = c;
    }

    /// `globalAlpha = v` (values outside 0..1 and non-finite are ignored).
    pub fn set_global_alpha(&mut self, v: f64) {
        if v.is_finite() && (0.0..=1.0).contains(&v) {
            self.state.global_alpha = v;
        }
    }

    pub fn begin_path(&mut self) {
        self.path = CanvasPath::new();
    }

    /// The device's local-to-device matrix (the 2D part of the `SkM44`).
    pub fn matrix(&self) -> Mat {
        let k = &self.state.m44;
        Mat {
            sx: k[0],
            ky: k[1],
            kx: k[4],
            sy: k[5],
            tx: k[12],
            ty: k[13],
        }
    }

    /// The premultiplied paint color.
    fn paint(&self, c: CanvasColor) -> [f32; 4] {
        c.with_alpha_scale(self.state.global_alpha as f32).premultiplied()
    }

    /// `fill()` of the current path.
    pub fn fill(&mut self) -> Result<(), Unsupported> {
        let path = self.path.clone();
        match path.drawn() {
            Drawn::Empty | Drawn::Line(..) => Ok(()),
            Drawn::Path(p) => self.fill_path(p),
        }
    }

    /// `fill(path2d)` / the fill of an `SkPath` (`Device::drawPath`).
    pub fn fill_path(&mut self, p: &SkPath) -> Result<(), Unsupported> {
        if p.is_line().is_some() {
            return Ok(());
        }
        let m = self.matrix();
        let color = self.paint(self.state.fill.0);
        if let Some((r, _)) = p.is_rect() {
            return self.fill_rect(r, &m, color);
        }
        let local = tess::fill_polygons_local(p, &m);
        let polys = tess::to_device(&local, &m, self.width, self.height);
        self.ops.push(Op::Polygons { polys, color, msaa: true });
        Ok(())
    }

    /// `stroke()` of the current path.
    pub fn stroke(&mut self) -> Result<(), Unsupported> {
        let path = self.path.clone();
        match path.drawn() {
            Drawn::Empty => Ok(()),
            Drawn::Line(a, b) => self.stroke_line(a, b),
            Drawn::Path(p) => self.stroke_path(p),
        }
    }

    fn stroke_line(&mut self, a: P, b: P) -> Result<(), Unsupported> {
        let m = self.matrix();
        let (params, cap) = self.stroke_params();
        let color = self.paint(self.state.stroke.0);
        let g = analytic::Geometry::StrokeLine { line: [a[0], a[1], b[0], b[1]], radius: params.radius, cap };
        self.draw_analytic(&g, &m, color);
        Ok(())
    }

    /// `Device::drawRect` with a fill: the non-AA bounds renderer when the
    /// device rectangle is pixel aligned (within `kDefaultPixelTolerance`),
    /// else the analytic renderer.
    fn fill_rect(&mut self, r: [f32; 4], m: &Mat, color: [f32; 4]) -> Result<(), Unsupported> {
        if r[2] - r[0] <= 0.0 || r[3] - r[1] <= 0.0 {
            return Ok(());
        }
        let rect_stays_rect = (m.kx == 0.0 && m.ky == 0.0) || (m.sx == 0.0 && m.sy == 0.0);
        if rect_stays_rect {
            let a = [m.sx * r[0] + m.kx * r[1] + m.tx, m.ky * r[0] + m.sy * r[1] + m.ty];
            let b = [m.sx * r[2] + m.kx * r[3] + m.tx, m.ky * r[2] + m.sy * r[3] + m.ty];
            let near = |v: f32| (v - libm::roundf(v)).abs() <= 0.0039;
            if near(a[0]) && near(a[1]) && near(b[0]) && near(b[1]) {
                let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
                let polys = tess::to_device(&[corners.to_vec()], m, self.width, self.height);
                self.ops.push(Op::Polygons { polys, color, msaa: false });
                return Ok(());
            }
        }
        self.draw_analytic(&analytic::Geometry::FillRect(r), m, color);
        Ok(())
    }

    fn draw_analytic(&mut self, g: &analytic::Geometry, m: &Mat, color: [f32; 4]) {
        let inst = analytic::instance(g, m);
        self.ops.push(Op::Analytic { inst, color });
    }

    /// The stroke parameters Graphite receives (`StrokeStyle`).
    fn stroke_params(&self) -> (StrokeParams, Cap) {
        let join = match self.state.join {
            LineJoin::Round => -1.0,
            LineJoin::Bevel => 0.0,
            LineJoin::Miter => self.state.miter_limit,
        };
        let cap = match self.state.cap {
            LineCap::Butt => Cap::Butt,
            LineCap::Round => Cap::Round,
            LineCap::Square => Cap::Square,
        };
        (StrokeParams { radius: self.state.line_width / 2.0, join }, cap)
    }

    /// `stroke(path2d)` / the stroke of an `SkPath` (`Device::drawPath`
    /// with a stroke paint).
    pub fn stroke_path(&mut self, p: &SkPath) -> Result<(), Unsupported> {
        if let Some([a, b]) = p.is_line() {
            return self.stroke_line(a, b);
        }
        let m = self.matrix();
        let (params, cap) = self.stroke_params();
        if let Some((r, true)) = p.is_rect() {
            let color = self.paint(self.state.stroke.0);
            let g = analytic::Geometry::StrokeRect { rect: r, radius: params.radius, join: params.join };
            self.draw_analytic(&g, &m, color);
            return Ok(());
        }
        let color = self.paint(self.state.stroke.0);
        let tris = stroke::stroke_triangles(p, &m, params, cap);
        self.ops.push(Op::Triangles { tris, color });
        Ok(())
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The resolved canvas: premultiplied RGBA8, top-down. Graphite
    /// multisamples the render pass only when one of its draws requires MSAA
    /// (the tessellating renderers); otherwise every draw is rasterized once
    /// per pixel at its center.
    pub fn finish(&self) -> Vec<u8> {
        let msaa = self.ops.iter().any(|o| match o {
            Op::Polygons { msaa, .. } => *msaa,
            Op::Triangles { .. } => true,
            Op::Analytic { .. } => false,
        });
        let pattern: &'static [[f64; 2]] = if msaa { &SAMPLES_4X } else { &SAMPLES_1X };
        let mut target = MsaaTarget::new(self.width, self.height, pattern);
        for op in &self.ops {
            match op {
                Op::Polygons { polys, color, .. } => {
                    let mut mask = target.mask();
                    fill_polygons(&mut mask, polys, Rule::NonZero);
                    target.blend(&mask, *color);
                }
                Op::Triangles { tris, color } => {
                    let mut mask = target.mask();
                    super::msaa::fill_triangles(&mut mask, tris);
                    target.blend(&mask, *color);
                }
                Op::Analytic { inst, color } => {
                    let samples = analytic::rasterize(inst, self.width, self.height, pattern);
                    target.blend_coverage(&samples, *color);
                }
            }
        }
        target.resolve()
    }
}
