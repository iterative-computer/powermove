//! 2.5D layers (plan section 8, "2.5D").
//!
//! Flat layers with the 3D switch (or inside a 3D group) share a
//! perspective space (`legacy/core/space-3d.ts`; `pm_eval::space3d`):
//! Z position, anchor and scale with their Z components, X and Y rotation,
//! orientation, and the lens of the outermost 3D group (or the layer's own
//! `perspective`). Today's compositor draws such a layer's content quad
//! through the plane's homography (`drawContent`: `planeMatrix` scaled to
//! output pixels, with the quad's size and anchor folded in), as a
//! projective `mat3` whose third row is the perspective divide, so the
//! rasterizer clips the part behind the camera (w ≤ 0) and interpolates
//! texture coordinates perspective-correctly. Within each run of 3D layers
//! the pass is reordered back to front: coplanar layers form one surface
//! that keeps timeline order (coplanar ordering), surfaces sort by the
//! depth of their first layer's anchor (painter sort), and 2D layers and
//! 3D object blocks are barriers (`compositionOrderedLayers`,
//! `depthOrderedLayers`).
//!
//! This module computes the projected matrices and the pass order; the
//! executor draws them with the same vertex shader as every quad.

use pm_eval::{JsResult, Scope};
use pm_model::Node;

pub(crate) mod local;

/// Layer types drawn as 2.5D planes and compared with today's (groups
/// supply the space). Shapes and text keep today's raster density (the 2D
/// world matrix's) and draw through the plane; 3D object layers draw in
/// screen space, as today; adjustment layers map their masks through the
/// plane; level 2 content rasterizes in layer space at the plane's density.
pub const DRAWN_TYPES: [&str; 11] = [
    "solid",
    "image",
    "video",
    "precomp",
    "group",
    "shape",
    "text",
    "captions",
    "shader",
    "extension",
    "adjustment",
];

/// A unit quad to target pixels: column-major 3×3 (today's `projected`
/// `Float32Array`), applied to `(u, v, 1)`.
pub type Plane = [f64; 9];

/// `compositionOrderedLayers(PM, pass, T)` for a pass in stack order (top
/// first): runs of 2.5D layers depth ordered, 3D object blocks kept in
/// stack order (the 3D scene pass orders the models it draws).
pub fn order<'d>(
    scope: &mut Scope<'_, 'd>,
    pass: Vec<&'d Node>,
    time: f64,
) -> JsResult<Vec<&'d Node>> {
    scope.pick_order(&pass, time)
}

/// The homography of a 2.5D layer's content quad in target pixels, or
/// `None` for a flat layer: `planeMatrix(PM, L, T)` with today's
/// `outputScale` (`scale`) and the quad's size `w × h` and anchor `(ax,
/// ay)`, in the compositor's expression order.
#[allow(clippy::too_many_arguments)]
pub fn projected<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    w: f64,
    h: f64,
    ax: f64,
    ay: f64,
) -> JsResult<Option<Plane>> {
    if !scope.is_3d_layer(layer)? {
        return Ok(None);
    }
    let m = scope.plane_matrix(layer, time, false)?;
    Ok(Some(fold(&m, scale, w, h, ax, ay)))
}

/// `drawContent`'s `projected` from a plane matrix `h` (layer pixels to
/// composition pixels).
pub fn fold(h: &[f64; 9], scale: (f64, f64), w: f64, hh: f64, ax: f64, ay: f64) -> Plane {
    let (sx, sy) = scale;
    [
        h[0] * w * sx,
        h[1] * w * sy,
        h[2] * w,
        h[3] * hh * sx,
        h[4] * hh * sy,
        h[5] * hh,
        (h[6] - h[0] * ax - h[3] * ay) * sx,
        (h[7] - h[1] * ax - h[4] * ay) * sy,
        h[8] - h[2] * ax - h[5] * ay,
    ]
}

/// A point of the unit quad through a plane, or `None` behind the camera
/// (w ≤ 0).
pub fn project(p: &Plane, u: f64, v: f64) -> Option<[f64; 2]> {
    let w = p[2] * u + p[5] * v + p[8];
    if w <= 0.0 || !w.is_finite() {
        return None;
    }
    Some([
        (p[0] * u + p[3] * v + p[6]) / w,
        (p[1] * u + p[4] * v + p[7]) / w,
    ])
}

/// The largest output-pixel density of a plane over its corners: output
/// pixels per unit of `u` and of `v` (the columns of the Jacobian of the
/// projection), for continuous rasterization. `None` when a corner is
/// behind the camera.
pub fn corner_density(p: &Plane) -> Option<(f64, f64)> {
    let mut du = 0.0f64;
    let mut dv = 0.0f64;
    for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
        let w = p[2] * u + p[5] * v + p[8];
        if w <= 0.0 || !w.is_finite() {
            return None;
        }
        let x = p[0] * u + p[3] * v + p[6];
        let y = p[1] * u + p[4] * v + p[7];
        // d(x/w)/du = (p0·w − x·p2) / w²; likewise for v.
        let jxu = (p[0] * w - x * p[2]) / (w * w);
        let jyu = (p[1] * w - y * p[2]) / (w * w);
        let jxv = (p[3] * w - x * p[5]) / (w * w);
        let jyv = (p[4] * w - y * p[5]) / (w * w);
        du = du.max(jxu.hypot(jyu));
        dv = dv.max(jxv.hypot(jyv));
    }
    Some((du, dv))
}

/// The quad's corners in target pixels, `(0,0) (1,0) (1,1) (0,1)`, for
/// overlay geometry; a corner behind the camera reports its direction's
/// far point clipped at w = 1e-6.
pub fn corners(p: &Plane) -> [[f64; 2]; 4] {
    let at = |u: f64, v: f64| {
        let w = (p[2] * u + p[5] * v + p[8]).max(1e-6);
        [
            (p[0] * u + p[3] * v + p[6]) / w,
            (p[1] * u + p[4] * v + p[7]) / w,
        ]
    };
    [at(0.0, 0.0), at(1.0, 0.0), at(1.0, 1.0), at(0.0, 1.0)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_plane_folds_into_the_affine_quad() {
        // A 2D world matrix [a b c d e f] as a plane, scaled 2×.
        let h = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 30.0, 40.0, 1.0];
        let p = fold(&h, (2.0, 2.0), 100.0, 50.0, 50.0, 25.0);
        assert_eq!(project(&p, 0.0, 0.0), Some([-40.0, 30.0]));
        assert_eq!(project(&p, 1.0, 1.0), Some([160.0, 130.0]));
        assert_eq!(corner_density(&p), Some((200.0, 100.0)));
    }

    #[test]
    fn perspective_density_is_largest_at_the_near_edge() {
        // w grows with v: the v = 0 edge is nearer and denser.
        let p = [100.0, 0.0, 0.0, 0.0, 100.0, 1.0, 0.0, 0.0, 1.0];
        let (du, _) = corner_density(&p).unwrap();
        assert!((du - 100.0).abs() < 1e-12);
        assert!(project(&[1.0, 0.0, 0.0, 0.0, 1.0, -2.0, 0.0, 0.0, 1.0], 0.0, 1.0).is_none());
        assert!(corner_density(&[1.0, 0.0, 0.0, 0.0, 1.0, -2.0, 0.0, 0.0, 1.0]).is_none());
    }
}
