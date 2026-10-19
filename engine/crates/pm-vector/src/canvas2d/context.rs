//! A Canvas2D context as Chromium 152 draws it on Skia Graphite (Metal):
//! Blink's state and paths, recorded paint operations replayed by a
//! Graphite device into a 4× multisampled RGBA8 target, resolved to the
//! bitmap the legacy compositor uploads.

use super::analytic;
use super::blink::{CanvasPath, Drawn, sat_f32};
use super::msaa::{MsaaTarget, Rule, SAMPLES_1X, SAMPLES_4X, fill_polygons};
use super::paint::Paint;
use super::skpath::{Mat, P, SkPath};
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
    fill_paint: Option<Paint>,
    stroke_paint: Option<Paint>,
    clips: Vec<Clip>,
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
            fill_paint: None,
            stroke_paint: None,
            clips: Vec::new(),
            ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            m44: [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        }
    }
}

type ClipShape = (SkPath, Mat, Rule, Option<analytic::Geometry>);
type Clip = Vec<ClipShape>;

/// One recorded draw, rasterized when the canvas is finished.
#[derive(Debug, Clone)]
enum Op {
    /// Closed polygons in device coordinates with the nonzero rule: the
    /// tessellated fill renderers (which require MSAA) or the non-AA bounds
    /// renderer (which does not). The pass's viewport maps them to window
    /// coordinates when they are rasterized.
    Polygons {
        polys: Vec<Vec<P>>,
        color: Paint,
        msaa: bool,
        rule: Rule,
    },
    /// The union of triangles in device coordinates (the tessellated fill
    /// and stroke renderers, MSAA).
    Triangles {
        tris: Vec<[P; 3]>,
        color: Paint,
    },
    /// One `AnalyticRRectRenderStep` instance (no MSAA requirement).
    Analytic {
        inst: analytic::Instance,
        color: Paint,
    },
    Clip {
        op: Box<Op>,
        clips: Vec<Clip>,
    },
    Bitmap {
        rgba: Vec<u8>,
    },
    Layer {
        ops: Vec<Op>,
        alpha: f32,
        blur: [f64; 2],
        linear: bool,
    },
}

/// A Canvas2D context on a `width` × `height` canvas.
pub struct Context {
    width: u32,
    height: u32,
    ops: Vec<Op>,
    state: State,
    stack: Vec<State>,
    layers: Vec<usize>,
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
            layers: Vec::new(),
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

    pub fn set_transform(&mut self, t: [f64; 6]) {
        self.state.ctm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        self.state.m44 = State::default().m44;
        self.transform(t);
    }
    pub fn set_fill_style(&mut self, s: &str) {
        if crate::render::color::parse(s).is_some() {
            self.state.fill_paint = None;
        }
        self.state.fill.set(s);
    }

    pub fn set_stroke_style(&mut self, s: &str) {
        if crate::render::color::parse(s).is_some() {
            self.state.stroke_paint = None;
        }
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
    fn paint(&self, c: CanvasColor) -> Paint {
        Paint::Solid(c.with_alpha_scale(self.state.global_alpha as f32).premultiplied())
    }
    fn fill_paint(&self) -> Paint {
        self.state
            .fill_paint
            .clone()
            .map_or_else(|| self.paint(self.state.fill.0), |p| p.alpha(self.state.global_alpha as f32))
    }
    fn stroke_paint(&self) -> Paint {
        self.state
            .stroke_paint
            .clone()
            .map_or_else(|| self.paint(self.state.stroke.0), |p| p.alpha(self.state.global_alpha as f32))
    }
    pub fn set_fill_paint(&mut self, p: Paint) {
        self.state.fill_paint = Some(p)
    }
    pub fn set_stroke_paint(&mut self, p: Paint) {
        self.state.stroke_paint = Some(p)
    }
    pub fn set_miter_limit(&mut self, v: f32) {
        if v > 0.0 {
            self.state.miter_limit = v
        }
    }
    pub fn clip_path(&mut self, p: SkPath, rule: Rule) {
        self.state.clips.push(vec![(p, self.matrix(), rule, None)]);
    }
    pub fn clip_geometry(&mut self, p: SkPath, rule: Rule, g: Option<analytic::Geometry>) {
        self.state.clips.push(vec![(p, self.matrix(), rule, g)]);
    }
    pub fn clip_shapes(&mut self, shapes: Vec<ClipShape>) {
        self.state.clips.push(shapes);
    }
    pub fn stroke_geometry(&mut self, g: analytic::Geometry) {
        let m = self.matrix();
        let color = self.stroke_paint();
        self.draw_analytic(&g, &m, color);
    }
    pub fn draw_bitmap(&mut self, rgba: Vec<u8>) {
        assert_eq!(rgba.len(), (self.width * self.height * 4) as usize);
        self.record(Op::Bitmap { rgba });
    }
    pub fn begin_layer(&mut self) {
        self.layers.push(self.ops.len());
    }
    pub fn end_layer(&mut self, alpha: f32, blur: [f64; 2], linear: bool) {
        if let Some(start) = self.layers.pop() {
            let ops = self.ops.drain(start..).collect();
            self.ops.push(Op::Layer { ops, alpha, blur, linear });
        }
    }
    fn record(&mut self, op: Op) {
        if self.state.clips.is_empty() {
            self.ops.push(op)
        } else {
            self.ops.push(Op::Clip {
                op: Box::new(op),
                clips: self.state.clips.clone(),
            })
        }
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
        self.fill_path_rule(p, Rule::NonZero)
    }
    pub fn fill_path_rule(&mut self, p: &SkPath, rule: Rule) -> Result<(), Unsupported> {
        if p.is_line().is_some() {
            return Ok(());
        }
        let m = self.matrix();
        let color = self.fill_paint();
        if let Some((r, _)) = p.is_rect() {
            return self.fill_rect(r, &m, color);
        }
        if p.is_convex() {
            let tris = tess::convex_wedge_triangles(p, &m);
            self.record(Op::Triangles { tris, color });
            return Ok(());
        }
        let local = tess::fill_polygons_local(p, &m);
        let polys = tess::to_device(&local, &m);
        self.record(Op::Polygons {
            polys,
            color,
            msaa: true,
            rule,
        });
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
        let color = self.stroke_paint();
        let g = analytic::Geometry::StrokeLine {
            line: [a[0], a[1], b[0], b[1]],
            radius: params.radius,
            cap,
        };
        self.draw_analytic(&g, &m, color);
        Ok(())
    }

    /// `Device::drawRect` with a fill: the non-AA bounds renderer when the
    /// device rectangle is pixel aligned (within `kDefaultPixelTolerance`),
    /// else the analytic renderer.
    fn fill_rect(&mut self, r: [f32; 4], m: &Mat, color: Paint) -> Result<(), Unsupported> {
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
                let polys = tess::to_device(&[corners.to_vec()], m);
                self.record(Op::Polygons {
                    polys,
                    color,
                    msaa: false,
                    rule: Rule::NonZero,
                });
                return Ok(());
            }
        }
        self.draw_analytic(&analytic::Geometry::FillRect(r), m, color);
        Ok(())
    }

    fn draw_analytic(&mut self, g: &analytic::Geometry, m: &Mat, color: Paint) {
        let inst = analytic::instance(g, m);
        self.record(Op::Analytic { inst, color });
    }

    pub fn fill_geometry(&mut self, g: analytic::Geometry) {
        let m = self.matrix();
        self.draw_analytic(&g, &m, self.fill_paint());
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
        (
            StrokeParams {
                radius: self.state.line_width / 2.0,
                join,
            },
            cap,
        )
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
            let color = self.stroke_paint();
            let g = analytic::Geometry::StrokeRect {
                rect: r,
                radius: params.radius,
                join: params.join,
            };
            self.draw_analytic(&g, &m, color);
            return Ok(());
        }
        let color = self.stroke_paint();
        let tris = stroke::stroke_triangles(p, &m, params, cap);
        self.record(Op::Triangles { tris, color });
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
        render_ops(self.width, self.height, &self.ops)
    }
}

fn needs_msaa(o: &Op) -> bool {
    match o {
        Op::Polygons { msaa, .. } => *msaa,
        Op::Triangles { .. } => true,
        Op::Clip { op, clips } => needs_msaa(op) || clips.iter().flatten().any(|c| c.3.is_none()),
        Op::Analytic { .. } | Op::Layer { .. } | Op::Bitmap { .. } => false,
    }
}
fn render_ops(width: u32, height: u32, ops: &[Op]) -> Vec<u8> {
    let pattern = if ops.iter().any(needs_msaa) {
        &SAMPLES_4X[..]
    } else {
        &SAMPLES_1X[..]
    };
    let mut target = MsaaTarget::new(width, height, pattern);
    for op in ops {
        render_op(&mut target, op, None)
    }
    target.resolve()
}
fn render_op(target: &mut MsaaTarget, op: &Op, clip: Option<&[f32]>) {
    use super::msaa::{fill_triangles, unorm8};
    let mut mask = target.mask();
    let mut coverage = None;
    let viewport = tess::Viewport::of(target.width, target.height);
    let paint = match op {
        Op::Polygons { polys, color, rule, .. } => {
            fill_polygons(&mut mask, &viewport.map_polygons(polys), *rule);
            color
        }
        Op::Triangles { tris, color } => {
            fill_triangles(&mut mask, &viewport.map_triangles(tris));
            color
        }
        Op::Analytic { inst, color } => {
            coverage = Some(analytic::rasterize(inst, target.width, target.height, target.pattern));
            color
        }
        Op::Clip { op, clips } => {
            let mut combined = vec![1.0; target.samples.len()];
            for shapes in clips {
                let mut union = vec![0.0; target.samples.len()];
                for (p, m, rule, geometry) in shapes {
                    let local = tess::fill_polygons_local(p, m);
                    let polys = viewport.map_polygons(&tess::to_device(&local, m));
                    if let Some(g) = geometry {
                        let c = analytic::rasterize(&analytic::instance(g, m), target.width, target.height, target.pattern);
                        for (a, b) in union.iter_mut().zip(c.coverage) {
                            let b = b.unwrap_or(0.0);
                            *a += b * (1.0 - *a);
                        }
                    } else {
                        let mut c = target.mask();
                        fill_polygons(&mut c, &polys, *rule);
                        for (i, a) in union.iter_mut().enumerate() {
                            let b = f32::from(u8::from(c.bits[i / target.pattern.len()] & (1 << (i % target.pattern.len())) != 0));
                            *a += b * (1.0 - *a);
                        }
                    }
                }
                for (a, b) in combined.iter_mut().zip(union) {
                    *a *= b;
                }
            }
            if let Some(c) = clip {
                for (a, b) in combined.iter_mut().zip(c) {
                    *a *= *b
                }
            }
            render_op(target, op, Some(&combined));
            return;
        }
        Op::Bitmap { rgba } => {
            for (i, d) in target.samples.iter_mut().enumerate() {
                let p = i / target.pattern.len();
                let alpha = clip.map_or(1.0, |c| c[i]);
                let s = &rgba[p * 4..p * 4 + 4];
                let inv = 1.0 - f32::from(s[3]) / 255.0 * alpha;
                for c in 0..4 {
                    d[c] = unorm8(f32::from(s[c]) / 255.0 * alpha + f32::from(d[c]) / 255.0 * inv);
                }
            }
            return;
        }
        Op::Layer { ops, alpha, blur, linear } => {
            let mut pixels = render_ops(target.width, target.height, ops);
            if blur[0] > 0.0 || blur[1] > 0.0 {
                if *linear {
                    linear_rgb(&mut pixels, false);
                }
                pixels = gaussian(&pixels, target.width, target.height, *blur);
                if *linear {
                    linear_rgb(&mut pixels, true);
                }
            }
            for (i, d) in target.samples.iter_mut().enumerate() {
                let p = i / target.pattern.len();
                let alpha = alpha * clip.map_or(1.0, |c| c[i]);
                let s = &pixels[p * 4..p * 4 + 4];
                let inv = 1.0 - f32::from(s[3]) / 255.0 * alpha;
                for c in 0..4 {
                    d[c] = unorm8(f32::from(s[c]) / 255.0 * alpha + f32::from(d[c]) / 255.0 * inv)
                }
            }
            return;
        }
    };
    for (i, d) in target.samples.iter_mut().enumerate() {
        let p = i / target.pattern.len();
        let s = i % target.pattern.len();
        let cov = if let Some(c) = &coverage {
            c.coverage[i].unwrap_or(0.0)
        } else if mask.bits[p] & (1 << s) != 0 {
            1.0
        } else {
            0.0
        };
        let cov = cov * clip.map_or(1.0, |c| c[i]);
        if cov == 0.0 {
            continue;
        }
        let color = paint
            .at((p % target.width as usize) as f32 + 0.5, (p / target.width as usize) as f32 + 0.5)
            .map(|v| v * cov);
        let inv = 1.0 - color[3];
        for c in 0..4 {
            d[c] = unorm8(color[c] + f32::from(d[c]) / 255.0 * inv)
        }
    }
}
fn gaussian(pixels: &[u8], w: u32, h: u32, sigma: [f64; 2]) -> Vec<u8> {
    let mut data = pixels.to_vec();
    for (axis, sigma) in sigma.into_iter().enumerate() {
        if sigma <= 0.0 {
            continue;
        }
        let r = (sigma * 3.0).ceil() as i32;
        let mut kernel: Vec<f64> = (-r..=r).map(|i| libm::exp(-0.5 * (f64::from(i) / sigma).powi(2))).collect();
        let sum: f64 = kernel.iter().sum();
        for k in &mut kernel {
            *k /= sum
        }
        let mut out = vec![0; data.len()];
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                for c in 0..4 {
                    let mut v = 0.0;
                    for (i, k) in (-r..=r).zip(&kernel) {
                        let (xx, yy) = if axis == 0 { (x + i, y) } else { (x, y + i) };
                        if xx >= 0 && xx < w as i32 && yy >= 0 && yy < h as i32 {
                            v += f64::from(data[(yy as usize * w as usize + xx as usize) * 4 + c]) * k;
                        }
                    }
                    out[(y as usize * w as usize + x as usize) * 4 + c] = (v + 0.5) as u8;
                }
            }
        }
        data = out;
    }
    data
}

fn linear_rgb(pixels: &mut [u8], encode: bool) {
    for p in pixels.chunks_exact_mut(4) {
        let a = f64::from(p[3]) / 255.0;
        if a == 0.0 {
            p.fill(0);
            continue;
        }
        for c in p.iter_mut().take(3) {
            let v = (f64::from(*c) / 255.0 / a).clamp(0.0, 1.0);
            let v = if encode {
                if v <= 0.0031308 {
                    v * 12.92
                } else {
                    1.055 * libm::pow(v, 1.0 / 2.4) - 0.055
                }
            } else if v <= 0.04045 {
                v / 12.92
            } else {
                libm::pow((v + 0.055) / 1.055, 2.4)
            };
            *c = (v * a * 255.0 + 0.5) as u8;
        }
    }
}
