//! Vector layers (plan section 10; design notes in docs/engine/vector.md).
//!
//! Shapes and editable paths are drawn at output resolution: their exact
//! geometry under the layer's world transform becomes a [`CoverageScene`],
//! covered exactly by the GPU and painted in float
//! ([`cache`]). SVG images are drawn at output resolution too ([`svg`]).
//! Solids are the composite pass's analytic quad (`fs_solid`).

pub mod cache;
pub mod svg;
pub mod svg_float;

use pm_eval::transform::Mat2d;
use pm_eval::{JsResult, JsValue, Scope};
use pm_model::Node;
use pm_vector::kurbo::Affine;
use pm_vector::path::{PathColors, PathLayer, PathNode};
use pm_vector::render::scene::{CoverageScene, PixelRect};
use pm_vector::shape::{ShapeColors, ShapeContent, ShapeKind, shape_geometry};
use sha2::{Digest, Sha256};

use crate::js;

/// Vector content the executor resolves to a texture.
#[derive(Debug, Clone, PartialEq)]
pub enum VectorSource {
    /// A coverage scene in target pixels, rendered into `rect`.
    Scene {
        scene: CoverageScene,
        rect: PixelRect,
    },
    /// An SVG image under `transform` (user units to target pixels) drawn
    /// into `rect`, clipped to the polygon `clip` (float GPU paint, or resvg).
    SvgRaster {
        path: std::path::PathBuf,
        stamp: svg::Stamp,
        transform: [f64; 6],
        rect: PixelRect,
        clip: [[f64; 2]; 4],
    },
}

impl VectorSource {
    /// A content key over every input that changes the pixels (section 8).
    pub fn key(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"pm-vector/1");
        match self {
            VectorSource::Scene { scene, rect } => {
                h.update(b"scene");
                h.update(format!("{scene:?}{rect:?}").as_bytes());
            }
            VectorSource::SvgRaster {
                path,
                stamp,
                transform,
                rect,
                clip,
            } => {
                h.update(b"svg");
                h.update(path.to_string_lossy().as_bytes());
                h.update([0]);
                h.update(stamp.0.to_le_bytes());
                h.update(stamp.1.to_le_bytes());
                for v in transform.iter().chain(clip.iter().flatten()) {
                    h.update(v.to_bits().to_le_bytes());
                }
                h.update(format!("{rect:?}").as_bytes());
            }
        }
        h.finalize().into()
    }
}

/// How a vector layer's content is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum VectorQuad {
    /// Pixels in target space covering `rect` exactly.
    Device {
        source: VectorSource,
        rect: PixelRect,
    },
}

/// A content color as the Canvas2D style setter receives it: strings as
/// they are, anything else through `String()`.
fn style_string(v: &JsValue) -> String {
    v.to_js_string().to_string_lossy()
}

fn number(d: &pm_eval::Content, key: &str) -> f64 {
    js::get(d, key).to_number()
}

/// The evaluated shape content as numbers and strings.
pub fn shape_content(d: &pm_eval::Content) -> (ShapeContent, ShapeColors) {
    let shape = match js::get(d, "shape") {
        JsValue::String(s) => ShapeKind::from_name(&s.to_string_lossy()),
        _ => ShapeKind::Rect,
    };
    let corner = |k: &str| match js::get(d, k) {
        JsValue::Undefined | JsValue::Null => None,
        v => Some(v.to_number()),
    };
    let content = ShapeContent {
        shape,
        w: number(d, "w"),
        h: number(d, "h"),
        radius: number(d, "radius"),
        independent_corners: js::get(d, "independentCorners").truthy(),
        corner_radii: [
            corner("radiusTL"),
            corner("radiusTR"),
            corner("radiusBR"),
            corner("radiusBL"),
        ],
        smoothing: number(d, "smoothing"),
        stroke: number(d, "stroke"),
        points: number(d, "points"),
    };
    let stroke_color = js::get(d, "strokeColor");
    let colors = ShapeColors {
        color: style_string(js::get(d, "color")),
        stroke_color: if stroke_color.truthy() {
            style_string(stroke_color)
        } else {
            String::new()
        },
    };
    (content, colors)
}

/// Whether a layer is `compat` level 2 content.
pub fn level2(layer: &Node) -> bool {
    pm_eval::node::member(Some(layer), "compat")
        .is_some_and(|c| pm_eval::node::to_number(Some(c)) >= 2.0)
}

/// Plans a shape layer's content at output resolution. `world` maps layer
/// units to target pixels; `target` is the target's pixel rectangle.
pub fn plan_shape<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    d: &pm_eval::Content,
    time: f64,
    world: &Mat2d,
    target: PixelRect,
) -> JsResult<Option<VectorQuad>> {
    let has_paths = pm_eval::node::array(pm_eval::node::member(
        pm_eval::node::member(Some(layer), "d"),
        "paths",
    ))
    .is_some_and(|p| !p.is_empty());
    let scene = if has_paths {
        let (_, colors, layer_paths) = paths(scope, layer, time)?;
        paths_scene(&layer_paths, &colors, world)
    } else {
        let (content, colors) = shape_content(d);
        match shape_scene(&content, &colors, world) {
            Some(s) => s,
            None => return Ok(None),
        }
    };
    Ok(scene.pixel_bounds(target).map(|rect| VectorQuad::Device {
        source: VectorSource::Scene { scene, rect },
        rect,
    }))
}

/// A shape's fill and stroke: shape space (origin at the box's top-left
/// corner) centered on the layer origin, then `world`. `None` when the
/// content draws nothing (a negative radius makes today's raster throw).
pub fn shape_scene(d: &ShapeContent, colors: &ShapeColors, world: &Mat2d) -> Option<CoverageScene> {
    let g = shape_geometry(d).ok()?;
    let to_layer = Affine::translate((-d.w / 2.0, -d.h / 2.0));
    let mut scene = CoverageScene::default();
    scene.push_shape(
        &g,
        Affine::new(*world) * to_layer,
        &colors.color,
        &colors.stroke_color,
    );
    Some(scene)
}

/// A layer's editable paths.
pub fn paths_scene(layer: &PathLayer, colors: &[PathColors], world: &Mat2d) -> CoverageScene {
    let fills: Vec<String> = colors.iter().map(|c| c.fill.clone()).collect();
    let strokes: Vec<String> = colors.iter().map(|c| c.stroke.clone()).collect();
    let mut scene = CoverageScene::default();
    scene.push_paths(layer, Affine::new(*world), &fills, &strokes);
    scene
}

/// A layer's editable paths: nodes, colors, and geometry.
pub fn paths<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    time: f64,
) -> JsResult<(Vec<PathNode>, Vec<PathColors>, PathLayer)> {
    let (evaluated, _, layer_paths) = scope.path_geometry(layer, time)?;
    let stored = pm_eval::node::array(pm_eval::node::member(
        pm_eval::node::member(Some(layer), "d"),
        "paths",
    ))
    .unwrap_or(&[]);
    let nodes = evaluated
        .iter()
        .zip(stored)
        .map(|(e, s)| PathNode {
            id: e.id.to_js_string().to_string_lossy(),
            parent: pm_eval::node::string(pm_eval::node::member(Some(s), "parent"))
                .map(|p| p.to_string_lossy()),
            values: e.path_values(),
        })
        .collect();
    let colors = evaluated
        .iter()
        .map(|e| {
            let get = |k: &str| {
                e.values
                    .get(&pm_model::JsString::from(k))
                    .cloned()
                    .unwrap_or(JsValue::Undefined)
            };
            PathColors {
                fill: style_string(&get("fill")),
                stroke: style_string(&get("stroke")),
            }
        })
        .collect();
    Ok((nodes, colors, layer_paths))
}
