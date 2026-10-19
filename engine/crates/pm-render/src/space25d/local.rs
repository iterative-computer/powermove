//! Level 2 content on a 2.5D plane (plan section 8, "continuous
//! rasterization"; amendment 10).
//!
//! Level 2 vectors, solids, SVG images, and text draw at output resolution
//! through the layer's 2D world matrix. A 3D layer has no affine placement:
//!
//! - coverage scenes (solids, shapes, editable paths, plain SVG images) are
//!   flattened in layer space finely enough for the plane's density and
//!   every vertex is projected through the plane's homography, clipped at
//!   the camera plane ([`project_scene`]); a projective map keeps straight
//!   segments straight, so the result is the exact projected outline,
//!   covered analytically at output resolution;
//! - resvg rasters and text are rasterized in layer space at the density the
//!   plane draws them at (the densest corner of their bounds on the plane,
//!   as a level 2 precomp is) and drawn through the plane's homography with
//!   the level 2 resampler (`crate::sampling`).
//!
//! Level 1 content keeps today's raster density and bilinear plane draw.

use pm_eval::{JsResult, Scope};
use pm_model::Node;

/// The largest edge of a layer-space raster (`crate::sampling::MAX_TARGET`).
pub const MAX_RASTER: f64 = crate::sampling::MAX_TARGET;

/// A clip that does not clip: layer-space rasters are bounded by their
/// content, not by the frame.
pub fn unbounded() -> pm_vector::render::scene::PixelRect {
    pm_vector::render::scene::PixelRect::new(-(1 << 29), -(1 << 29), 1 << 30, 1 << 30)
}

/// Output pixels per content unit of a level 2 layer's layer-space raster,
/// per axis: the densest corner of `bounds` (`[x, y, w, h]` in content
/// units, before the size anchor offset) on the layer's plane, never below
/// the output scale, and capped so the raster stays within
/// [`MAX_RASTER`]. A plane with a corner behind the camera uses the output
/// scale.
pub fn density<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
    bounds: [f64; 4],
) -> JsResult<(f64, f64)> {
    let [x, y, w, h] = bounds;
    let (w, h) = (w.max(1e-9), h.max(1e-9));
    let offset = scope.size_anchor_offset(layer, time)?;
    let corner = match super::projected(
        scope,
        layer,
        time,
        scale,
        w,
        h,
        -x - offset.x,
        -y - offset.y,
    )? {
        Some(plane) => super::corner_density(&plane).map(|(du, dv)| (du / w, dv / h)),
        None => None,
    };
    let (du, dv) = corner.unwrap_or(scale);
    let cap = |d: f64, s: f64, size: f64| {
        let d = if d.is_finite() { d.max(s) } else { s };
        d.min(MAX_RASTER / size.max(1.0)).max(1e-6)
    };
    Ok((cap(du, scale.0, w), cap(dv, scale.1, h)))
}

/// The plane of a layer's content coordinates (before the size anchor
/// offset) in target pixels, or `None` for a flat layer.
pub fn content_plane<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
    scale: (f64, f64),
) -> JsResult<Option<super::Plane>> {
    let offset = scope.size_anchor_offset(layer, time)?;
    super::projected(scope, layer, time, scale, 1.0, 1.0, -offset.x, -offset.y)
}

/// A coverage scene placed in content coordinates (identity world),
/// projected through `plane` into target pixels: every item's area outline
/// (strokes expanded in layer space) flattened within the coverage
/// tolerance at `density` (target pixels per content unit at the densest
/// point), each vertex mapped through the homography, and each closed
/// contour clipped at the camera plane (w ≥ 10⁻⁶) in homogeneous
/// coordinates. Items keep their order, paint, and fill rule.
pub fn project_scene(
    scene: &pm_vector::render::scene::CoverageScene,
    plane: &super::Plane,
    density: f64,
) -> pm_vector::render::scene::CoverageScene {
    use pm_vector::kurbo::{Affine, BezPath, PathEl, Point};
    use pm_vector::render::scene::{CoverageItem, ItemStyle};
    let mut out = pm_vector::render::scene::CoverageScene::default();
    for item in &scene.items {
        // The outline in content units at the finer tolerance: scale the
        // item up by the density, take its device outline, and scale back.
        let up = Affine::scale(density.max(1e-6));
        let scaled = CoverageItem {
            transform: up * item.transform,
            ..item.clone()
        };
        let (outline, rule) = scaled.device_outline();
        let down = up.inverse();
        let mut contours: Vec<Vec<[f64; 3]>> = Vec::new();
        for el in outline.iter() {
            let homogeneous = |p: Point| {
                let q = down * p;
                [
                    plane[0] * q.x + plane[3] * q.y + plane[6],
                    plane[1] * q.x + plane[4] * q.y + plane[7],
                    plane[2] * q.x + plane[5] * q.y + plane[8],
                ]
            };
            match el {
                PathEl::MoveTo(p) => contours.push(vec![homogeneous(p)]),
                PathEl::LineTo(p) => {
                    if let Some(c) = contours.last_mut() {
                        c.push(homogeneous(p));
                    }
                }
                PathEl::ClosePath => {}
                // `device_outline` is flattened.
                PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
            }
        }
        let mut path = BezPath::new();
        for c in contours {
            let clipped = clip_near(&c);
            let mut points = clipped.iter().map(|[x, y, w]| Point::new(x / w, y / w));
            if let Some(first) = points.next() {
                path.move_to(first);
                for p in points {
                    path.line_to(p);
                }
                path.close_path();
            }
        }
        if path.elements().is_empty() {
            continue;
        }
        out.items.push(CoverageItem {
            path,
            transform: Affine::IDENTITY,
            style: ItemStyle::Fill(rule),
            color: item.color,
        });
    }
    out
}

/// Sutherland–Hodgman against the plane w = 10⁻⁶ (the part in front of the
/// camera) of a closed contour in homogeneous coordinates.
fn clip_near(contour: &[[f64; 3]]) -> Vec<[f64; 3]> {
    const NEAR: f64 = 1e-6;
    let mut out = Vec::with_capacity(contour.len() + 2);
    let n = contour.len();
    for i in 0..n {
        let a = contour[i];
        let b = contour[(i + 1) % n];
        let (ina, inb) = (a[2] >= NEAR, b[2] >= NEAR);
        if ina {
            out.push(a);
        }
        if ina != inb {
            let t = (NEAR - a[2]) / (b[2] - a[2]);
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, NEAR]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn contours_are_clipped_at_the_camera_plane() {
        let c = [
            [0.0, 0.0, 1.0],
            [1.0, 0.0, -1.0],
            [1.0, 1.0, -1.0],
            [0.0, 1.0, 1.0],
        ];
        let clipped = super::clip_near(&c);
        assert_eq!(clipped.len(), 4);
        assert!(clipped.iter().all(|p| p[2] >= 1e-6));
    }

    #[test]
    fn a_flat_plane_projects_a_scene_onto_itself() {
        use pm_vector::kurbo::{Affine, BezPath, Rect, Shape};
        use pm_vector::render::scene::{CoverageItem, CoverageScene, FillRule, ItemStyle};
        let scene = CoverageScene {
            items: vec![CoverageItem {
                path: Rect::new(0.0, 0.0, 10.0, 5.0).to_path(0.1),
                transform: Affine::IDENTITY,
                style: ItemStyle::Fill(FillRule::NonZero),
                color: [1.0; 4],
            }],
        };
        // Translate by (20, 30) and scale by 2.
        let plane = [2.0, 0.0, 0.0, 0.0, 2.0, 0.0, 20.0, 30.0, 1.0];
        let p = super::project_scene(&scene, &plane, 2.0);
        let b = p.items[0].device_bounds().unwrap();
        assert_eq!((b.x0, b.y0, b.x1, b.y1), (20.0, 30.0, 40.0, 40.0));
        let _ = BezPath::new();
    }

    #[test]
    fn the_unbounded_clip_contains_negative_coordinates() {
        let r = super::unbounded();
        assert!(r.x < -1_000_000 && r.right() > 1_000_000);
    }
}
