//! The legacy Canvas2D path raster for platform text glyphs. The Skia-style
//! transform, tessellation, sample positions and N32 resolve remain unchanged.

use super::analytic;
use super::msaa::{MsaaTarget, Rule, SAMPLES_1X, SAMPLES_4X, fill_polygons};
use super::paint::Paint;
use super::skpath::{Mat, P, SkPath};
use super::stroke::{self, Cap, StrokeParams};
use super::tess;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
}

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
    line_width: f32,
    join: LineJoin,
    miter_limit: f32,
    fill_paint: Paint,
    stroke_paint: Paint,
    ctm: [f64; 6],
    m44: [f32; 16],
}
impl Default for State {
    fn default() -> Self {
        Self {
            line_width: 1.0,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            fill_paint: Paint::Solid([0.0, 0.0, 0.0, 1.0]),
            stroke_paint: Paint::Solid([0.0, 0.0, 0.0, 1.0]),
            ctm: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            m44: [
                1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }
}

#[derive(Debug, Clone)]
enum Op {
    Polygons {
        native: Option<pm_text::render::graphite::Geometry>,
        polys: Vec<Vec<P>>,
        color: Paint,
        msaa: bool,
        rule: Rule,
    },
    Triangles {
        native: Option<pm_text::render::graphite::Geometry>,
        tris: Vec<[P; 3]>,
        color: Paint,
    },
    Analytic {
        inst: analytic::Instance,
        color: Paint,
    },
}

pub struct Context {
    native_paths: bool,
    pass_msaa: bool,
    width: u32,
    height: u32,
    ops: Vec<Op>,
    state: State,
}

fn sat_f32(v: f64) -> f32 {
    if v.is_nan() {
        0.0
    } else if v >= f64::from(f32::MAX) {
        f32::MAX
    } else if v <= f64::from(f32::MIN) {
        f32::MIN
    } else {
        v as f32
    }
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

fn m44_concat(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        let r = &b[col * 4..col * 4 + 4];
        for i in 0..4 {
            out[col * 4 + i] =
                a[i] * r[0] + (a[4 + i] * r[1] + (a[8 + i] * r[2] + a[12 + i] * r[3]));
        }
    }
    out
}

impl Context {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            native_paths: false,
            pass_msaa: false,
            width,
            height,
            ops: Vec::new(),
            state: State::default(),
        }
    }

    pub fn retain_native_paths(&mut self) {
        self.native_paths = true;
    }

    pub fn multisample_pass(&mut self) {
        self.pass_msaa = true;
    }

    fn multisampled(&self) -> bool {
        self.pass_msaa || self.ops.iter().any(needs_msaa)
    }

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
        let r = [
            f[0], f[1], 0.0, 0.0, f[2], f[3], 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, f[4], f[5], 0.0, 1.0,
        ];
        if r != [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ] {
            self.state.m44 = m44_concat(&self.state.m44, &r);
        }
    }

    pub fn set_transform(&mut self, t: [f64; 6]) {
        self.state.ctm = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
        self.state.m44 = State::default().m44;
        self.transform(t);
    }

    pub fn set_line_width(&mut self, v: f64) {
        if v.is_finite() && v > 0.0 {
            self.state.line_width = sat_f32(v);
        }
    }

    pub fn set_line_join(&mut self, j: LineJoin) {
        self.state.join = j;
    }

    pub fn set_miter_limit(&mut self, v: f32) {
        if v > 0.0 {
            self.state.miter_limit = v
        }
    }

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

    pub fn set_fill_paint(&mut self, p: Paint) {
        self.state.fill_paint = p;
    }

    pub fn set_stroke_paint(&mut self, p: Paint) {
        self.state.stroke_paint = p;
    }

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
            let native = self
                .native_paths
                .then(|| super::native::fill(p, &m, false, rule == Rule::EvenOdd));
            self.record(Op::Triangles {
                tris,
                color,
                native,
            });
            return Ok(());
        }
        let local = tess::fill_polygons_local(p, &m);
        let polys = tess::to_device(&local, &m);
        self.record(Op::Polygons {
            native: self
                .native_paths
                .then(|| super::native::fill(p, &m, true, rule == Rule::EvenOdd)),
            polys,
            color,
            msaa: true,
            rule,
        });
        Ok(())
    }

    fn fill_rect(&mut self, r: [f32; 4], m: &Mat, color: Paint) -> Result<(), Unsupported> {
        if r[2] - r[0] <= 0.0 || r[3] - r[1] <= 0.0 {
            return Ok(());
        }
        let rect_stays_rect = (m.kx == 0.0 && m.ky == 0.0) || (m.sx == 0.0 && m.sy == 0.0);
        if rect_stays_rect {
            let a = [
                m.sx * r[0] + m.kx * r[1] + m.tx,
                m.ky * r[0] + m.sy * r[1] + m.ty,
            ];
            let b = [
                m.sx * r[2] + m.kx * r[3] + m.tx,
                m.ky * r[2] + m.sy * r[3] + m.ty,
            ];
            let near = |v: f32| (v - libm::roundf(v)).abs() <= 0.0039;
            if near(a[0]) && near(a[1]) && near(b[0]) && near(b[1]) {
                let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
                let polys = tess::to_device(&[corners.to_vec()], m);
                self.record(Op::Polygons {
                    native: None,
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
        let native = self
            .native_paths
            .then(|| super::native::stroke(p, &m, params, cap));
        self.record(Op::Triangles {
            tris,
            color,
            native,
        });
        Ok(())
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn finish_region(&self, rect: [i32; 4]) -> Option<Vec<u8>> {
        if rect[2] <= rect[0] || rect[3] <= rect[1] {
            return None;
        }
        let pattern = if self.multisampled() {
            &SAMPLES_4X[..]
        } else {
            &SAMPLES_1X[..]
        };
        let mut target = MsaaTarget::new(
            (rect[2] - rect[0]) as u32,
            (rect[3] - rect[1]) as u32,
            pattern,
        );
        let viewport = tess::Viewport::of(self.width, self.height);
        for op in &self.ops {
            render_op_region(&mut target, op, viewport, [rect[0], rect[1]]);
        }
        Some(target.resolve())
    }

    pub fn graphite_region(&self, rect: [i32; 4]) -> Option<pm_text::render::graphite::Raster> {
        use pm_text::render::graphite::{Draw, Geometry, Raster};
        let mut draws = Vec::new();
        for op in &self.ops {
            let (geometry, color) = match op {
                Op::Analytic { inst, color } => {
                    let m = inst.m.map(|v| [v[0], v[1], v[2], 0.0]);
                    (
                        Geometry::Analytic([
                            inst.x_radii_or_flags,
                            inst.radii_or_quad_xs,
                            inst.ltrb_or_quad_ys,
                            inst.center,
                            m[0],
                            m[1],
                            m[2],
                        ]),
                        color,
                    )
                }
                Op::Triangles {
                    native: Some(geometry),
                    color,
                    ..
                }
                | Op::Polygons {
                    native: Some(geometry),
                    color,
                    ..
                } => (geometry.clone(), color),
                Op::Triangles { tris, color, .. } => (
                    Geometry::Triangles(tris.iter().flatten().copied().collect()),
                    color,
                ),
                Op::Polygons {
                    polys, color, rule, ..
                } => {
                    let mut vertices = Vec::new();
                    for p in polys {
                        for i in 2..p.len() {
                            vertices.extend([p[0], p[i - 1], p[i]]);
                        }
                    }
                    (
                        Geometry::Winding {
                            vertices,
                            even_odd: *rule == Rule::EvenOdd,
                        },
                        color,
                    )
                }
            };
            let Paint::Solid(color) = color;
            draws.push(Draw {
                geometry,
                color: *color,
            });
        }
        Some(Raster {
            width: self.width,
            height: self.height,
            crop: rect.map(|v| v as u32),
            multisampled: self.multisampled(),
            draws,
        })
    }

    fn fill_paint(&self) -> Paint {
        self.state.fill_paint.clone()
    }
    fn stroke_paint(&self) -> Paint {
        self.state.stroke_paint.clone()
    }
    fn record(&mut self, op: Op) {
        self.ops.push(op);
    }
    fn stroke_params(&self) -> (StrokeParams, Cap) {
        let join = match self.state.join {
            LineJoin::Round => -1.0,
            LineJoin::Miter => self.state.miter_limit,
        };
        (
            StrokeParams {
                radius: self.state.line_width / 2.0,
                join,
            },
            Cap::Butt,
        )
    }
}

fn needs_msaa(op: &Op) -> bool {
    match op {
        Op::Polygons { msaa, .. } => *msaa,
        Op::Triangles { .. } => true,
        Op::Analytic { .. } => false,
    }
}

fn render_op_region(target: &mut MsaaTarget, op: &Op, viewport: tess::Viewport, origin: [i32; 2]) {
    use super::msaa::{fill_triangles, unorm8};
    let mut mask = target.mask();
    let mut coverage = None;
    let color = match op {
        Op::Polygons {
            polys, color, rule, ..
        } => {
            let mut mapped = viewport.map_polygons(polys);
            for p in mapped.iter_mut().flatten() {
                p[0] -= f64::from(origin[0]);
                p[1] -= f64::from(origin[1]);
            }
            fill_polygons(&mut mask, &mapped, *rule);
            color
        }
        Op::Triangles { tris, color, .. } => {
            let mut mapped = viewport.map_triangles(tris);
            for p in mapped.iter_mut().flatten() {
                p[0] -= origin[0] as f32;
                p[1] -= origin[1] as f32;
            }
            fill_triangles(&mut mask, &mapped);
            color
        }
        Op::Analytic { inst, color } => {
            coverage = Some(analytic::rasterize_region(
                inst,
                target.width,
                target.height,
                target.pattern,
                origin,
            ));
            color
        }
    };
    let Paint::Solid(color) = color;
    if coverage.is_none() {
        target.blend(&mask, *color);
        return;
    }
    for (i, d) in target.samples.iter_mut().enumerate() {
        let cov = coverage.as_ref().unwrap().coverage[i].unwrap_or(0.0);
        if cov == 0.0 {
            continue;
        }
        let color = color.map(|v| v * cov);
        let inv = 1.0 - color[3];
        for c in 0..4 {
            d[c] = unorm8(color[c] + f32::from(d[c]) / 255.0 * inv);
        }
    }
}
