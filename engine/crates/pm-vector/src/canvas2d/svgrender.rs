//! Graphite's `Device` replaying the SVG paint record (`blinksvg::Op`) into
//! the Canvas2D target (pinned Skia `b6d1062`): the `SkCanvas` matrix and
//! layer stack, `ClipStack` (elements pre-transformed to device space,
//! `visitClipStackForDraw`: draws contained by a clip ignore it, rect and
//! rounded-rect clips intersect rect draws geometrically, one rect or
//! circular-corner rrect becomes the analytic clip shader, the rest are
//! depth-only clip draws in the MSAA pass), `chooseRenderer`, and the
//! per-sample rasterization (`msaa`, `analytic`, `tess`, `stroke`).
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::analytic::{self, Geometry as AnalyticGeometry};
use super::blinksvg::{Flags, ImageFilter, Op, StrokeFlags};
use super::gradient::{self, Shader};
use super::m44::M44;
use super::msaa::{MsaaTarget, Rule, SAMPLES_1X, SAMPLES_4X, SampleMask, fill_polygons, fill_triangles, unorm8};
use super::skpath::{Mat, P, SkPath, Verb};
use super::skrrect::SkRRect;
use super::stroke::{self, Cap, StrokeParams};
use super::tess;
use crate::render::svg::SvgError;
use std::sync::Arc;

/// `Shape::kDefaultPixelTolerance`.
pub const PIXEL_TOLERANCE: f32 = 0.0039;

// ---------------------------------------------------------------------------
// Graphite's Transform

/// `skgpu::graphite::Transform::Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum TransformType {
    Identity,
    SimpleRectStaysRect,
    RectStaysRect,
    Affine,
    Invalid,
}

/// `skgpu::graphite::Transform` (the affine case).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub m: Mat,
    pub inv: Mat,
    pub ty: TransformType,
    pub min_scale: f32,
    pub max_scale: f32,
}

fn compute_svd(m00: f32, m01: f32, m10: f32, m11: f32) -> (f32, f32) {
    let s1 = m00 * m00 + m01 * m01 + m10 * m10 + m11 * m11;
    let e = m00 * m00 + m01 * m01 - m10 * m10 - m11 * m11;
    let f = m00 * m10 + m01 * m11;
    let s2 = (e * e + 4.0 * f * f).sqrt();
    ((0.5 * (s1 - s2)).sqrt(), (0.5 * (s1 + s2)).sqrt())
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        m: Mat::IDENTITY,
        inv: Mat::IDENTITY,
        ty: TransformType::Identity,
        min_scale: 1.0,
        max_scale: 1.0,
    };

    /// `Transform::Transform(const SkM44&)` for an affine matrix.
    pub fn new(m: Mat) -> Self {
        let (sx, sy, kx, ky, tx, ty) = (m.sx, m.sy, m.kx, m.ky, m.tx, m.ty);
        if kx == 0.0 && ky == 0.0 {
            if sx == 0.0 || sy == 0.0 {
                return Self { m, inv: Mat::IDENTITY, ty: TransformType::Invalid, min_scale: 0.0, max_scale: 0.0 };
            }
            if sx == 1.0 && sy == 1.0 && tx == 0.0 && ty == 0.0 {
                return Self::IDENTITY;
            }
            let ix = 1.0 / sx;
            let iy = 1.0 / sy;
            let ty_ = if sx > 0.0 && sy > 0.0 { TransformType::SimpleRectStaysRect } else { TransformType::RectStaysRect };
            let (min, max) = sort_scale(sx, sy);
            return Self {
                m,
                inv: Mat { sx: ix, kx: 0.0, tx: -ix * tx, ky: 0.0, sy: iy, ty: -iy * ty },
                ty: ty_,
                min_scale: min,
                max_scale: max,
            };
        }
        if sx == 0.0 && sy == 0.0 {
            if kx == 0.0 || ky == 0.0 {
                return Self { m, inv: Mat::IDENTITY, ty: TransformType::Invalid, min_scale: 0.0, max_scale: 0.0 };
            }
            let ix = 1.0 / kx;
            let iy = 1.0 / ky;
            let (min, max) = sort_scale(kx, ky);
            return Self {
                m,
                inv: Mat { sx: 0.0, kx: iy, tx: -iy * ty, ky: ix, sy: 0.0, ty: -ix * tx },
                ty: TransformType::RectStaysRect,
                min_scale: min,
                max_scale: max,
            };
        }
        // SkInvert2x2Matrix on the column-major upper 2x2 {sx, ky, kx, sy}.
        let det = sx * sy - kx * ky;
        if det == 0.0 || !det.is_finite() {
            return Self { m, inv: Mat::IDENTITY, ty: TransformType::Invalid, min_scale: 0.0, max_scale: 0.0 };
        }
        let inv_det = 1.0 / det;
        let inv_upper = [sy * inv_det, -ky * inv_det, -kx * inv_det, sx * inv_det];
        let (min, max) = compute_svd(sx, kx, ky, sy);
        Self {
            m,
            inv: Mat {
                sx: inv_upper[0],
                kx: inv_upper[2],
                tx: -inv_upper[0] * tx - inv_upper[2] * ty,
                ky: inv_upper[1],
                sy: inv_upper[3],
                ty: -inv_upper[1] * tx - inv_upper[3] * ty,
            },
            ty: TransformType::Affine,
            min_scale: min,
            max_scale: max,
        }
    }

    pub fn valid(&self) -> bool {
        self.ty != TransformType::Invalid
    }

    /// `localAARadius`.
    pub fn local_aa_radius(&self) -> f32 {
        let r = 1.0 / self.min_scale;
        if r.is_finite() { r } else { f32::INFINITY }
    }

    /// `mapRect` (`Transform::mapRect` through `map_rect`).
    pub fn map_rect(&self, r: [f32; 4]) -> [f32; 4] {
        match self.ty {
            TransformType::Identity => r,
            TransformType::SimpleRectStaysRect | TransformType::RectStaysRect if self.m.kx == 0.0 && self.m.ky == 0.0 => {
                sorted([r[0] * self.m.sx + self.m.tx, r[1] * self.m.sy + self.m.ty, r[2] * self.m.sx + self.m.tx, r[3] * self.m.sy + self.m.ty])
            }
            TransformType::RectStaysRect => sorted([
                r[1] * self.m.kx + self.m.tx,
                r[0] * self.m.ky + self.m.ty,
                r[3] * self.m.kx + self.m.tx,
                r[2] * self.m.ky + self.m.ty,
            ]),
            _ => {
                let pts = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]].map(|p| self.m.map(p));
                let mut b = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];
                for p in pts {
                    b[0] = b[0].min(p[0]);
                    b[1] = b[1].min(p[1]);
                    b[2] = b[2].max(p[0]);
                    b[3] = b[3].max(p[1]);
                }
                b
            }
        }
    }

    pub fn inverse_map_rect(&self, r: [f32; 4]) -> [f32; 4] {
        Transform::new(self.inv).map_rect(r)
    }
}

fn sort_scale(sx: f32, sy: f32) -> (f32, f32) {
    let (a, b) = (sx.abs(), sy.abs());
    if a > b { (b, a) } else { (a, b) }
}

#[inline]
fn sorted(r: [f32; 4]) -> [f32; 4] {
    [r[0].min(r[2]), r[1].min(r[3]), r[0].max(r[2]), r[1].max(r[3])]
}

#[inline]
fn rect_union(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

fn rect_intersect(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].min(b[2]), a[3].min(b[3])]
}

#[inline]
fn rect_empty(r: [f32; 4]) -> bool {
    !(r[2] > r[0] && r[3] > r[1])
}

/// `Rect::contains`: `all(vals <= rect.vals)`, true for an inverted `b`.
#[inline]
fn rect_contains(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] <= b[0] && a[1] <= b[1] && a[2] >= b[2] && a[3] >= b[3]
}

/// `Rect::intersects`: strict, false for an inverted rect.
#[inline]
fn rect_intersects(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0] < b[2] && a[1] < b[3] && b[0] < a[2] && b[1] < a[3]
}

const INFINITE_INVERTED: [f32; 4] = [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY, f32::NEG_INFINITY];

#[inline]
fn rect_round(r: [f32; 4]) -> [f32; 4] {
    r.map(libm::roundf)
}

#[inline]
fn rect_round_out(r: [f32; 4]) -> [f32; 4] {
    [libm::floorf(r[0]), libm::floorf(r[1]), libm::ceilf(r[2]), libm::ceilf(r[3])]
}

fn rect_nearly_equals(a: [f32; 4], b: [f32; 4], tol: f32) -> bool {
    (0..4).all(|i| (a[i] - b[i]).abs() <= tol)
}

// ---------------------------------------------------------------------------
// Shapes

/// `skgpu::graphite::Shape` (lines, rects, rrects, paths, the inverted
/// empty flood fill).
#[derive(Debug, Clone)]
pub enum Shape {
    Empty,
    Line([f32; 4]),
    Rect([f32; 4]),
    RRect(SkRRect),
    Path(SkPath, Rule),
}

impl Shape {
    pub fn bounds(&self) -> [f32; 4] {
        match self {
            Shape::Empty => [0.0; 4],
            Shape::Line(l) => sorted(*l),
            Shape::Rect(r) => *r,
            Shape::RRect(rr) => rr.rect,
            Shape::Path(p, _) => p.bounds().unwrap_or([0.0; 4]),
        }
    }

    /// `Shape::asPath` for the tessellated renderers.
    pub fn as_path(&self) -> (SkPath, Rule) {
        match self {
            Shape::Path(p, r) => (p.clone(), *r),
            Shape::Rect(r) => (super::skrrect::rect_path(*r, true, 0), Rule::NonZero),
            Shape::RRect(rr) => (rr.to_path(true, 0), Rule::NonZero),
            Shape::Line(l) => {
                let mut p = SkPath::new();
                p.move_to([l[0], l[1]]);
                p.line_to([l[2], l[3]]);
                (p, Rule::NonZero)
            }
            Shape::Empty => (SkPath::new(), Rule::NonZero),
        }
    }

    fn convex(&self) -> bool {
        match self {
            Shape::Path(p, _) => p.is_convex(),
            Shape::Empty => false,
            _ => true,
        }
    }

    /// `Shape::conservativeContains(rect)`.
    fn conservative_contains(&self, r: [f32; 4]) -> bool {
        match self {
            Shape::Rect(s) => rect_contains(*s, r),
            Shape::RRect(rr) => rrect_contains_rect(rr, r),
            _ => false,
        }
    }
}

/// `SkRRect::contains(rect)`.
fn rrect_contains_rect(rr: &SkRRect, r: [f32; 4]) -> bool {
    if !rect_contains(rr.rect, r) {
        return false;
    }
    if rr.is_rect() {
        return true;
    }
    let inside = |x: f32, y: f32| -> bool {
        let b = rr.rect;
        // Which corner's ellipse does the point fall in, if any.
        let corners = [
            ([b[0], b[1]], rr.radii[0], [1.0, 1.0]),
            ([b[2], b[1]], rr.radii[1], [-1.0, 1.0]),
            ([b[2], b[3]], rr.radii[2], [-1.0, -1.0]),
            ([b[0], b[3]], rr.radii[3], [1.0, -1.0]),
        ];
        for (corner, radii, sign) in corners {
            let cx = corner[0] + sign[0] * radii[0];
            let cy = corner[1] + sign[1] * radii[1];
            let in_x = if sign[0] > 0.0 { x < cx } else { x > cx };
            let in_y = if sign[1] > 0.0 { y < cy } else { y > cy };
            if in_x && in_y && radii[0] > 0.0 && radii[1] > 0.0 {
                let dx = (x - cx) / radii[0];
                let dy = (y - cy) / radii[1];
                return dx * dx + dy * dy <= 1.0;
            }
        }
        true
    };
    inside(r[0], r[1]) && inside(r[2], r[1]) && inside(r[2], r[3]) && inside(r[0], r[3])
}

// ---------------------------------------------------------------------------
// Clip stack

/// `AnalyticClip` as `can_apply_analytic_clip` builds it (the 2×2 inverse
/// `xform`, the translated rect, four circular radii).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalyticClip {
    /// `fBounds`: the device-space rect of the clip.
    pub rect: [f32; 4],
    /// `fRadius`: the circular corner radius (`kRadiusMin` = 0.5 when the
    /// corners are square or collapsed to square).
    pub radius: f32,
    /// `fEdgeFlags`: which edges (L, T, R, B as bits 0..3) meet circular
    /// corners.
    pub edge_flags: u32,
    /// `fInverted`: true for intersect clips (the depth-draw convention).
    pub inverted: bool,
}

const EDGE_LEFT: u32 = 0b0001;
const EDGE_TOP: u32 = 0b0010;
const EDGE_RIGHT: u32 = 0b0100;
const EDGE_BOTTOM: u32 = 0b1000;
const EDGE_ALL: u32 = 0b1111;
/// `kRadiusMin`.
const RADIUS_MIN: f32 = 0.5;

#[derive(Debug, Clone)]
struct RawElement {
    shape: Shape,
    local_to_device: Transform,
    intersect: bool,
    outer: [f32; 4],
    inner: [f32; 4],
    invalid: bool,
    save_depth: usize,
    /// The pending depth-only clip draw's painter's order (`fOrder`), set
    /// at the first draw that needs the element drawn into the depth
    /// attachment; `None` until then.
    order: Option<u32>,
    /// The highest Z of the draws this element clips (`fMaxZ`).
    max_z: u32,
    /// The joined, snapped bounds of those draws (`fUsageBounds`).
    usage: [f32; 4],
}

/// Draws recorded in a device, with their painter's order bookkeeping
/// (`Device::fColorDepthBoundsManager`, a naive exact manager: the SVG
/// records are small).
#[derive(Debug, Default)]
pub struct BoundsManager {
    rects: Vec<([f32; 4], u32)>,
}

/// `CompressedPaintersOrder::First()`.
const FIRST_ORDER: u32 = 1;

impl BoundsManager {
    /// `getMostRecentDraw`: the highest order of a recorded draw whose
    /// bounds intersect `bounds` (`First` when none does).
    fn most_recent(&self, bounds: [f32; 4]) -> u32 {
        self.rects
            .iter()
            .filter(|(r, _)| rect_intersects(*r, bounds))
            .map(|(_, o)| *o)
            .max()
            .unwrap_or(FIRST_ORDER)
    }
    fn record(&mut self, bounds: [f32; 4], order: u32) {
        self.rects.push((bounds, order));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipState {
    Empty,
    WideOpen,
    DeviceRect,
    DeviceRRect,
    Complex,
}

#[derive(Debug, Clone)]
struct SaveRecord {
    starting_element_index: usize,
    oldest_valid_index: usize,
    state: ClipState,
    outer: [f32; 4],
    inner: [f32; 4],
    intersect: bool,
    deferred_saves: usize,
}

/// A simplified-but-faithful `ClipStack`.
struct ClipStack {
    device_bounds: [f32; 4],
    elements: Vec<RawElement>,
    saves: Vec<SaveRecord>,
    /// Depth-only clip draws produced when elements leave the stack
    /// (`RawElement::drawClip`), taken by the device when recorded.
    clip_draws: Vec<Draw>,
    /// Classification only: clip draws carry no geometry.
    classify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Influence {
    ClipsOut,
    Replaces,
    None,
    Complex,
}

/// `TransformedShape` for the simplify tests.
struct TShape<'a> {
    shape: &'a Shape,
    xf: &'a Transform,
    outer: [f32; 4],
    inner: [f32; 4],
    intersect: bool,
    bounds_only: bool,
}

impl TShape<'_> {
    fn intersects(&self, o: &TShape) -> bool {
        if !rect_intersects(self.outer, o.outer) {
            return false;
        }
        if self.xf.ty <= TransformType::RectStaysRect && o.xf.ty <= TransformType::RectStaysRect {
            return true;
        }
        if self.xf == o.xf {
            return rect_intersects(self.shape.bounds(), o.shape.bounds());
        }
        // Oriented bounding boxes: conservative (assume intersection).
        true
    }

    fn contains(&self, o: &TShape) -> bool {
        if rect_contains(self.inner, o.outer) {
            return true;
        }
        if self.bounds_only || !rect_contains(self.outer, o.outer) {
            return false;
        }
        if self.xf == o.xf {
            if let (Shape::RRect(a), Shape::RRect(b)) = (self.shape, o.shape) {
                return rrect_contains_rect(a, b.rect) && conservative_rrect_contains(a, b);
            }
            return self.shape.conservative_contains(o.shape.bounds());
        }
        if self.xf.ty <= TransformType::RectStaysRect && o.xf.ty <= TransformType::RectStaysRect {
            let local = self.xf.inverse_map_rect(o.xf.map_rect(o.shape.bounds()));
            return self.shape.conservative_contains(local);
        }
        if self.shape.convex() {
            let b = o.shape.bounds();
            let pts = [[b[0], b[1]], [b[2], b[1]], [b[2], b[3]], [b[0], b[3]]];
            return pts.iter().all(|p| {
                let d = o.xf.m.map(*p);
                let l = self.xf.inv.map(d);
                match self.shape {
                    Shape::Rect(r) => l[0] >= r[0] && l[0] <= r[2] && l[1] >= r[1] && l[1] <= r[3],
                    Shape::RRect(rr) => rrect_contains_rect(rr, [l[0], l[1], l[0], l[1]]),
                    _ => false,
                }
            });
        }
        false
    }
}

/// `SkRRectPriv::ConservativeIntersect(a, b) == b`: `a` contains `b`.
fn conservative_rrect_contains(a: &SkRRect, b: &SkRRect) -> bool {
    rect_contains(a.rect, b.rect) && (a.is_rect() || rrect_contains_rect(a, b.rect))
}

fn simplify(a: &TShape, b: &TShape) -> Influence {
    match (a.intersect, b.intersect) {
        (true, true) => {
            if !a.intersects(b) {
                Influence::ClipsOut
            } else if b.contains(a) {
                Influence::Replaces
            } else if a.contains(b) {
                Influence::None
            } else {
                Influence::Complex
            }
        }
        (true, false) => {
            if !a.intersects(b) {
                Influence::Replaces
            } else if b.contains(a) {
                Influence::ClipsOut
            } else {
                Influence::Complex
            }
        }
        (false, true) => {
            if !b.intersects(a) {
                Influence::None
            } else if a.contains(b) {
                Influence::ClipsOut
            } else {
                Influence::Complex
            }
        }
        (false, false) => {
            if a.contains(b) {
                Influence::Replaces
            } else if b.contains(a) {
                Influence::None
            } else {
                Influence::Complex
            }
        }
    }
}

/// `snap_scissor`: outset by 3, rounded out to a 4 px grid, within bounds.
fn snap_scissor(a: [f32; 4], device: [f32; 4]) -> [f32; 4] {
    let o = [a[0] - 3.0, a[1] - 3.0, a[2] + 3.0, a[3] + 3.0];
    let r = [
        libm::floorf(o[0] * 0.25) * 4.0,
        libm::floorf(o[1] * 0.25) * 4.0,
        libm::ceilf(o[2] * 0.25) * 4.0,
        libm::ceilf(o[3] * 0.25) * 4.0,
    ];
    rect_intersect(r, device)
}

impl ClipStack {
    fn new(width: u32, height: u32, classify: bool) -> Self {
        let device_bounds = [0.0, 0.0, width as f32, height as f32];
        Self {
            device_bounds,
            elements: Vec::new(),
            saves: vec![SaveRecord {
                starting_element_index: 0,
                oldest_valid_index: 0,
                state: ClipState::WideOpen,
                outer: device_bounds,
                inner: device_bounds,
                intersect: true,
                deferred_saves: 0,
            }],
            clip_draws: Vec::new(),
            classify,
        }
    }

    /// `RawElement::drawClip` for element `i`: the deferred depth-only draw
    /// of its shape (inverse-filled for intersect elements), at one Z past
    /// the draws it clipped, sorted at the order fixed by its first use.
    fn draw_clip(&mut self, i: usize) {
        let e = &mut self.elements[i];
        let Some(order) = e.order.take() else {
            return;
        };
        let device = self.device_bounds;
        let snapped_outer = snap_scissor(e.outer, device);
        let mut scissor = rect_intersect(e.usage, snapped_outer);
        let area = |r: [f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
        if 0.5 * area(snapped_outer) < area(scissor) {
            scissor = snapped_outer;
        }
        let draw_bounds = if e.intersect { scissor } else { rect_intersect(e.outer, scissor) };
        let z = e.max_z + 1;
        e.max_z = 0;
        e.usage = INFINITE_INVERTED;
        if rect_empty(draw_bounds) {
            return;
        }
        let (path, rule) = e.shape.as_path();
        let polys = if self.classify {
            Vec::new()
        } else {
            let local = tess::fill_polygons_local(&path, &e.local_to_device.m);
            tess::to_device(&local, &e.local_to_device.m, 0, 0)
        };
        let inverse = e.intersect;
        self.clip_draws.push(Draw {
            geom: Geom::Polygons { polys, rule, msaa: true },
            paint: Paint::Solid([0.0; 4]),
            clip: DrawClip {
                scissor: [scissor[0] as i32, scissor[1] as i32, scissor[2] as i32, scissor[3] as i32],
                analytic: None,
                clip_order: FIRST_ORDER,
                draw_bounds,
            },
            z,
            order,
            depth_only: true,
            inverse,
        });
    }

    /// `recordDeferredClipDraws`: every pending element's clip draw.
    fn record_deferred_clip_draws(&mut self) {
        for i in 0..self.elements.len() {
            self.draw_clip(i);
        }
    }

    fn save(&mut self) {
        self.saves.last_mut().unwrap().deferred_saves += 1;
    }

    fn restore(&mut self) {
        let top = self.saves.last_mut().unwrap();
        if top.deferred_saves > 0 {
            top.deferred_saves -= 1;
            return;
        }
        let start = top.starting_element_index;
        // Elements leaving the stack draw their pending depth-only clips.
        for i in start..self.elements.len() {
            self.draw_clip(i);
        }
        self.elements.truncate(start);
        self.saves.pop();
        let depth = self.saves.len();
        for e in &mut self.elements {
            if e.invalid && e.save_depth >= depth {
                e.invalid = false;
            }
        }
    }

    fn ensure_record(&mut self) {
        let top = self.saves.last_mut().unwrap();
        if top.deferred_saves > 0 {
            top.deferred_saves -= 1;
            let new = SaveRecord {
                starting_element_index: self.elements.len(),
                oldest_valid_index: top.oldest_valid_index,
                state: top.state,
                outer: top.outer,
                inner: top.inner,
                intersect: top.intersect,
                deferred_saves: 0,
            };
            self.saves.push(new);
        }
    }

    /// `clipShape`.
    fn clip_shape(&mut self, local_to_device: Transform, shape: Shape, intersect: bool, pixel_snap: bool) {
        self.ensure_record();
        if self.saves.last().unwrap().state == ClipState::Empty {
            return;
        }
        let mut e = RawElement {
            shape,
            local_to_device,
            intersect,
            outer: [0.0; 4],
            inner: INFINITE_INVERTED,
            invalid: false,
            save_depth: self.saves.len(),
            order: None,
            max_z: 0,
            usage: INFINITE_INVERTED,
        };
        if matches!(e.shape, Shape::Line(_)) || !local_to_device.valid() {
            e.shape = Shape::Empty;
        }
        e.outer = rect_intersect(local_to_device.map_rect(e.shape.bounds()), self.device_bounds);
        if !rect_empty(e.outer) && local_to_device.ty <= TransformType::RectStaysRect {
            match &e.shape {
                Shape::Rect(_) => {
                    if pixel_snap {
                        e.outer = rect_round(e.outer);
                    }
                    e.shape = Shape::Rect(e.outer);
                    e.local_to_device = Transform::IDENTITY;
                    e.inner = e.outer;
                }
                Shape::RRect(rr) => {
                    if let Some(mut x) = rr.transform(&local_to_device.m) {
                        if pixel_snap {
                            x = SkRRect::make_rect_radii(rect_round(x.rect), x.radii);
                        }
                        e.outer = rect_intersect(x.rect, self.device_bounds);
                        let ib = x.inner_bounds();
                        e.inner = rect_intersect(ib, e.outer);
                        e.shape = Shape::RRect(x);
                        e.local_to_device = Transform::IDENTITY;
                    }
                }
                _ => {}
            }
        }
        if rect_empty(e.outer) {
            e.shape = Shape::Empty;
            e.inner = INFINITE_INVERTED;
        }
        // SaveRecord::addElement.
        let depth = self.saves.len();
        let top_i = self.saves.len() - 1;
        if matches!(e.shape, Shape::Empty) {
            if e.intersect {
                let top = &mut self.saves[top_i];
                top.state = ClipState::Empty;
                top.outer = [0.0; 4];
                top.inner = [0.0; 4];
                for i in 0..self.elements.len() {
                    self.draw_clip(i);
                    self.elements[i].invalid = true;
                }
            }
            return;
        }
        // Simplify against existing valid elements.
        let added = TShape {
            shape: &e.shape,
            xf: &e.local_to_device,
            outer: e.outer,
            inner: e.inner,
            intersect: e.intersect,
            bounds_only: false,
        };
        let mut skip_added = false;
        let mut invalidate = Vec::new();
        for (i, el) in self.elements.iter().enumerate() {
            if el.invalid {
                continue;
            }
            let existing = TShape {
                shape: &el.shape,
                xf: &el.local_to_device,
                outer: el.outer,
                inner: el.inner,
                intersect: el.intersect,
                bounds_only: false,
            };
            let a_only = |_: ()| {};
            let _ = a_only;
            match simplify_elements(&existing, &added) {
                SimplifyResult::Empty => {
                    self.saves[top_i].state = ClipState::Empty;
                    for i in 0..self.elements.len() {
                        self.draw_clip(i);
                        self.elements[i].invalid = true;
                    }
                    return;
                }
                SimplifyResult::AOnly => skip_added = true,
                SimplifyResult::BOnly => invalidate.push(i),
                SimplifyResult::Both => {}
            }
        }
        for i in invalidate {
            self.draw_clip(i);
            self.elements[i].invalid = true;
        }
        if skip_added {
            return;
        }
        // Update the record's bounds and state.
        let top = &mut self.saves[top_i];
        if e.intersect {
            top.outer = rect_intersect(top.outer, e.outer);
            top.inner = rect_intersect(top.inner, e.inner);
        } else {
            // Difference: the outer bounds are unchanged, the inner bounds lose
            // what the element covers (conservatively empty unless disjoint).
            if rect_intersects(top.inner, e.outer) {
                top.inner = INFINITE_INVERTED;
            }
        }
        let count = self.elements.iter().filter(|x| !x.invalid).count() + 1;
        top.state = if count == 1 && e.intersect {
            match e.shape {
                Shape::Rect(_) if e.local_to_device.ty == TransformType::Identity => ClipState::DeviceRect,
                Shape::RRect(_) if e.local_to_device.ty == TransformType::Identity => ClipState::DeviceRRect,
                _ => ClipState::Complex,
            }
        } else {
            ClipState::Complex
        };
        if rect_empty(top.outer) {
            top.state = ClipState::Empty;
        }
        top.intersect = top.intersect && e.intersect;
        e.save_depth = depth;
        self.elements.push(e);
    }

    /// `devClipBounds()`: the conservative bounds rounded out.
    fn dev_clip_bounds(&self) -> [i32; 4] {
        let r = rect_round_out(self.saves.last().unwrap().outer);
        [r[0] as i32, r[1] as i32, r[2] as i32, r[3] as i32]
    }

    /// `visitClipStackForDraw`: the clip a draw is subject to, with the
    /// draw's shape possibly replaced (geometric intersection).
    fn visit(&mut self, draw: &mut DrawShape, stroke_outset: f32, bm: &BoundsManager, z: u32) -> Option<DrawClip> {
        let cs = self.saves.last().unwrap().clone();
        if cs.state == ClipState::Empty {
            return None;
        }
        let device = self.device_bounds;
        // DrawShape::applyStyle (outsets for strokes and sub-pixel shapes).
        draw.apply_style(stroke_outset, device);
        draw.apply_scissor(if cs.intersect { snap_scissor(cs.outer, device) } else { device });
        let save = TShape {
            shape: &Shape::Rect(cs.outer),
            xf: &Transform::IDENTITY,
            outer: cs.outer,
            inner: cs.inner,
            intersect: cs.intersect,
            bounds_only: true,
        };
        match simplify(&save, &draw.tshape()) {
            Influence::ClipsOut => return None,
            Influence::None => return Some(draw.make_clip(None, FIRST_ORDER)),
            Influence::Replaces => draw.reset_to_flood_fill(),
            Influence::Complex => {}
        }
        let mut analytic: Option<AnalyticClip> = None;
        let mut effective: Vec<usize> = Vec::new();
        let oldest = cs.oldest_valid_index;
        for i in (0..self.elements.len()).rev() {
            if i < oldest {
                break;
            }
            if self.elements[i].invalid {
                continue;
            }
            let influence = {
                let e = &self.elements[i];
                let es = TShape {
                    shape: &e.shape,
                    xf: &e.local_to_device,
                    outer: e.outer,
                    inner: e.inner,
                    intersect: e.intersect,
                    bounds_only: false,
                };
                simplify(&es, &draw.tshape())
            };
            match influence {
                Influence::ClipsOut => return None,
                Influence::None => continue,
                Influence::Replaces | Influence::Complex => {
                    if influence == Influence::Replaces {
                        draw.reset_to_flood_fill();
                    }
                    let e = &self.elements[i];
                    if e.intersect && draw.intersect_clip_element(e) {
                        continue;
                    }
                    if let Shape::Rect(r) = e.shape
                        && e.local_to_device.ty == TransformType::Identity
                    {
                        let scissor = rect_round(r);
                        if rect_nearly_equals(r, scissor, PIXEL_TOLERANCE) {
                            draw.apply_scissor(scissor);
                            continue;
                        }
                    }
                    if analytic.is_none() {
                        analytic = can_apply_analytic_clip(&e.shape, &e.local_to_device, e.intersect);
                        if analytic.is_some() {
                            continue;
                        }
                    }
                    effective.push(i);
                }
            }
        }
        let mut clip = draw.make_clip(analytic, FIRST_ORDER);
        if !effective.is_empty() {
            // `RawElement::updateForDraw` for every element drawn into the
            // depth attachment: the first use fixes the clip draw's order
            // right after the most recent draw under the element's bounds.
            let snapped = snap_scissor(clip.draw_bounds, device);
            let mut max_order = FIRST_ORDER;
            for i in effective {
                let e = &mut self.elements[i];
                let order = match e.order {
                    None => {
                        let snapped_outer = snap_scissor(e.outer, device);
                        e.usage = snapped;
                        e.max_z = z;
                        let o = bm.most_recent(snapped_outer) + 1;
                        e.order = Some(o);
                        o
                    }
                    Some(o) => {
                        e.usage = if rect_empty(e.usage) { snapped } else { rect_union(e.usage, snapped) };
                        e.max_z = e.max_z.max(z);
                        o
                    }
                };
                max_order = max_order.max(order);
            }
            clip.clip_order = max_order;
        }
        Some(clip)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SimplifyResult {
    Empty,
    AOnly,
    BOnly,
    Both,
}

fn simplify_elements(a: &TShape, b: &TShape) -> SimplifyResult {
    match simplify(a, b) {
        Influence::ClipsOut => SimplifyResult::Empty,
        Influence::Replaces => SimplifyResult::AOnly,
        Influence::None => SimplifyResult::BOnly,
        Influence::Complex => SimplifyResult::Both,
    }
}

/// `can_apply_analytic_clip` (Chromium's build defines
/// `SK_GRAPHITE_USE_LEGACY_RRECT_CLIP_SHADER`): rects, and round rects whose
/// rounded corners are relatively circular and share one radius; an
/// elliptical oval (a non-uniformly scaled circle) is drawn into the depth
/// attachment with MSAA instead.
fn can_apply_analytic_clip(shape: &Shape, ltd: &Transform, intersect: bool) -> Option<AnalyticClip> {
    if ltd.ty != TransformType::Identity {
        return None;
    }
    let inverted = intersect;
    let rr = match shape {
        Shape::Rect(r) => return Some(AnalyticClip { rect: *r, radius: RADIUS_MIN, edge_flags: 0, inverted }),
        Shape::RRect(rr) => rr,
        _ => return None,
    };
    if rr.is_oval() || rr.is_simple() {
        let radii = rr.radii[0];
        if radii[0] < RADIUS_MIN || radii[1] < RADIUS_MIN {
            return Some(AnalyticClip { rect: rr.rect, radius: RADIUS_MIN, edge_flags: 0, inverted });
        }
        return if SkRRect::is_relatively_circular(radii[0], radii[1], PIXEL_TOLERANCE) {
            Some(AnalyticClip { rect: rr.rect, radius: radii[0], edge_flags: EDGE_ALL, inverted })
        } else {
            None
        };
    }
    const CORNER_FLAGS: [u32; 4] = [
        EDGE_TOP | EDGE_LEFT,
        EDGE_TOP | EDGE_RIGHT,
        EDGE_BOTTOM | EDGE_RIGHT,
        EDGE_BOTTOM | EDGE_LEFT,
    ];
    let mut circular_radius = 0.0f32;
    let mut edge_flags = 0u32;
    let mut square = 0;
    for (radii, flags) in rr.radii.iter().zip(CORNER_FLAGS) {
        if !SkRRect::is_relatively_circular(radii[0], radii[1], PIXEL_TOLERANCE) {
            return None;
        }
        if radii[0] < RADIUS_MIN || radii[1] < RADIUS_MIN {
            square += 1;
            continue;
        }
        if edge_flags == 0 {
            circular_radius = radii[0];
        } else if !SkRRect::is_relatively_circular(radii[0], circular_radius, PIXEL_TOLERANCE) {
            return None;
        }
        edge_flags |= flags;
    }
    if edge_flags == 0 {
        Some(AnalyticClip { rect: rr.rect, radius: RADIUS_MIN, edge_flags, inverted })
    } else if edge_flags == EDGE_ALL && square != 0 {
        None
    } else {
        Some(AnalyticClip { rect: rr.rect, radius: circular_radius, edge_flags, inverted })
    }
}

/// `sk_analytic_clip_legacy` evaluated at a device coordinate, with the
/// uniforms `ShadingParams::handleClipping` packs: the bounds outset by
/// half a pixel, `radiusPlusHalf = (±(r + 0.5), 1 / (r + 0.5))` (positive
/// for an inverted, i.e. intersect, clip) and the edge selector.
pub fn analytic_clip_coverage(c: &AnalyticClip, coords: [f32; 2]) -> f32 {
    let radius_plus_half = c.radius + 0.5;
    let rp = [if c.inverted { radius_plus_half } else { -radius_plus_half }, 1.0 / radius_plus_half];
    let r = [c.rect[0] - 0.5, c.rect[1] - 0.5, c.rect[2] + 0.5, c.rect[3] + 0.5];
    let sel = [
        if c.edge_flags & EDGE_LEFT != 0 { 1.0f32 } else { 0.0 },
        if c.edge_flags & EDGE_TOP != 0 { 1.0f32 } else { 0.0 },
        if c.edge_flags & EDGE_RIGHT != 0 { 1.0f32 } else { 0.0 },
        if c.edge_flags & EDGE_BOTTOM != 0 { 1.0f32 } else { 0.0 },
    ];
    let radius = rp[0].abs();
    let dxy0 = [sel[0] * ((r[0] + radius) - coords[0]), sel[1] * ((r[1] + radius) - coords[1])];
    let dxy1 = [sel[2] * (coords[0] - (r[2] - radius)), sel[3] * (coords[1] - (r[3] - radius))];
    let dxy = [dxy0[0].max(dxy1[0]).max(0.0), dxy0[1].max(dxy1[1]).max(0.0)];
    let len = ((dxy[0] * rp[1]) * (dxy[0] * rp[1]) + (dxy[1] * rp[1]) * (dxy[1] * rp[1])).sqrt();
    let circle = half((radius * (1.0 - len)).clamp(0.0, 1.0));
    let mut edges = [
        half((coords[0] - r[0]).clamp(0.0, 1.0)),
        half((coords[1] - r[1]).clamp(0.0, 1.0)),
        half((r[2] - coords[0]).clamp(0.0, 1.0)),
        half((r[3] - coords[1]).clamp(0.0, 1.0)),
    ];
    for i in 0..4 {
        if sel[i] != 0.0 {
            edges[i] = 1.0;
        }
    }
    let alpha = half(half(half(half(circle * edges[0]) * edges[1]) * edges[2]) * edges[3]);
    if rp[0] < 0.0 { half(1.0 - alpha) } else { alpha }
}

/// `half(x)`: round to the nearest `f16`.
#[inline]
pub fn half(v: f32) -> f32 {
    gradient::f16_round(v)
}

// ---------------------------------------------------------------------------
// Draw shapes

/// `ClipStack::DrawShape`.
struct DrawShape {
    /// The geometry to draw (replaced only by a geometric clip intersection).
    geometry: Shape,
    /// The shape the clip queries use (`fShape`; outset for strokes).
    shape: Shape,
    local_to_device: Transform,
    transformed_bounds: [f32; 4],
    outer: [f32; 4],
    inner: [f32; 4],
    scissor: [f32; 4],
    modified: bool,
    can_modify: bool,
    matches_geometry: bool,
    flood: bool,
}

impl DrawShape {
    fn new(shape: Shape, local_to_device: Transform) -> Self {
        let can_modify = matches!(shape, Shape::Rect(_) | Shape::RRect(_));
        Self {
            geometry: shape.clone(),
            shape,
            local_to_device,
            transformed_bounds: [0.0; 4],
            outer: [0.0; 4],
            inner: INFINITE_INVERTED,
            scissor: [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::INFINITY],
            modified: false,
            can_modify,
            matches_geometry: true,
            flood: false,
        }
    }

    fn tshape(&self) -> TShape<'_> {
        // A flood fill (inverted empty shape) is a "difference" shape.
        TShape {
            shape: &self.shape,
            xf: &self.local_to_device,
            outer: self.outer,
            inner: self.inner,
            intersect: !self.flood,
            bounds_only: !(self.can_modify && self.matches_geometry),
        }
    }

    /// `applyStyle` for a fill or a stroke with the given local outset.
    fn apply_style(&mut self, local_outset: f32, device: [f32; 4]) {
        let mut b = self.shape.bounds();
        let aa = self.local_to_device.local_aa_radius();
        if !aa.is_finite() {
            self.transformed_bounds = device;
            self.shape = Shape::Rect(device);
            self.local_to_device = Transform::IDENTITY;
            self.matches_geometry = false;
        } else {
            let mut outset = local_outset;
            let size = [b[2] - b[0], b[3] - b[1]];
            if (local_outset > 0.0 && local_outset * 2.0 < aa) || (local_outset == 0.0 && (size[0] < aa || size[1] < aa)) {
                outset += aa;
            }
            if outset > 0.0 {
                b = [b[0] - outset, b[1] - outset, b[2] + outset, b[3] + outset];
                if let Shape::RRect(rr) = &self.shape {
                    let mut radii = rr.radii;
                    for r in &mut radii {
                        r[0] += outset;
                        r[1] += outset;
                    }
                    self.shape = Shape::RRect(SkRRect::make_rect_radii(b, radii));
                } else {
                    self.shape = Shape::Rect(b);
                }
                self.matches_geometry = false;
            }
            self.transformed_bounds = self.local_to_device.map_rect(b);
        }
        self.outer = self.transformed_bounds;
        self.inner = INFINITE_INVERTED;
        if self.can_modify && self.matches_geometry && self.local_to_device.ty <= TransformType::RectStaysRect {
            match &self.shape {
                Shape::Rect(_) => self.inner = self.outer,
                Shape::RRect(rr) => {
                    let ib = rr.inner_bounds();
                    if !rect_empty(ib) {
                        self.inner = self.local_to_device.map_rect(ib);
                    }
                }
                _ => {}
            }
        }
    }

    fn apply_scissor(&mut self, scissor: [f32; 4]) {
        self.scissor = rect_intersect(self.scissor, scissor);
        self.outer = rect_intersect(self.outer, scissor);
        self.inner = rect_intersect(self.inner, scissor);
    }

    fn reset_to_flood_fill(&mut self) {
        if self.can_modify && self.matches_geometry && !self.flood {
            self.flood = true;
            self.outer = INFINITE_INVERTED;
            self.inner = INFINITE_INVERTED;
            self.modified = true;
        }
    }

    /// `intersectClipElement`: geometric intersection with a rect or rrect.
    fn intersect_clip_element(&mut self, clip: &RawElement) -> bool {
        let dbg = super::diagnostic("PM_SVG_DEBUG").is_some();
        if !(self.can_modify && self.matches_geometry) {
            if dbg {
                eprintln!("  intersect: cannot modify (can_modify={} matches_geometry={})", self.can_modify, self.matches_geometry);
            }
            return false;
        }
        let other_intersectable = matches!(clip.shape, Shape::Rect(_) | Shape::RRect(_));
        if !other_intersectable {
            return false;
        }
        // local -> other transform.
        let local_to_other: Option<Transform> = if clip.local_to_device == self.local_to_device {
            None
        } else if clip.local_to_device.ty == TransformType::Identity && self.local_to_device.ty <= TransformType::RectStaysRect {
            Some(self.local_to_device)
        } else if clip.local_to_device.ty <= TransformType::RectStaysRect && self.local_to_device.ty == TransformType::Identity {
            Some(Transform::new(clip.local_to_device.inv))
        } else {
            let m = Mat::concat(&clip.local_to_device.inv, &self.local_to_device.m);
            let t = Transform::new(m);
            if t.ty <= TransformType::RectStaysRect { Some(t) } else { return false }
        };
        let mut local_other_rect = clip.shape.bounds();
        if let Some(lto) = &local_to_other {
            let mapped = lto.inverse_map_rect(local_other_rect);
            let other_tol = PIXEL_TOLERANCE * clip.local_to_device.local_aa_radius();
            if rect_empty(local_other_rect) || !rect_nearly_equals(lto.map_rect(mapped), local_other_rect, other_tol) {
                return false;
            }
            local_other_rect = mapped;
        }
        if !self.flood {
            local_other_rect = rect_intersect(local_other_rect, self.shape.bounds());
        }
        let aa = self.local_to_device.local_aa_radius();
        if !aa.is_finite() || local_other_rect[2] - local_other_rect[0] <= aa || local_other_rect[3] - local_other_rect[1] <= aa {
            if dbg {
                eprintln!("  intersect: too small for aa {aa} rect {local_other_rect:?}");
            }
            return false;
        }
        if dbg {
            eprintln!("  intersect: shape {:?} clip {:?} local_other_rect {local_other_rect:?} lto {}", self.shape, clip.shape, local_to_other.is_some());
        }
        let other_rrect: SkRRect = match &clip.shape {
            Shape::Rect(_) => {
                if matches!(self.shape, Shape::Rect(_)) || self.flood {
                    self.shape = Shape::Rect(local_other_rect);
                    self.flood = false;
                    self.after_intersect(clip);
                    return true;
                }
                SkRRect::make_rect(local_other_rect)
            }
            Shape::RRect(rr) => {
                let r = match &local_to_other {
                    Some(lto) => match rr.transform(&lto.inv) {
                        Some(x) => x,
                        None => return false,
                    },
                    None => *rr,
                };
                if self.flood {
                    self.shape = Shape::RRect(r);
                    self.flood = false;
                    self.after_intersect(clip);
                    return true;
                }
                r
            }
            _ => return false,
        };
        let own = match &self.shape {
            Shape::Rect(r) => SkRRect::make_rect(*r),
            Shape::RRect(rr) => *rr,
            _ => return false,
        };
        let Some(local_rrect) = conservative_intersect(&other_rrect, &own) else {
            if dbg {
                eprintln!("  intersect: conservative_intersect failed for {other_rrect:?} and {own:?}");
            }
            return false;
        };
        if local_rrect.is_rect() {
            self.shape = Shape::Rect(local_rrect.rect);
        } else if !local_rrect.is_empty() {
            self.shape = Shape::RRect(local_rrect);
        } else {
            return false;
        }
        self.after_intersect(clip);
        true
    }

    fn after_intersect(&mut self, clip: &RawElement) {
        self.geometry = self.shape.clone();
        if rect_empty(self.outer) {
            self.outer = clip.outer;
            self.inner = clip.inner;
        } else {
            self.outer = rect_intersect(self.outer, clip.outer);
            self.inner = rect_intersect(self.inner, clip.inner);
        }
        self.modified = true;
    }

    fn make_clip(&mut self, analytic: Option<AnalyticClip>, clip_order: u32) -> DrawClip {
        if self.modified {
            self.transformed_bounds = self.local_to_device.map_rect(self.shape.bounds());
            self.outer = rect_intersect(self.transformed_bounds, self.scissor);
        }
        let s = rect_intersect(self.scissor, [-1e9, -1e9, 1e9, 1e9]);
        DrawClip {
            scissor: [s[0] as i32, s[1] as i32, s[2] as i32, s[3] as i32],
            analytic,
            clip_order,
            draw_bounds: if self.flood { self.scissor } else { self.outer },
        }
    }
}

/// `SkRRect::checkCornerContainment`.
fn check_corner_containment(rr: &SkRRect, x: f32, y: f32) -> bool {
    let r = rr.rect;
    let (cx, cy, index);
    if rr.is_oval() {
        cx = x - ((f64::from(r[0]) + f64::from(r[2])) * 0.5) as f32;
        cy = y - ((f64::from(r[1]) + f64::from(r[3])) * 0.5) as f32;
        index = 0;
    } else if x < r[0] + rr.radii[0][0] && y < r[1] + rr.radii[0][1] {
        index = 0;
        cx = x - (r[0] + rr.radii[0][0]);
        cy = y - (r[1] + rr.radii[0][1]);
    } else if x < r[0] + rr.radii[3][0] && y > r[3] - rr.radii[3][1] {
        index = 3;
        cx = x - (r[0] + rr.radii[3][0]);
        cy = y - (r[3] - rr.radii[3][1]);
    } else if x > r[2] - rr.radii[1][0] && y < r[1] + rr.radii[1][1] {
        index = 1;
        cx = x - (r[2] - rr.radii[1][0]);
        cy = y - (r[1] + rr.radii[1][1]);
    } else if x > r[2] - rr.radii[2][0] && y > r[3] - rr.radii[2][1] {
        index = 2;
        cx = x - (r[2] - rr.radii[2][0]);
        cy = y - (r[3] - rr.radii[2][1]);
    } else {
        return true;
    }
    let (rx, ry) = (rr.radii[index][0], rr.radii[index][1]);
    let dist = cx * cx * (ry * ry) + cy * cy * (rx * rx);
    dist <= (rx * ry) * (rx * ry)
}

fn rrect_corner(r: [f32; 4], corner: usize) -> [f32; 2] {
    match corner {
        0 => [r[0], r[1]],
        1 => [r[2], r[1]],
        2 => [r[2], r[3]],
        _ => [r[0], r[3]],
    }
}

/// `SkRRectPriv::ConservativeIntersect`: `None` when the intersection is
/// not a round rect.
fn conservative_intersect(a: &SkRRect, b: &SkRRect) -> Option<SkRRect> {
    let r = rect_intersect(a.rect, b.rect);
    if rect_empty(r) {
        return Some(SkRRect::make_rect([0.0; 4]));
    }
    let inside_corner = |corner: usize, p: [f32; 2], q: [f32; 2]| match corner {
        0 => p[0] >= q[0] && p[1] >= q[1],
        1 => p[0] <= q[0] && p[1] >= q[1],
        2 => p[0] <= q[0] && p[1] <= q[1],
        _ => p[0] >= q[0] && p[1] <= q[1],
    };
    let mut radii = [[0.0f32; 2]; 4];
    for (corner, out) in radii.iter_mut().enumerate() {
        let test = rrect_corner(r, corner);
        let ac = rrect_corner(a.rect, corner);
        let bc = rrect_corner(b.rect, corner);
        let (ar, br) = (a.radii[corner], b.radii[corner]);
        let ok = if test == ac && test == bc {
            if ar[0] >= br[0] && ar[1] >= br[1] {
                *out = ar;
                true
            } else if br[0] >= ar[0] && br[1] >= ar[1] {
                *out = br;
                true
            } else {
                false
            }
        } else if test == ac {
            *out = ar;
            if ar == br { inside_corner(corner, ac, bc) } else { check_corner_containment(b, ac[0], ac[1]) }
        } else if test == bc {
            *out = br;
            if br == ar { inside_corner(corner, bc, ac) } else { check_corner_containment(a, bc[0], bc[1]) }
        } else {
            *out = [0.0, 0.0];
            check_corner_containment(a, test[0], test[1]) && check_corner_containment(b, test[0], test[1])
        };
        if !ok {
            return None;
        }
    }
    // `SkRRect::AreRectAndRadiiValid` (`are_radius_check_predicates_valid`
    // per radius, in float: a top of 10.000001 rejects a radius of 30 in a
    // rect of height 30 because `bottom - radius >= top` fails) and no
    // radii scaling.
    let ok = |rad: f32, min: f32, max: f32| min <= max && rad <= max - min && min + rad <= max && max - rad >= min && rad >= 0.0;
    let valid = radii
        .iter()
        .all(|c| ok(c[0], r[0], r[2]) && ok(c[1], r[1], r[3]));
    if !valid {
        return None;
    }
    let out = SkRRect::make_rect_radii(r, radii);
    if out.radii != radii {
        return None;
    }
    Some(out)
}

/// What clips one draw: the scissor, at most one analytic clip, and the
/// depth-only elements.
#[derive(Debug, Clone)]
pub struct DrawClip {
    pub scissor: [i32; 4],
    pub analytic: Option<AnalyticClip>,
    /// The highest painter's order of the clip elements drawn into the
    /// depth attachment for this draw (`First` when none).
    pub clip_order: u32,
    pub draw_bounds: [f32; 4],
}

// ---------------------------------------------------------------------------
// Draws

/// The paint of a draw: a premultiplied solid color, a gradient shader, or
/// an image (a restored layer) sampled at pixel centers.
#[derive(Debug, Clone)]
pub enum Paint {
    Solid([f32; 4]),
    Gradient { shader: Arc<Shader>, local_to_device: Transform, alpha: f32, dither: bool },
    Image(Arc<LayerImage>),
}

#[derive(Debug, Clone)]
pub enum Geom {
    /// Polygons under a fill rule (tessellated fills: MSAA; the non-AA
    /// bounds renderer for pixel-aligned rects: no MSAA requirement).
    Polygons { polys: Vec<Vec<[f64; 2]>>, rule: Rule, msaa: bool },
    /// The union of triangles (tessellated strokes and convex fills, MSAA).
    Triangles(Vec<[P; 3]>),
    /// One analytic rrect instance.
    Analytic(analytic::Instance),
}

/// A rendered layer, premultiplied RGBA8.
#[derive(Debug)]
pub struct LayerImage {
    pub origin: [i32; 2],
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub alpha: f32,
}

#[derive(Debug, Clone)]
pub struct Draw {
    pub geom: Geom,
    pub paint: Paint,
    pub clip: DrawClip,
    /// `PaintersDepth`: recording order, 1-based; the depth test passes
    /// where this is greater than the stored Z (`kDirectDepthLessPass` on
    /// `1 - z / max`).
    pub z: u32,
    /// `CompressedPaintersOrder`: the primary sort key of the draw pass.
    pub order: u32,
    /// A depth-only clip draw (no color).
    pub depth_only: bool,
    /// The geometry is inverse-filled (an intersect clip element covers
    /// everything outside its shape).
    pub inverse: bool,
}

/// What the filter evaluation needs to know about the device it draws into.
#[derive(Debug, Clone, Copy)]
pub struct DeviceInfo {
    /// `devClipBounds()` in the device's pixel space.
    pub clip_bounds: [i32; 4],
}

/// A device (the canvas or a layer): its draw list and its origin in the
/// parent's device space.
struct Device {
    origin: [i32; 2],
    width: u32,
    height: u32,
    draws: Vec<Draw>,
    /// The layer's restore paint.
    alpha: f32,
    filter: Option<ImageFilter>,
    /// The clip stack depth when the layer began (restored on pop).
    layer_bounds_in_parent: [i32; 4],
    /// `fCurrentDepth`: the last Z handed out.
    current_z: u32,
    bm: BoundsManager,
}

struct Mc {
    matrix: M44,
    /// Whether this save started a layer.
    layer: bool,
}

/// The replayed canvas.
pub struct Canvas<'a> {
    mc: Vec<Mc>,
    devices: Vec<Device>,
    clip: ClipStack,
    clip_saves: Vec<ClipStack>,
    pub text: Option<Box<TextDrawFn<'a>>>,
    /// Classification only ([`Canvas::classifier`]): draws are recorded
    /// with their renderer but without tessellated geometry, and nothing is
    /// rasterized.
    classify: bool,
    /// The first device whose draw pass the emulation does not reproduce
    /// (see [`pass_verdict`]).
    verdict: Option<SvgError>,
}

/// The host's SVG text raster: the text element, its local-to-device
/// matrix and the device size, to premultiplied RGBA8.
pub type TextDrawFn<'a> = dyn FnMut(&usvg::Text, Mat, u32, u32) -> Result<Vec<u8>, SvgError> + 'a;

impl<'a> Canvas<'a> {
    pub fn new(width: u32, height: u32) -> Self {
        Self::with_mode(width, height, false)
    }

    /// A canvas that records which renderer and clip each draw gets, as the
    /// raster would, without tessellating or rasterizing; [`Canvas::verdict`]
    /// then says whether every draw pass is one the emulation reproduces.
    pub fn classifier(width: u32, height: u32) -> Self {
        Self::with_mode(width, height, true)
    }

    fn with_mode(width: u32, height: u32, classify: bool) -> Self {
        Self {
            mc: vec![Mc { matrix: M44::IDENTITY, layer: false }],
            devices: vec![Device {
                origin: [0, 0],
                width,
                height,
                draws: Vec::new(),
                alpha: 1.0,
                filter: None,
                layer_bounds_in_parent: [0, 0, width as i32, height as i32],
                current_z: 0,
                bm: BoundsManager::default(),
            }],
            clip: ClipStack::new(width, height, classify),
            clip_saves: Vec::new(),
            text: None,
            classify,
            verdict: None,
        }
    }

    /// After [`Canvas::finish`] (or [`Canvas::classify_finish`]): the first
    /// draw pass outside what the emulation reproduces, if any.
    pub fn verdict(&self) -> Result<(), SvgError> {
        match &self.verdict {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn judge(&mut self, dev: &Device) {
        if self.verdict.is_none()
            && let Err(e) = pass_verdict(dev)
        {
            self.verdict = Some(e);
        }
    }

    fn matrix(&self) -> M44 {
        self.mc.last().unwrap().matrix
    }

    /// The local-to-device transform of the current device (the layer's
    /// origin removed).
    fn local_to_device(&self) -> Transform {
        let mut m = self.matrix().to_mat();
        let d = self.devices.last().unwrap();
        m.tx -= d.origin[0] as f32;
        m.ty -= d.origin[1] as f32;
        Transform::new(m)
    }

    pub fn save(&mut self) {
        let m = self.matrix();
        self.mc.push(Mc { matrix: m, layer: false });
        self.clip.save();
    }

    pub fn restore(&mut self) -> Result<(), SvgError> {
        if self.mc.len() <= 1 {
            return Ok(());
        }
        let mc = self.mc.pop().unwrap();
        self.clip.restore();
        if mc.layer {
            self.restore_layer()?;
        }
        Ok(())
    }

    pub fn concat(&mut self, m: &M44) {
        self.mc.last_mut().unwrap().matrix.pre_concat(m);
    }

    pub fn translate(&mut self, x: f32, y: f32) {
        self.mc.last_mut().unwrap().matrix.pre_translate(x, y);
    }

    pub fn clip_rect(&mut self, rect: [f32; 4], aa: bool) {
        let ltd = self.local_to_device();
        self.clip.clip_shape(ltd, Shape::Rect(sorted(rect)), true, !aa);
    }

    pub fn clip_rrect(&mut self, rrect: SkRRect, aa: bool) {
        let ltd = self.local_to_device();
        self.clip.clip_shape(ltd, Shape::RRect(rrect), true, !aa);
    }

    /// `SkCanvas::clipPath` → `onClipPath`: rects, ovals, and rrects become
    /// rect/rrect clips when the matrix keeps rectangles.
    pub fn clip_path(&mut self, path: &SkPath, rule: Rule, aa: bool) {
        let ltd = self.local_to_device();
        if ltd.ty <= TransformType::RectStaysRect {
            if let Some((r, _)) = path.is_rect() {
                return self.clip_rect(r, aa);
            }
            if let Some(r) = path.is_oval() {
                return self.clip_rrect(SkRRect::make_oval(r), aa);
            }
            if let Some(rr) = path.is_rrect() {
                return self.clip_rrect(rr, aa);
            }
        }
        self.clip.clip_shape(ltd, Shape::Path(path.clone(), rule), true, false);
    }

    /// `saveLayerAlphaf(nullptr, alpha)` / `saveLayer(nullptr, paint with
    /// an image filter)`: the layer sized to the clip bounds (plus what the
    /// filter needs and one pixel of padding).
    pub fn save_layer(&mut self, alpha: f32, filter: Option<ImageFilter>) -> Result<(), SvgError> {
        let m = self.matrix();
        self.mc.push(Mc { matrix: m, layer: true });
        self.clip.save();
        let parent = self.devices.last().unwrap();
        let clip_bounds = self.clip.dev_clip_bounds();
        let clip_bounds = [
            clip_bounds[0].max(0),
            clip_bounds[1].max(0),
            clip_bounds[2].min(parent.width as i32),
            clip_bounds[3].min(parent.height as i32),
        ];
        let mut bounds = clip_bounds;
        if let Some(f) = &filter {
            let ltd = self.local_to_device();
            bounds = super::blur::input_layer_bounds(f, &ltd, clip_bounds);
            // One pixel of padding (internalSaveLayer).
            bounds = [bounds[0] - 1, bounds[1] - 1, bounds[2] + 1, bounds[3] + 1];
        }
        if bounds[2] <= bounds[0] || bounds[3] <= bounds[1] {
            bounds = [0, 0, 0, 0];
        }
        let (w, h) = ((bounds[2] - bounds[0]).max(0) as u32, (bounds[3] - bounds[1]).max(0) as u32);
        // The layer's origin in the parent's device space; draws inside use
        // the parent's origin plus this.
        let origin = [parent.origin[0] + bounds[0], parent.origin[1] + bounds[1]];
        self.devices.push(Device {
            origin,
            width: w,
            height: h,
            draws: Vec::new(),
            alpha,
            filter,
            layer_bounds_in_parent: bounds,
            current_z: 0,
            bm: BoundsManager::default(),
        });
        // The layer starts with a wide-open clip of its own bounds.
        let saved = std::mem::replace(&mut self.clip, ClipStack::new(w, h, self.classify));
        self.clip_saves.push(saved);
        Ok(())
    }

    fn restore_layer(&mut self) -> Result<(), SvgError> {
        self.clip.record_deferred_clip_draws();
        self.take_clip_draws();
        let layer = self.devices.pop().unwrap();
        self.clip = self.clip_saves.pop().unwrap();
        // The layer's clip stack state was saved at saveLayer; restore it.
        self.clip.restore();
        self.judge(&layer);
        let image = if self.classify { Vec::new() } else { render_device(&layer) };
        let mut pending: Vec<([f32; 4], Arc<LayerImage>)> = Vec::new();
        match &layer.filter {
            None => {
                let img = Arc::new(LayerImage {
                    origin: [layer.layer_bounds_in_parent[0], layer.layer_bounds_in_parent[1]],
                    width: layer.width,
                    height: layer.height,
                    rgba: image,
                    alpha: layer.alpha,
                });
                let rect = [
                    img.origin[0] as f32,
                    img.origin[1] as f32,
                    (img.origin[0] + img.width as i32) as f32,
                    (img.origin[1] + img.height as i32) as f32,
                ];
                pending.push((rect, img));
            }
            Some(_) if self.classify => {
                // The filter's output draws are image draws like the
                // unfiltered composite; their bounds do not change which
                // renderer they get.
                let b = layer.layer_bounds_in_parent;
                let img = Arc::new(LayerImage { origin: [b[0], b[1]], width: layer.width, height: layer.height, rgba: Vec::new(), alpha: layer.alpha });
                pending.push(([b[0] as f32, b[1] as f32, b[2] as f32, b[3] as f32], img));
            }
            Some(filter) => {
                let ltd = self.local_to_device();
                let parent = self.devices.last().unwrap();
                let cb = self.clip.dev_clip_bounds();
                let info = DeviceInfo {
                    clip_bounds: [cb[0].max(0), cb[1].max(0), cb[2].min(parent.width as i32), cb[3].min(parent.height as i32)],
                };
                pending = super::blur::filter_layer(filter, &ltd, &layer.layer_bounds_in_parent, layer.width, layer.height, &image, &info)?;
            }
        }
        for (rect, img) in pending {
            let mut shape = DrawShape::new(Shape::Rect(rect), Transform::IDENTITY);
            let z = self.next_z();
            let dev = self.devices.last().unwrap();
            let Some(clip) = self.clip.visit(&mut shape, 0.0, &dev.bm, z) else {
                continue;
            };
            let Some(geom) = fill_geometry(&shape.geometry, &shape.local_to_device, self.classify)? else {
                continue;
            };
            self.record(geom, Paint::Image(img), clip, z);
        }
        Ok(())
    }

    /// `fCurrentDepth.next()` for the current device, after the clip draws
    /// recorded by restores since the last draw have advanced the depth
    /// (`Device::drawClipShape`).
    fn next_z(&mut self) -> u32 {
        self.take_clip_draws();
        let dev = self.devices.last_mut().unwrap();
        dev.current_z += 1;
        dev.current_z
    }

    /// Moves the clip stack's pending depth-only draws into the device,
    /// advancing its Z past theirs (`Device::drawClipShape`).
    fn take_clip_draws(&mut self) {
        let draws = std::mem::take(&mut self.clip.clip_draws);
        let dev = self.devices.last_mut().unwrap();
        for d in draws {
            dev.current_z = dev.current_z.max(d.z);
            dev.draws.push(d);
        }
    }

    // -- draws ------------------------------------------------------------

    fn paint_of(&self, flags: &Flags, local_to_device: Transform) -> Paint {
        match &flags.shader {
            None => {
                let c = flags.color;
                Paint::Solid([c[0] * c[3], c[1] * c[3], c[2] * c[3], c[3]])
            }
            Some(g) => Paint::Gradient {
                shader: Arc::new(gradient::Shader::from_blink(g)),
                local_to_device,
                alpha: flags.color[3],
                dither: flags.dither,
            },
        }
    }

    pub fn draw_rect(&mut self, r: [f32; 4], flags: &Flags) -> Result<(), SvgError> {
        let r = sorted(r);
        self.draw_shape(Shape::Rect(r), flags)
    }

    pub fn draw_oval(&mut self, r: [f32; 4], flags: &Flags) -> Result<(), SvgError> {
        let r = sorted(r);
        if flags.stroke.as_ref().is_some_and(|s| s.dash.is_some()) {
            return self.draw_shape(Shape::Path(super::skrrect::oval_path(r, true, 0), Rule::NonZero), flags);
        }
        self.draw_shape(Shape::RRect(SkRRect::make_oval(r)), flags)
    }

    /// `SkCanvas::drawPath` → `Device::drawPath`: lines, ovals, rrects, and
    /// rects are recognized before path rendering.
    pub fn draw_path(&mut self, path: &SkPath, rule: Rule, flags: &Flags) -> Result<(), SvgError> {
        let has_effect = flags.stroke.as_ref().is_some_and(|s| s.dash.is_some());
        if !has_effect {
            if let Some([a, b]) = path.is_line() {
                if flags.stroke.is_some() {
                    return self.draw_shape(Shape::Line([a[0], a[1], b[0], b[1]]), flags);
                }
                return Ok(());
            }
            if let Some(r) = path.is_oval() {
                return self.draw_shape(Shape::RRect(SkRRect::make_oval(r)), flags);
            }
            if let Some(rr) = path.is_rrect() {
                return self.draw_shape(Shape::RRect(rr), flags);
            }
            if let Some((r, closed)) = path.is_rect()
                && (flags.stroke.is_none() || closed)
            {
                return self.draw_shape(Shape::Rect(r), flags);
            }
        }
        self.draw_shape(Shape::Path(path.clone(), rule), flags)
    }

    /// `Device::drawGeometryWithPathEffect` + `drawGeometry`.
    fn draw_shape(&mut self, mut shape: Shape, flags: &Flags) -> Result<(), SvgError> {
        let ltd = self.local_to_device();
        if !ltd.valid() {
            return Ok(());
        }
        if let Some(s) = &flags.stroke
            && let Some((intervals, phase)) = &s.dash
        {
            let (path, rule) = shape.as_path();
            // Classification needs only the dashed shape's kind (a path).
            let dashed = if self.classify { path } else { super::dash::dash_path(&path, intervals, *phase, ltd.max_scale)? };
            shape = Shape::Path(dashed, rule);
        }
        let paint = self.paint_of(flags, ltd);
        let stroke = flags.stroke.as_ref().map(stroke_style);
        // Stroke outset for the clip query (SkStrokeRec::GetInflationRadius).
        let outset = stroke.as_ref().map_or(0.0, |(p, cap, miter)| {
            let is_path = matches!(shape, Shape::Path(..));
            let effective_miter = if is_path { *miter } else { 1.0 };
            let effective_cap = if matches!(shape, Shape::Rect(_) | Shape::RRect(_)) { Cap::Butt } else { *cap };
            inflation_radius(p.join, effective_miter, effective_cap, p.radius * 2.0)
        });
        let mut draw = DrawShape::new(shape, ltd);
        let z = self.next_z();
        let dev = self.devices.last().unwrap();
        let Some(clip) = self.clip.visit(&mut draw, outset, &dev.bm, z) else {
            return Ok(());
        };
        let shape = draw.geometry.clone();
        let ltd = draw.local_to_device;
        if draw.flood {
            // A flood fill: the whole scissor.
            let s = clip.scissor;
            let polys = vec![vec![
                [s[0] as f64, s[1] as f64],
                [s[2] as f64, s[1] as f64],
                [s[2] as f64, s[3] as f64],
                [s[0] as f64, s[3] as f64],
            ]];
            self.record(Geom::Polygons { polys, rule: Rule::NonZero, msaa: false }, paint, clip, z);
            return Ok(());
        }
        let geom = match &stroke {
            None => fill_geometry(&shape, &ltd, self.classify)?,
            Some((params, cap, _)) => stroke_geometry(&shape, &ltd, *params, *cap, self.classify)?,
        };
        let Some(geom) = geom else { return Ok(()) };
        self.record(geom, paint, clip, z);
        Ok(())
    }

    /// Records a draw with its painter's order (`Device::drawGeometry`): a
    /// draw that reads the destination (coverage AA or a translucent paint)
    /// sorts after the most recent draw it overlaps; every draw sorts after
    /// the clip elements it needs drawn.
    fn record(&mut self, geom: Geom, paint: Paint, clip: DrawClip, z: u32) {
        self.take_clip_draws();
        let depends_on_dst = match (&geom, &paint) {
            (Geom::Analytic(_), _) => true,
            (_, Paint::Solid(c)) => c[3] < 1.0,
            (_, Paint::Gradient { shader, alpha, .. }) => *alpha < 1.0 || shader.colors.iter().any(|c| c[3] < 1.0),
            (_, Paint::Image(_)) => true,
        };
        let dev = self.devices.last_mut().unwrap();
        // `DrawOrder order(depth)` starts at First; `dependsOnPaintersOrder`
        // takes the next order after each dependency.
        let mut order = FIRST_ORDER.max(clip.clip_order);
        if depends_on_dst {
            order = order.max(dev.bm.most_recent(clip.draw_bounds) + 1);
        }
        dev.bm.record(clip.draw_bounds, order);
        dev.draws.push(Draw { geom, paint, clip, z, order, depth_only: false, inverse: false });
    }

    pub fn draw_text(&mut self, t: &usvg::Text) -> Result<(), SvgError> {
        let ltd = self.local_to_device();
        let d = self.devices.last().unwrap();
        let (w, h) = (d.width, d.height);
        let Some(text) = self.text.as_mut() else {
            return Err(SvgError("SVG text needs pm-text".into()));
        };
        let rgba = text(t, ltd.m, w, h)?;
        let img = Arc::new(LayerImage { origin: [0, 0], width: w, height: h, rgba, alpha: 1.0 });
        let rect = [0.0, 0.0, w as f32, h as f32];
        let mut draw = DrawShape::new(Shape::Rect(rect), Transform::IDENTITY);
        let z = self.next_z();
        let dev = self.devices.last().unwrap();
        let Some(clip) = self.clip.visit(&mut draw, 0.0, &dev.bm, z) else {
            return Ok(());
        };
        let Some(geom) = fill_geometry(&draw.geometry, &draw.local_to_device, self.classify)? else {
            return Ok(());
        };
        self.record(geom, Paint::Image(img), clip, z);
        Ok(())
    }

    /// Replays the ops and renders the canvas.
    pub fn replay(&mut self, ops: &[Op]) -> Result<(), SvgError> {
        for op in ops {
            match op {
                Op::Save => self.save(),
                Op::Restore => self.restore()?,
                Op::Concat(m) => self.concat(m),
                Op::Translate(x, y) => self.translate(*x, *y),
                Op::ClipRect { rect, aa } => self.clip_rect(*rect, *aa),
                Op::ClipRRect { rrect, aa } => self.clip_rrect(*rrect, *aa),
                Op::ClipPath { path, rule, aa } => self.clip_path(path, *rule, *aa),
                Op::SaveLayerAlpha(a) => self.save_layer(*a, None)?,
                Op::SaveLayer { filter } => self.save_layer(1.0, Some(filter.clone()))?,
                Op::DrawRect { rect, flags } => self.draw_rect(*rect, flags)?,
                Op::DrawOval { rect, flags } => self.draw_oval(*rect, flags)?,
                Op::DrawPath { path, rule, flags } => self.draw_path(path, *rule, flags)?,
                Op::DrawText(t) => self.draw_text(&t.node)?,
            }
        }
        Ok(())
    }

    pub fn finish(&mut self) -> Vec<u8> {
        self.clip.record_deferred_clip_draws();
        self.take_clip_draws();
        if self.verdict.is_none()
            && let Err(e) = pass_verdict(&self.devices[0])
        {
            self.verdict = Some(e);
        }
        if self.classify {
            return Vec::new();
        }
        render_device(&self.devices[0])
    }
}

/// `StrokeStyle` from the flags: `(params, cap, miter limit)`.
fn stroke_style(s: &StrokeFlags) -> (StrokeParams, Cap, f32) {
    let join = match s.join {
        1 => -1.0,
        2 => 0.0,
        _ => s.miter,
    };
    let cap = match s.cap {
        1 => Cap::Round,
        2 => Cap::Square,
        _ => Cap::Butt,
    };
    (StrokeParams { radius: s.width / 2.0, join }, cap, s.miter)
}

/// `SkStrokeRec::GetInflationRadius`.
fn inflation_radius(join: f32, miter_limit: f32, cap: Cap, width: f32) -> f32 {
    if width < 0.0 {
        return 0.0;
    }
    let mut multiplier = 1.0f32;
    if join > 0.0 {
        multiplier = miter_limit.max(multiplier);
    }
    if cap == Cap::Square {
        multiplier = core::f32::consts::SQRT_2.max(multiplier);
    }
    width * 0.5 * multiplier
}

/// `is_pixel_aligned`.
fn is_pixel_aligned(r: [f32; 4], t: &Transform) -> bool {
    if t.ty <= TransformType::RectStaysRect {
        let dev = t.map_rect(r);
        return rect_nearly_equals(dev, rect_round(dev), PIXEL_TOLERANCE);
    }
    false
}

/// `is_simple_shape` for fills and strokes.
fn is_simple_shape(shape: &Shape, t: &Transform, stroke: bool) -> bool {
    match shape {
        Shape::RRect(rr) if stroke => {
            let tol = t.local_aa_radius() * PIXEL_TOLERANCE;
            rr.all_corners_relatively_circular(tol)
        }
        Shape::RRect(_) | Shape::Rect(_) | Shape::Line(_) => true,
        _ => false,
    }
}

/// `load_x_radii` / `load_y_radii`: per-corner radii in UL, UR, LR, LL order.
fn corner_radii(rr: &SkRRect) -> ([f32; 4], [f32; 4]) {
    (
        [rr.radii[0][0], rr.radii[1][0], rr.radii[2][0], rr.radii[3][0]],
        [rr.radii[0][1], rr.radii[1][1], rr.radii[2][1], rr.radii[3][1]],
    )
}

/// The fill geometry `chooseRenderer` selects (`classify`: the renderer
/// without the tessellated polygons).
fn fill_geometry(shape: &Shape, t: &Transform, classify: bool) -> Result<Option<Geom>, SvgError> {
    match shape {
        Shape::Empty | Shape::Line(_) => Ok(None),
        Shape::Rect(r) => {
            if r[2] - r[0] <= 0.0 || r[3] - r[1] <= 0.0 {
                return Ok(None);
            }
            if is_pixel_aligned(*r, t) {
                let corners = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
                let polys = tess::to_device(&[corners.to_vec()], &t.m, 0, 0);
                return Ok(Some(Geom::Polygons { polys, rule: Rule::NonZero, msaa: false }));
            }
            Ok(Some(Geom::Analytic(analytic::instance(&AnalyticGeometry::FillRect(*r), &t.m))))
        }
        Shape::RRect(rr) => {
            if rr.is_rect() {
                return fill_geometry(&Shape::Rect(rr.rect), t, classify);
            }
            let (x_radii, y_radii) = corner_radii(rr);
            Ok(Some(Geom::Analytic(analytic::instance(&AnalyticGeometry::FillRRect { rect: rr.rect, x_radii, y_radii }, &t.m))))
        }
        Shape::Path(p, rule) => {
            if classify {
                return Ok(Some(Geom::Triangles(Vec::new())));
            }
            if p.is_convex() && *rule == Rule::NonZero {
                let tris = tess::convex_wedge_triangles(p, &t.m, 0, 0);
                return Ok(Some(Geom::Triangles(tris)));
            }
            let local = tess::fill_polygons_local(p, &t.m);
            let polys = tess::to_device(&local, &t.m, 0, 0);
            Ok(Some(Geom::Polygons { polys, rule: *rule, msaa: true }))
        }
    }
}

/// The stroke geometry `chooseRenderer` selects.
fn stroke_geometry(shape: &Shape, t: &Transform, params: StrokeParams, cap: Cap, classify: bool) -> Result<Option<Geom>, SvgError> {
    if is_simple_shape(shape, t, true) {
        let g = match shape {
            Shape::Line(l) => AnalyticGeometry::StrokeLine { line: *l, radius: params.radius, cap },
            Shape::Rect(r) => AnalyticGeometry::StrokeRect { rect: *r, radius: params.radius, join: params.join },
            Shape::RRect(rr) => {
                if rr.is_rect() {
                    AnalyticGeometry::StrokeRect { rect: rr.rect, radius: params.radius, join: params.join }
                } else {
                    let (x_radii, y_radii) = corner_radii(rr);
                    AnalyticGeometry::StrokeRRect { rect: rr.rect, x_radii, y_radii, radius: params.radius, join: params.join }
                }
            }
            _ => unreachable!(),
        };
        return Ok(Some(Geom::Analytic(analytic::instance(&g, &t.m))));
    }
    if classify {
        return Ok(Some(Geom::Triangles(Vec::new())));
    }
    let (path, _) = shape.as_path();
    let tris = stroke::stroke_triangles(&path, &t.m, params, cap);
    Ok(Some(Geom::Triangles(tris)))
}

// ---------------------------------------------------------------------------
// Rasterization

/// Whether the emulation reproduces Chromium's pixels for a device's draw
/// pass. Measured on Chromium 152 / Graphite / Metal (docs/engine/vector.md,
/// "Chromium SVG paint-record replay"):
///
/// - analytic draws (round rects, ovals, unaligned rects) in a pass with no
///   multisampled draw, and multisampled draws (tessellated paths) in a pass
///   with no analytic draw and no clip, match;
/// - an analytic draw or an analytic clip in a multisampled pass takes, at
///   isolated pixels along mesh edges, one mesh triangle's fragment for
///   every sample, which the emulation does not model;
/// - clip elements drawn into the depth attachment have no reference that
///   is free of the multisampled-pass behaviour.
///
/// A document whose bucket raster contains any such pass is not drawn.
fn pass_verdict(dev: &Device) -> Result<(), SvgError> {
    if dev.draws.iter().any(|d| d.depth_only) {
        return Err(SvgError("an SVG clip drawn into the depth attachment needs compatibility support".into()));
    }
    let msaa = dev.draws.iter().any(needs_msaa);
    if msaa && dev.draws.iter().any(|d| matches!(d.geom, Geom::Analytic(_)) || d.clip.analytic.is_some()) {
        return Err(SvgError(
            "analytic SVG draws sharing a multisampled pass with tessellated draws need compatibility support".into(),
        ));
    }
    Ok(())
}

fn needs_msaa(d: &Draw) -> bool {
    match &d.geom {
        Geom::Polygons { msaa, .. } => *msaa || d.depth_only,
        Geom::Triangles(_) => true,
        Geom::Analytic(_) => false,
    }
}

/// Renders a device's draws: MSAA when any draw or clip element requires it.
fn render_device(dev: &Device) -> Vec<u8> {
    if let Ok(path) = super::diagnostic("PM_SVG_MESH").ok_or(()) {
        // The analytic draws' device-space mesh strips (36 vertices, index
        // strip expanded to sequential vertices) for the hardware probe
        // (tools/canvas-ref/wgpu-probe.ts).
        let mut meshes = Vec::new();
        for d in &dev.draws {
            if let Geom::Analytic(inst) = &d.geom {
                meshes.push(analytic::mesh_strip(inst));
            }
        }
        if super::diagnostic("PM_SVG_MESH_VERTS").is_some() {
            for d in &dev.draws {
                if let Geom::Analytic(inst) = &d.geom {
                    for (i, v) in analytic::mesh_vertices(inst).iter().enumerate() {
                        eprintln!("VERT {i} pos {:?} edge {:?} ctrl {:?}", v.pos, v.edge, v.control);
                    }
                }
            }
        }
        let strips: Vec<String> = meshes
            .iter()
            .map(|m| format!("[{}]", m.iter().map(|p| format!("[{},{}]", p[0], p[1])).collect::<Vec<_>>().join(",")))
            .collect();
        let doc = format!("{{\"width\":{},\"height\":{},\"meshes\":[{}]}}", dev.width, dev.height, strips.join(","));
        std::fs::write(&path, doc).unwrap();
    }
    if super::diagnostic("PM_SVG_STRADDLE").is_some() {
        // Every pixel where an analytic draw's mesh splits the samples
        // between triangles (MSAA devices only): "STRADDLE draw x y
        // tri:covers:center;..." for offline comparison with a capture.
        let msaa = dev.draws.iter().any(needs_msaa);
        if msaa {
            for (i, d) in dev.draws.iter().enumerate() {
                if let Geom::Analytic(inst) = &d.geom {
                    for py in 0..dev.height {
                        for px in 0..dev.width {
                            let e = analytic::explain_data(inst, px, py, &SAMPLES_4X);
                            let all = super::diagnostic("PM_SVG_STRADDLE").is_some_and(|v| v == "all");
                            if e.len() >= 2 || (all && !e.is_empty()) {
                                let parts: Vec<String> = e.iter().map(|(k, c, cov)| format!("{k}:{}:{cov}", c.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(""))).collect();
                                println!("STRADDLE {i} {px} {py} {}", parts.join(";"));
                            }
                        }
                    }
                }
            }
        }
    }
    if super::diagnostic("PM_SVG_DEBUG").is_some() {
        eprintln!("render device origin={:?} size={}x{} draws={} alpha={} filter={}", dev.origin, dev.width, dev.height, dev.draws.len(), dev.alpha, dev.filter.is_some());
        if let Some(probe) = super::diagnostic("PM_SVG_PROBE").and_then(|p| {
            let (x, y) = p.split_once(',')?;
            Some((x.trim().parse::<u32>().ok()?, y.trim().parse::<u32>().ok()?))
        }) {
            let msaa = dev.draws.iter().any(needs_msaa);
            let pattern: &[[f64; 2]] = if msaa { &SAMPLES_4X } else { &SAMPLES_1X };
            for (i, d) in dev.draws.iter().enumerate() {
                eprintln!("  probe draw {i} at ({},{}) msaa={msaa}", probe.0, probe.1);
                match &d.geom {
                    Geom::Analytic(inst) => {
                        eprintln!("    instance {inst:?}");
                        let e = analytic::explain(inst, probe.0, probe.1, pattern);
                        eprintln!("    coverage per sample {e:?}");
                    }
                    Geom::Polygons { polys, rule, .. } => {
                        let mut m = SampleMask::new(dev.width, dev.height, pattern);
                        fill_polygons(&mut m, polys, *rule);
                        eprintln!("    polygon sample bits {:04b}", m.bits[(probe.1 * dev.width + probe.0) as usize]);
                    }
                    Geom::Triangles(tris) => {
                        let mut m = SampleMask::new(dev.width, dev.height, pattern);
                        fill_triangles(&mut m, tris);
                        eprintln!("    triangle sample bits {:04b}", m.bits[(probe.1 * dev.width + probe.0) as usize]);
                    }
                }
                if let Some(a) = &d.clip.analytic {
                    eprintln!("    analytic clip coverage {}", analytic_clip_coverage(a, [probe.0 as f32 + 0.5, probe.1 as f32 + 0.5]));
                }
            }
        }
        for d in &dev.draws {
            let g = match &d.geom {
                Geom::Polygons { polys, rule, msaa } => format!("Polygons n={} rule={rule:?} msaa={msaa}", polys.len()),
                Geom::Triangles(t) => format!("Triangles {}", t.len()),
                Geom::Analytic(_) => "Analytic".into(),
            };
            let g = match &d.paint {
                Paint::Image(i) => format!("{g} image origin={:?} {}x{} alpha={}", i.origin, i.width, i.height, i.alpha),
                _ => g,
            };
            eprintln!("  {g} scissor={:?} analytic_clip={} z={} order={} depth_only={} inverse={} bounds={:?}", d.clip.scissor, d.clip.analytic.is_some(), d.z, d.order, d.depth_only, d.inverse, d.clip.draw_bounds);
        }
    }
    let msaa = dev.draws.iter().any(needs_msaa);
    let pattern = if msaa { &SAMPLES_4X[..] } else { &SAMPLES_1X[..] };
    let mut target = MsaaTarget::new(dev.width.max(1), dev.height.max(1), pattern);
    if dev.width == 0 || dev.height == 0 {
        return Vec::new();
    }
    // `DrawPass::Make` sorts by painter's order; the depth test resolves
    // the rest. Depth-only clip draws precede the draws of their order.
    let mut draws: Vec<&Draw> = dev.draws.iter().collect();
    draws.sort_by_key(|d| (d.order, !d.depth_only, d.z));
    let mut depth = vec![0u32; (target.width * target.height) as usize * pattern.len()];
    for d in draws {
        render_draw(&mut target, &mut depth, d);
    }
    target.resolve()
}

fn render_draw(target: &mut MsaaTarget, depth: &mut [u32], d: &Draw) {
    let n = target.pattern.len();
    let (w, h) = (target.width as usize, target.height as usize);
    let scissor = [
        d.clip.scissor[0].max(0) as usize,
        d.clip.scissor[1].max(0) as usize,
        d.clip.scissor[2].min(w as i32).max(0) as usize,
        d.clip.scissor[3].min(h as i32).max(0) as usize,
    ];
    // Only the pixels the geometry can touch (an inverse fill: its draw
    // bounds) can pass; the rest of the scissor is skipped.
    let region = draw_region(d, w, h);
    let scissor = [scissor[0].max(region[0]), scissor[1].max(region[1]), scissor[2].min(region[2]), scissor[3].min(region[3])];
    if scissor[2] <= scissor[0] || scissor[3] <= scissor[1] {
        return;
    }
    // Coverage per sample.
    // Analytic draws carry their own per-sample coverage; only the
    // tessellated geometry needs the sample mask.
    let mut mask = match d.geom {
        Geom::Analytic(_) => SampleMask { width: 0, height: 0, pattern: target.pattern, bits: Vec::new() },
        _ => target.mask(),
    };
    let mut coverage: Option<analytic::Samples> = None;
    match &d.geom {
        Geom::Polygons { polys, rule, .. } => fill_polygons(&mut mask, polys, *rule),
        Geom::Triangles(tris) => fill_triangles(&mut mask, tris),
        Geom::Analytic(inst) => {
            let r = [scissor[0] as u32, scissor[1] as u32, scissor[2] as u32, scissor[3] as u32];
            coverage = Some(analytic::rasterize_region(inst, r, target.pattern));
        }
    }
    if d.inverse {
        // An inverse fill covers every sample of the draw bounds outside
        // the shape.
        let all = (1u8 << n) - 1;
        for b in &mut mask.bits {
            *b = !*b & all;
        }
    }
    let db = d.clip.draw_bounds;
    let pass: Option<SampleMask> = None;
    for py in scissor[1]..scissor[3] {
        for px in scissor[0]..scissor[2] {
            let p = py * w + px;
            // Which samples does the geometry cover?
            let mut covered = 0u8;
            for s in 0..n {
                let c = match &coverage {
                    Some(cov) => cov.get(px as u32, py as u32, s, n).is_some(),
                    None => mask.bits[p] & (1 << s) != 0,
                };
                if c {
                    covered |= 1 << s;
                }
            }
            if let Some(pm) = &pass {
                covered &= pm.bits[p];
            }
            if d.inverse && !rect_intersects(db, [px as f32, py as f32, px as f32 + 1.0, py as f32 + 1.0]) {
                continue;
            }
            // The depth test per sample: later draws (greater Z) pass and
            // write; a depth-only clip draw writes its Z outside its shape.
            let mut passed = 0u8;
            for s in 0..n {
                if covered & (1 << s) != 0 && d.z > depth[p * n + s] {
                    depth[p * n + s] = d.z;
                    passed |= 1 << s;
                }
            }
            covered = passed;
            if covered == 0 || d.depth_only {
                continue;
            }
            // The fragment: shader color at the pixel center, times the
            // analytic clip, times the renderer's coverage.
            let center = [px as f32 + 0.5, py as f32 + 0.5];
            let clip_cov = d.clip.analytic.as_ref().map_or(1.0, |a| analytic_clip_coverage(a, center));
            if clip_cov <= 0.0 {
                continue;
            }
            let color = match &d.paint {
                Paint::Image(img) => {
                    let ix = px as i32 - img.origin[0];
                    let iy = py as i32 - img.origin[1];
                    if ix < 0 || iy < 0 || ix >= img.width as i32 || iy >= img.height as i32 {
                        continue;
                    }
                    let i = (iy as usize * img.width as usize + ix as usize) * 4;
                    let s = &img.rgba[i..i + 4];
                    let a = img.alpha;
                    [f32::from(s[0]) / 255.0 * a, f32::from(s[1]) / 255.0 * a, f32::from(s[2]) / 255.0 * a, f32::from(s[3]) / 255.0 * a]
                }
                Paint::Solid(c) => *c,
                Paint::Gradient { shader, local_to_device, alpha, dither } => {
                    let local = local_to_device.inv.map(center);
                    gradient::evaluate(shader, local, *alpha, *dither, [px as u32, py as u32])
                }
            };
            for s in 0..n {
                if covered & (1 << s) == 0 {
                    continue;
                }
                let i = p * n + s;
                let cov = match &coverage {
                    Some(c) => c.get(px as u32, py as u32, s, n).unwrap_or(0.0),
                    None => 1.0,
                } * clip_cov;
                if cov <= 0.0 {
                    continue;
                }
                let src = color.map(|v| v * cov);
                let inv = 1.0 - src[3];
                let dst = &mut target.samples[i];
                for c in 0..4 {
                    dst[c] = unorm8(src[c] + f32::from(dst[c]) / 255.0 * inv);
                }
            }
        }
    }
}

/// The target pixels `[x0, y0, x1, y1)` a draw can cover: the geometry's
/// bounds plus a pixel (sub-pixel snapping), or an inverse fill's draw
/// bounds.
fn draw_region(d: &Draw, w: usize, h: usize) -> [usize; 4] {
    let from = |x0: f64, y0: f64, x1: f64, y1: f64| -> [usize; 4] {
        if x0 > x1 || y0 > y1 {
            return [0, 0, 0, 0];
        }
        let c = |v: f64, max: usize| v.clamp(0.0, max as f64) as usize;
        [c(x0.floor() - 1.0, w), c(y0.floor() - 1.0, h), c(x1.floor() + 2.0, w), c(y1.floor() + 2.0, h)]
    };
    if d.inverse {
        let b = d.clip.draw_bounds;
        return from(f64::from(b[0]), f64::from(b[1]), f64::from(b[2]), f64::from(b[3]));
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut add = |x: f64, y: f64| {
        if x.is_finite() && y.is_finite() {
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x);
            y1 = y1.max(y);
        }
    };
    match &d.geom {
        Geom::Polygons { polys, .. } => polys.iter().flatten().for_each(|p| add(p[0], p[1])),
        Geom::Triangles(tris) => tris.iter().flatten().for_each(|p| add(f64::from(p[0]), f64::from(p[1]))),
        Geom::Analytic(inst) => {
            let b = analytic::mesh_pixel_bounds(inst, w as u32, h as u32);
            return [b[0] as usize, b[1] as usize, b[2] as usize, b[3] as usize];
        }
    }
    if x0 == f64::MAX {
        return [0, 0, 0, 0];
    }
    from(x0, y0, x1, y1)
}

/// Convenience: the shape's device polygons (for tests).
pub fn polygons_of(path: &SkPath, m: &Mat) -> Vec<Vec<[f64; 2]>> {
    let local = tess::fill_polygons_local(path, m);
    tess::to_device(&local, m, 0, 0)
}

#[allow(dead_code)]
fn verbs_count(p: &SkPath) -> usize {
    p.verbs.iter().filter(|v| !matches!(v, Verb::Move | Verb::Close)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(c: [f32; 4]) -> Flags {
        Flags { color: c, shader: None, anti_alias: true, dither: false, stroke: None }
    }

    fn triangle() -> SkPath {
        let mut p = SkPath::new();
        p.move_to([2.0, 2.0]);
        p.line_to([30.0, 6.0]);
        p.line_to([12.0, 28.0]);
        p.close();
        p
    }

    /// (renderer, depth-only, analytic clip) of every draw of the root.
    fn kinds(c: &Canvas) -> Vec<(u8, bool, bool)> {
        c.devices[0]
            .draws
            .iter()
            .map(|d| {
                let g = match &d.geom {
                    Geom::Polygons { msaa: false, .. } => 0,
                    Geom::Polygons { msaa: true, .. } | Geom::Triangles(_) => 1,
                    Geom::Analytic(_) => 2,
                };
                (g, d.depth_only, d.clip.analytic.is_some())
            })
            .collect()
    }

    fn cases() -> Vec<(&'static str, Vec<Op>, bool)> {
        let red = solid([1.0, 0.0, 0.0, 1.0]);
        let blue = solid([0.0, 0.0, 1.0, 0.6]);
        let scale = Op::Concat(M44::affine(1.5, 0.0, 0.0, 1.5, 0.25, 0.0));
        vec![
            ("path and oval", vec![scale.clone(), Op::DrawPath { path: triangle(), rule: Rule::NonZero, flags: red.clone() }, Op::DrawOval { rect: [4.0, 4.0, 20.0, 14.0], flags: blue.clone() }], false),
            ("analytic only", vec![scale.clone(), Op::DrawOval { rect: [4.0, 4.0, 20.0, 14.0], flags: blue.clone() }, Op::DrawRect { rect: [1.3, 2.0, 9.0, 9.0], flags: red.clone() }], true),
            ("paths only", vec![scale.clone(), Op::DrawPath { path: triangle(), rule: Rule::EvenOdd, flags: red.clone() }, Op::DrawPath { path: triangle(), rule: Rule::NonZero, flags: blue.clone() }], true),
            ("path clip", vec![Op::ClipPath { path: triangle(), rule: Rule::NonZero, aa: true }, Op::DrawRect { rect: [0.0, 0.0, 20.0, 20.0], flags: red.clone() }], false),
            ("oval clip over a path", vec![Op::ClipRRect { rrect: SkRRect::make_oval([3.0, 3.0, 25.0, 25.0]), aa: true }, Op::DrawPath { path: triangle(), rule: Rule::NonZero, flags: red.clone() }], false),
            (
                "layer of ovals under a path",
                vec![
                    Op::SaveLayerAlpha(0.5),
                    Op::DrawOval { rect: [4.0, 4.0, 20.0, 14.0], flags: blue.clone() },
                    Op::Restore,
                    Op::DrawPath { path: triangle(), rule: Rule::NonZero, flags: red.clone() },
                ],
                true,
            ),
        ]
    }

    /// The classifier gives every draw the renderer, depth-only flag and
    /// clip the raster gives it, and the same verdict.
    #[test]
    fn the_classifier_chooses_the_raster_renderers() {
        for (name, ops, ok) in cases() {
            let mut raster = Canvas::new(32, 32);
            raster.replay(&ops).unwrap();
            raster.finish();
            let mut class = Canvas::classifier(32, 32);
            class.replay(&ops).unwrap();
            assert!(class.finish().is_empty(), "{name}: the classifier rasterized");
            assert_eq!(kinds(&raster), kinds(&class), "{name}");
            assert_eq!(raster.verdict().is_ok(), ok, "{name}: {:?}", raster.verdict());
            assert_eq!(class.verdict().is_ok(), ok, "{name}: {:?}", class.verdict());
        }
    }

    /// Later draws cover earlier ones (painter's depth); a translucent draw
    /// blends over what is under it in premultiplied 8-bit.
    #[test]
    fn draws_composite_in_painter_order() {
        let ops = vec![
            Op::DrawRect { rect: [0.0, 0.0, 8.0, 8.0], flags: solid([1.0, 0.0, 0.0, 1.0]) },
            Op::DrawRect { rect: [4.0, 4.0, 12.0, 12.0], flags: solid([0.0, 0.0, 1.0, 1.0]) },
            Op::DrawRect { rect: [0.0, 10.0, 4.0, 14.0], flags: solid([0.0, 1.0, 0.0, 0.5]) },
        ];
        let mut c = Canvas::new(16, 16);
        c.replay(&ops).unwrap();
        let px = c.finish();
        let at = |x: usize, y: usize| &px[(y * 16 + x) * 4..(y * 16 + x) * 4 + 4];
        assert_eq!(at(2, 2), [255, 0, 0, 255]);
        assert_eq!(at(6, 6), [0, 0, 255, 255]);
        assert_eq!(at(2, 12), [0, 128, 0, 128]);
        assert_eq!(at(14, 2), [0, 0, 0, 0]);
    }

    /// An unaligned rect is the analytic renderer: its edge pixel carries
    /// the covered fraction.
    #[test]
    fn unaligned_rect_edges_are_fractional() {
        let ops = vec![Op::DrawRect { rect: [2.5, 0.0, 10.0, 8.0], flags: solid([1.0, 1.0, 1.0, 1.0]) }];
        let mut c = Canvas::new(12, 8);
        c.replay(&ops).unwrap();
        assert_eq!(kinds(&c), vec![(2, false, false)]);
        let px = c.finish();
        let a = |x: usize| px[(4 * 12 + x) * 4 + 3];
        assert_eq!(a(1), 0);
        assert!((120..=135).contains(&a(2)), "{}", a(2));
        assert_eq!(a(5), 255);
    }
}
