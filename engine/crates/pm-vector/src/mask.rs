//! Layer masks: parametric rectangles and ellipses (feather, rotation, add and
//! subtract, at most 8 on today's GPU path) and bezier path masks.
//!
//! A port of `applyMasks`, `applyPathMasks`, and `FRAG_MASK` in
//! `legacy/gl/compositor.ts` and `legacy/gl/shaders.ts`. Today a layer takes
//! one of two routes, and so does [`LayerMasks::new`]:
//!
//! - **Parametric** (no enabled mask has a path): the first 8 enabled masks
//!   are evaluated analytically per pixel in layer space. Feather is a
//!   `smoothstep` across the signed distance; add masks take the maximum,
//!   subtract masks multiply by the complement through `min`.
//! - **Path** (any enabled mask has a path): every enabled mask, parametric or
//!   not and with no count limit, is filled as a path in sequence onto a
//!   coverage canvas, each blurred by a Gaussian of standard deviation
//!   `feather · |world x axis| / 2` composition pixels, add masks drawn over
//!   and subtract masks erased. Rectangles keep their signed size here.

use kurbo::{Affine, BezPath, Point};

use crate::canvas::{CanvasPath, CanvasTransform, TransformOp};
use crate::js;
use crate::path::{self, PathValues};

/// Most parametric masks the GPU path evaluates per layer.
pub const MAX_PARAMETRIC_MASKS: usize = 8;

/// `m.shape`: anything other than `ellipse` is a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaskShape {
    Rect,
    Ellipse,
}

impl MaskShape {
    pub fn from_name(name: &str) -> Self {
        if name == "ellipse" {
            Self::Ellipse
        } else {
            Self::Rect
        }
    }
}

/// `m.mode`: anything other than `subtract` adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaskMode {
    Add,
    Subtract,
}

impl MaskMode {
    pub fn from_name(name: &str) -> Self {
        if name == "subtract" {
            Self::Subtract
        } else {
            Self::Add
        }
    }
}

/// One enabled mask's evaluated values (`m.on` is not false and `m.p` exists).
/// Numbers are as evaluated; the `Number(x) || 0` coercions happen here.
#[derive(Debug, Clone, PartialEq)]
pub struct MaskValues {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// Degrees.
    pub rotation: f64,
    pub feather: f64,
    pub shape: MaskShape,
    pub mode: MaskMode,
    /// `m.path` evaluated with `pathValues(.., 'mp.<id>')`.
    pub path: Option<PathValues>,
}

impl Default for MaskValues {
    fn default() -> Self {
        Self {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0,
            rotation: 0.0,
            feather: 0.0,
            shape: MaskShape::Rect,
            mode: MaskMode::Add,
            path: None,
        }
    }
}

/// One mask on the parametric GPU path, in layer pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParametricMask {
    pub center: Point,
    /// Absolute width and height.
    pub size: (f64, f64),
    /// Radians.
    pub rotation: f64,
    /// Non-negative, layer pixels.
    pub feather: f64,
    pub shape: MaskShape,
    pub mode: MaskMode,
}

impl ParametricMask {
    fn new(m: &MaskValues) -> Self {
        Self {
            center: Point::new(js::or0(m.x), js::or0(m.y)),
            size: (js::or0(m.w).abs(), js::or0(m.h).abs()),
            rotation: js::or0(m.rotation) * core::f64::consts::PI / 180.0,
            feather: js::max(0.0, js::or0(m.feather)),
            shape: m.shape,
            mode: m.mode,
        }
    }

    /// The mask outline in layer pixels (the zero level of its distance
    /// function), for drawing or level 2 coverage.
    pub fn outline(&self) -> BezPath {
        let mut ctx = CanvasPath::new();
        let (w, h) = self.size;
        match self.shape {
            MaskShape::Rect => ctx.rect(-w / 2.0, -h / 2.0, w, h),
            MaskShape::Ellipse => {
                let _ = ctx.ellipse(0.0, 0.0, w / 2.0, h / 2.0, 0.0, 0.0, core::f64::consts::TAU);
            }
        }
        let place = Affine::translate(self.center.to_vec2()) * path::rotate(self.rotation);
        place * ctx.finish()
    }
}

/// Composition pixels to layer pixels for the parametric route (`u_inv`):
/// the inverse of the layer's 2×3 world matrix, or `None` when its
/// determinant is not finite or below 1e-9, in which case today's compositor
/// skips the layer's masks entirely.
pub fn layer_inverse(world: [f64; 6]) -> Option<Affine> {
    let w = world;
    let det = w[0] * w[3] - w[1] * w[2];
    if !det.is_finite() || det.abs() < 1e-9 {
        return None;
    }
    Some(Affine::new([
        w[3] / det,
        -w[1] / det,
        -w[2] / det,
        w[0] / det,
        (-w[3] * w[4] + w[2] * w[5]) / det,
        (w[1] * w[4] - w[0] * w[5]) / det,
    ]))
}

/// The parametric route: up to 8 masks, evaluated per pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct ParametricMasks {
    pub masks: Vec<ParametricMask>,
    /// Whether any mask adds; coverage starts at 0 if so, else at 1.
    pub has_add: bool,
}

impl ParametricMasks {
    /// The shader's uniforms: `u_g[8]` (cx, cy, w, h) and `u_q[8]` (rotation
    /// radians, feather, shape 0 rect / 1 ellipse, mode 0 add / 1 subtract).
    pub fn uniforms(&self) -> ([f32; 32], [f32; 32]) {
        let mut g = [0f32; 32];
        let mut q = [0f32; 32];
        for (i, m) in self.masks.iter().enumerate() {
            let k = i * 4;
            g[k..k + 4].copy_from_slice(&[
                m.center.x as f32,
                m.center.y as f32,
                m.size.0 as f32,
                m.size.1 as f32,
            ]);
            q[k..k + 4].copy_from_slice(&[
                m.rotation as f32,
                m.feather as f32,
                if m.shape == MaskShape::Ellipse {
                    1.0
                } else {
                    0.0
                },
                if m.mode == MaskMode::Subtract {
                    1.0
                } else {
                    0.0
                },
            ]);
        }
        (g, q)
    }

    /// Coverage at a layer-space point, computed as `FRAG_MASK` does in
    /// single precision. A CPU reference for the GPU port.
    pub fn coverage(&self, layer: Point) -> f32 {
        let (g, q) = self.uniforms();
        let (lx, ly) = (layer.x as f32, layer.y as f32);
        let mut cov: f32 = if self.has_add { 0.0 } else { 1.0 };
        for i in 0..self.masks.len() {
            let (gx, gy, gw, gh) = (g[i * 4], g[i * 4 + 1], g[i * 4 + 2], g[i * 4 + 3]);
            let (r, feather, shape, mode) = (q[i * 4], q[i * 4 + 1], q[i * 4 + 2], q[i * 4 + 3]);
            if gw < 0.01 || gh < 0.01 {
                continue;
            }
            let (mut px, mut py) = (lx - gx, ly - gy);
            if r.abs() > 1e-4 {
                let (s, c) = (libm::sinf(r), libm::cosf(r));
                // GLSL `mat2(c, -s, s, c) * p` (column-major).
                (px, py) = (c * px + s * py, -s * px + c * py);
            }
            let (hx, hy) = ((gw * 0.5).max(0.001), (gh * 0.5).max(0.001));
            let d = if shape < 0.5 {
                let (dx, dy) = (px.abs() - hx, py.abs() - hy);
                let outside = libm::sqrtf(dx.max(0.0) * dx.max(0.0) + dy.max(0.0) * dy.max(0.0));
                outside + dx.max(dy).min(0.0)
            } else {
                let (ex, ey) = (px / hx, py / hy);
                (libm::sqrtf(ex * ex + ey * ey) - 1.0) * hx.min(hy)
            };
            let f = feather.max(0.0);
            let a = if f > 0.01 {
                1.0 - smoothstep(-f * 0.5, f * 0.5, d)
            } else if d <= 0.0 {
                1.0
            } else {
                0.0
            };
            cov = if mode < 0.5 {
                cov.max(a)
            } else {
                cov.min(1.0 - a)
            };
        }
        cov
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One mask on the path route, in its own space.
#[derive(Debug, Clone, PartialEq)]
pub struct PathMask {
    /// Filled with the nonzero rule.
    pub path: BezPath,
    /// The canvas calls that build `path` (`tracePath`, `ellipse`, or
    /// `rect`), for the Canvas2D emulation.
    pub outline: MaskOutline,
    /// The canvas calls placing the mask in layer pixels: `translate(x, y)`,
    /// `rotate(rotation)`, and for a bezier mask its path's own
    /// `translate`, `rotate`, and `scale`.
    pub ops: Vec<TransformOp>,
    /// `ops` applied to the identity: mask space to layer pixels.
    pub transform: Affine,
    pub mode: MaskMode,
    /// `max(0, feather)`, layer units.
    pub feather: f64,
}

impl PathMask {
    /// Mask space to composition pixels for a layer whose world matrix is
    /// `world`, composed call by call as today's coverage canvas does.
    pub fn transform_from(&self, world: [f64; 6]) -> CanvasTransform {
        let mut t = CanvasTransform::IDENTITY;
        t.transform(world);
        t.apply_all(&self.ops);
        t
    }

    /// Gaussian blur standard deviation in composition pixels for a layer
    /// whose world matrix is `world` (CSS `blur(feather · hypot(a, b) / 2 px)`).
    pub fn blur_sigma(&self, world: Affine) -> f64 {
        let c = world.as_coeffs();
        self.feather * js::hypot(c[0], c[1]) / 2.0
    }
}

/// How a mask on the path route builds its outline on the canvas.
#[derive(Debug, Clone, PartialEq)]
pub enum MaskOutline {
    /// `tracePath(c, {...v, closed: true, trimStart: 0, trimEnd: 100})`:
    /// the traced path (`path`).
    Path,
    /// `c.ellipse(0, 0, |w| / 2, |h| / 2, 0, 0, 2π)`.
    Ellipse { rx: f64, ry: f64 },
    /// `c.rect(-w / 2, -h / 2, w, h)` (signed size).
    Rect { w: f64, h: f64 },
}

/// The path route: every enabled mask, drawn in order.
#[derive(Debug, Clone, PartialEq)]
pub struct PathMasks {
    /// Whether coverage starts full (no mask adds).
    pub start_covered: bool,
    pub masks: Vec<PathMask>,
}

/// A layer's masks on the route today's compositor takes.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerMasks {
    None,
    Parametric(ParametricMasks),
    Path(PathMasks),
}

impl LayerMasks {
    /// Masks in layer order, already filtered to enabled masks with values.
    pub fn new(masks: &[MaskValues]) -> Self {
        if masks.is_empty() {
            return Self::None;
        }
        if masks.iter().any(|m| m.path.is_some()) {
            return Self::Path(PathMasks {
                start_covered: !masks.iter().any(|m| m.mode != MaskMode::Subtract),
                masks: masks.iter().map(path_mask).collect(),
            });
        }
        let masks: Vec<ParametricMask> = masks
            .iter()
            .take(MAX_PARAMETRIC_MASKS)
            .map(ParametricMask::new)
            .collect();
        let has_add = masks.iter().any(|m| m.mode == MaskMode::Add);
        Self::Parametric(ParametricMasks { masks, has_add })
    }
}

fn path_mask(m: &MaskValues) -> PathMask {
    let (x, y, w, h) = (js::or0(m.x), js::or0(m.y), js::or0(m.w), js::or0(m.h));
    let mut ops = vec![
        TransformOp::Translate(x, y),
        TransformOp::Rotate(js::or0(m.rotation) * core::f64::consts::PI / 180.0),
    ];
    let mut outline = MaskOutline::Path;
    let path = match &m.path {
        Some(v) => {
            ops.extend([
                TransformOp::Translate(v.x, v.y),
                TransformOp::Rotate(js::radians(v.rotation)),
                TransformOp::Scale(v.scale_x / 100.0, v.scale_y / 100.0),
            ]);
            path::trace_path(&PathValues {
                closed: true,
                trim_start: 0.0,
                trim_end: 100.0,
                ..v.clone()
            })
        }
        None => {
            let mut ctx = CanvasPath::new();
            match m.shape {
                // Absolute radii never throw.
                MaskShape::Ellipse => {
                    outline = MaskOutline::Ellipse {
                        rx: w.abs() / 2.0,
                        ry: h.abs() / 2.0,
                    };
                    let _ = ctx.ellipse(
                        0.0,
                        0.0,
                        w.abs() / 2.0,
                        h.abs() / 2.0,
                        0.0,
                        0.0,
                        core::f64::consts::TAU,
                    );
                }
                MaskShape::Rect => {
                    outline = MaskOutline::Rect { w, h };
                    ctx.rect(-w / 2.0, -h / 2.0, w, h)
                }
            }
            ctx.finish()
        }
    };
    PathMask {
        path,
        outline,
        transform: CanvasTransform::of(&ops).affine(),
        ops,
        mode: m.mode,
        feather: js::max(0.0, js::or0(m.feather)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::Vertex;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> MaskValues {
        MaskValues {
            x,
            y,
            w,
            h,
            ..MaskValues::default()
        }
    }

    fn parametric(masks: &[MaskValues]) -> ParametricMasks {
        match LayerMasks::new(masks) {
            LayerMasks::Parametric(p) => p,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hard_rect_is_inside_or_outside() {
        let p = parametric(&[rect(50.0, 50.0, 20.0, 10.0)]);
        assert_eq!(p.coverage(Point::new(50.0, 50.0)), 1.0);
        assert_eq!(p.coverage(Point::new(59.0, 54.0)), 1.0);
        assert_eq!(p.coverage(Point::new(61.0, 50.0)), 0.0);
    }

    #[test]
    fn feather_is_half_at_the_edge() {
        let p = parametric(&[MaskValues {
            feather: 10.0,
            ..rect(0.0, 0.0, 100.0, 100.0)
        }]);
        assert!((p.coverage(Point::new(50.0, 0.0)) - 0.5).abs() < 1e-6);
        assert_eq!(p.coverage(Point::new(44.0, 0.0)), 1.0);
        assert_eq!(p.coverage(Point::new(56.0, 0.0)), 0.0);
    }

    #[test]
    fn subtract_only_starts_covered() {
        let p = parametric(&[MaskValues {
            mode: MaskMode::Subtract,
            ..rect(0.0, 0.0, 10.0, 10.0)
        }]);
        assert!(!p.has_add);
        assert_eq!(p.coverage(Point::new(0.0, 0.0)), 0.0);
        assert_eq!(p.coverage(Point::new(20.0, 0.0)), 1.0);
    }

    #[test]
    fn rotation_turns_the_mask_clockwise_in_y_down_space() {
        let p = parametric(&[MaskValues {
            rotation: 90.0,
            ..rect(0.0, 0.0, 40.0, 4.0)
        }]);
        assert_eq!(p.coverage(Point::new(0.0, 15.0)), 1.0);
        assert_eq!(p.coverage(Point::new(15.0, 0.0)), 0.0);
    }

    #[test]
    fn ellipse_distance_is_scaled_by_the_short_radius() {
        let p = parametric(&[MaskValues {
            shape: MaskShape::Ellipse,
            ..rect(0.0, 0.0, 40.0, 20.0)
        }]);
        assert_eq!(p.coverage(Point::new(19.0, 0.0)), 1.0);
        assert_eq!(p.coverage(Point::new(0.0, 11.0)), 0.0);
    }

    #[test]
    fn layer_inverse_inverts_or_skips_singular_worlds() {
        let world = [2.0, 0.0, 0.0, 4.0, 10.0, 20.0];
        let inv = layer_inverse(world).unwrap();
        assert_eq!(inv * Point::new(30.0, 60.0), Point::new(10.0, 10.0));
        assert!(layer_inverse([1e-5, 0.0, 0.0, 1e-5, 0.0, 0.0]).is_none());
        assert!(layer_inverse([f64::NAN, 0.0, 0.0, 1.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn only_eight_masks_reach_the_gpu() {
        let many: Vec<_> = (0..12).map(|i| rect(f64::from(i), 0.0, 1.0, 1.0)).collect();
        assert_eq!(parametric(&many).masks.len(), MAX_PARAMETRIC_MASKS);
    }

    #[test]
    fn tiny_masks_are_skipped() {
        let p = parametric(&[rect(0.0, 0.0, 0.005, 10.0)]);
        assert_eq!(p.coverage(Point::new(0.0, 0.0)), 0.0);
    }

    #[test]
    fn any_path_mask_moves_every_mask_to_the_path_route() {
        let shape = PathValues {
            vertices: vec![
                Vertex::corner(0.0, 0.0),
                Vertex::corner(10.0, 0.0),
                Vertex::corner(0.0, 10.0),
            ],
            trim_end: 20.0,
            ..PathValues::default()
        };
        let mut masks: Vec<_> = (0..9).map(|_| rect(0.0, 0.0, -10.0, 10.0)).collect();
        masks.push(MaskValues {
            path: Some(shape),
            mode: MaskMode::Subtract,
            feather: 4.0,
            ..MaskValues::default()
        });
        let LayerMasks::Path(p) = LayerMasks::new(&masks) else {
            panic!()
        };
        assert_eq!(p.masks.len(), 10);
        assert!(!p.start_covered);
        // Trim is ignored and the path is closed.
        assert!(matches!(
            p.masks[9].path.elements().last(),
            Some(kurbo::PathEl::ClosePath)
        ));
        // Signed rectangle size is kept on this route.
        use kurbo::Shape;
        assert_eq!(
            p.masks[0].path.bounding_box(),
            kurbo::Rect::new(-5.0, -5.0, 5.0, 5.0)
        );
        assert_eq!(p.masks[9].blur_sigma(Affine::scale(2.0)), 4.0);
    }
}
