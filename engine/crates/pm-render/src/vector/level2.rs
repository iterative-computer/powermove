//! `compat` level 2 vector content: geometry at output resolution with
//! analytic area coverage (plan section 8, "Sampling and antialiasing";
//! section 10: shapes at output resolution, solid `radius` honored).
//!
//! The scene is the shape's or paths' exact geometry under the layer's world
//! transform; coverage comes from Vello (or `vello_cpu`), and the engine's
//! float paint pass composites each item ([`super::cache`]).

use pm_vector::kurbo::{Affine, RoundedRect, Shape};
use pm_eval::transform::Mat2d;
use pm_vector::canvas2d::shape::{PathColors, ShapeColors};
use pm_vector::path::PathLayer;
use pm_vector::render::scene::{CoverageItem, CoverageScene, FillRule, ItemStyle};
use pm_vector::shape::{ShapeContent, shape_geometry};

fn affine(m: &Mat2d) -> Affine {
    Affine::new(*m)
}

/// A shape's fill and stroke at output resolution: shape space (origin at
/// the box's top-left corner) centered on the layer origin, then `world`.
/// `None` when the content draws nothing (a negative radius makes today's
/// raster throw).
pub fn shape_scene(d: &ShapeContent, colors: &ShapeColors, world: &Mat2d) -> Option<CoverageScene> {
    let g = shape_geometry(d).ok()?;
    let to_layer = Affine::translate((-d.w / 2.0, -d.h / 2.0));
    let mut scene = CoverageScene::default();
    scene.push_shape(&g, affine(world) * to_layer, &colors.color, &colors.stroke_color);
    Some(scene)
}

/// A layer's editable paths at output resolution.
pub fn paths_scene(layer: &PathLayer, colors: &[PathColors], world: &Mat2d) -> CoverageScene {
    let fills: Vec<String> = colors.iter().map(|c| c.fill.clone()).collect();
    let strokes: Vec<String> = colors.iter().map(|c| c.stroke.clone()).collect();
    let mut scene = CoverageScene::default();
    scene.push_paths(layer, affine(world), &fills, &strokes);
    scene
}

/// A level 2 solid with `radius`: a rounded rectangle (`min(r, w/2, h/2)`
/// circular corners) filled with the solid's color, its box from (0, 0) as
/// today's solid quad.
pub fn solid_scene(w: f64, h: f64, radius: f64, rgba: [f32; 4], world: &Mat2d) -> CoverageScene {
    let r = radius.min(w.abs() / 2.0).min(h.abs() / 2.0).max(0.0);
    let path = RoundedRect::new(0.0, 0.0, w, h, r).to_path(1e-3);
    CoverageScene {
        items: vec![CoverageItem {
            path,
            transform: affine(world),
            style: ItemStyle::Fill(FillRule::NonZero),
            color: rgba,
        }],
    }
}
