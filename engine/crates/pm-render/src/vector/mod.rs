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
use pm_vector::kurbo::{Affine, Cap, Join, PathEl, Point};
use pm_vector::path::{PathColors, PathLayer, PathNode};
use pm_vector::render::scene::{CoverageScene, FillRule, ItemStyle, PixelRect};
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
                h.update(b"scene/2");
                hash_scene(&mut h, scene);
                hash_rect(&mut h, *rect);
            }
            VectorSource::SvgRaster {
                path,
                stamp,
                transform,
                rect,
                clip,
            } => {
                h.update(b"svg");
                hash_bytes(&mut h, path.as_os_str().as_encoded_bytes());
                h.update(stamp.0.to_le_bytes());
                h.update(stamp.1.to_le_bytes());
                for v in transform.iter().chain(clip.iter().flatten()) {
                    h.update(v.to_bits().to_le_bytes());
                }
                hash_rect(&mut h, *rect);
            }
        }
        h.finalize().into()
    }
}

fn hash_bytes(h: &mut Sha256, bytes: &[u8]) {
    h.update((bytes.len() as u64).to_le_bytes());
    h.update(bytes);
}

fn hash_f64(h: &mut Sha256, value: f64) {
    h.update(value.to_bits().to_le_bytes());
}

fn hash_point(h: &mut Sha256, point: Point) {
    hash_f64(h, point.x);
    hash_f64(h, point.y);
}

fn hash_rect(h: &mut Sha256, rect: PixelRect) {
    h.update(rect.x.to_le_bytes());
    h.update(rect.y.to_le_bytes());
    h.update(rect.width.to_le_bytes());
    h.update(rect.height.to_le_bytes());
}

fn hash_scene(h: &mut Sha256, scene: &CoverageScene) {
    h.update((scene.items.len() as u64).to_le_bytes());
    for item in &scene.items {
        h.update((item.path.elements().len() as u64).to_le_bytes());
        for element in item.path.elements() {
            match *element {
                PathEl::MoveTo(p) => {
                    h.update([0]);
                    hash_point(h, p);
                }
                PathEl::LineTo(p) => {
                    h.update([1]);
                    hash_point(h, p);
                }
                PathEl::QuadTo(p1, p2) => {
                    h.update([2]);
                    hash_point(h, p1);
                    hash_point(h, p2);
                }
                PathEl::CurveTo(p1, p2, p3) => {
                    h.update([3]);
                    hash_point(h, p1);
                    hash_point(h, p2);
                    hash_point(h, p3);
                }
                PathEl::ClosePath => h.update([4]),
            }
        }
        for coefficient in item.transform.as_coeffs() {
            hash_f64(h, coefficient);
        }
        match &item.style {
            ItemStyle::Fill(FillRule::NonZero) => h.update([0]),
            ItemStyle::Fill(FillRule::EvenOdd) => h.update([1]),
            ItemStyle::Stroke(stroke) => {
                h.update([2]);
                hash_f64(h, stroke.width);
                h.update([match stroke.join {
                    Join::Bevel => 0,
                    Join::Miter => 1,
                    Join::Round => 2,
                }]);
                hash_f64(h, stroke.miter_limit);
                for cap in [stroke.start_cap, stroke.end_cap] {
                    h.update([match cap {
                        Cap::Butt => 0,
                        Cap::Square => 1,
                        Cap::Round => 2,
                    }]);
                }
                h.update((stroke.dash_pattern.len() as u64).to_le_bytes());
                for &dash in &stroke.dash_pattern {
                    hash_f64(h, dash);
                }
                hash_f64(h, stroke.dash_offset);
            }
        }
        for color in item.color {
            h.update(color.to_bits().to_le_bytes());
        }
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

#[cfg(test)]
mod key_tests {
    use super::*;
    use pm_vector::kurbo::{BezPath, Stroke};
    use pm_vector::render::scene::CoverageItem;

    #[test]
    fn scene_key_includes_every_path_style_paint_transform_and_rect_field() {
        let mut path = BezPath::new();
        path.move_to((1.0, 2.0));
        path.quad_to((3.0, 4.0), (5.0, 6.0));
        path.curve_to((7.0, 8.0), (9.0, 10.0), (11.0, 12.0));
        path.line_to((13.0, 14.0));
        path.close_path();
        let fill = CoverageItem {
            path: path.clone(),
            transform: Affine::new([1.0, 0.0, 0.0, 1.0, 20.0, 30.0]),
            style: ItemStyle::Fill(FillRule::NonZero),
            color: [0.1, 0.2, 0.3, 0.4],
        };
        let mut stroke = Stroke::new(2.0);
        stroke.dash_pattern.extend([3.0, 4.0]);
        let source = VectorSource::Scene {
            scene: CoverageScene {
                items: vec![
                    fill.clone(),
                    CoverageItem {
                        style: ItemStyle::Stroke(stroke),
                        ..fill
                    },
                ],
            },
            rect: PixelRect::new(-1, 2, 30, 40),
        };
        assert_eq!(source.key(), source.clone().key());
        let key = source.key();
        macro_rules! changes_key {
            ($label:literal, $change:expr) => {{
                let mut changed = source.clone();
                ($change)(&mut changed);
                assert_ne!(changed.key(), key, "{}", $label);
            }};
        }
        // The variant tag, every point coordinate, and the element count
        // matter independently of the path's rendered bounds.
        changes_key!("MoveTo", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].path.elements_mut()[0] = PathEl::MoveTo((1.25, 2.0).into());
        });
        changes_key!("QuadTo", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].path.elements_mut()[1] =
                PathEl::QuadTo((3.0, 4.25).into(), (5.0, 6.0).into());
        });
        changes_key!("CurveTo", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].path.elements_mut()[2] =
                PathEl::CurveTo((7.0, 8.0).into(), (9.0, 10.0).into(), (11.0, 12.25).into());
        });
        changes_key!("LineTo", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].path.elements_mut()[3] = PathEl::LineTo((13.25, 14.0).into());
        });
        changes_key!("path variant", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].path.elements_mut()[3] = PathEl::MoveTo((13.0, 14.0).into());
        });
        changes_key!("ClosePath", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].path.pop();
        });
        changes_key!("transform bits", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].transform = Affine::new([1.0, -0.0, 0.0, 1.0, 20.0, 30.0]);
        });
        changes_key!("fill rule", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items[0].style = ItemStyle::Fill(FillRule::EvenOdd);
        });
        for channel in 0..4 {
            let mut changed = source.clone();
            let VectorSource::Scene { scene, .. } = &mut changed else {
                unreachable!()
            };
            scene.items[0].color[channel] += 0.01;
            assert_ne!(changed.key(), key, "color channel {channel}");
        }
        macro_rules! stroke_change {
            ($label:literal, $change:expr) => {
                changes_key!($label, |s: &mut VectorSource| {
                    let VectorSource::Scene { scene, .. } = s else {
                        unreachable!()
                    };
                    let ItemStyle::Stroke(stroke) = &mut scene.items[1].style else {
                        unreachable!()
                    };
                    ($change)(stroke);
                });
            };
        }
        stroke_change!("width", |v: &mut Stroke| v.width = 2.25);
        stroke_change!("join", |v: &mut Stroke| v.join = Join::Bevel);
        stroke_change!("miter limit", |v: &mut Stroke| v.miter_limit = 5.0);
        stroke_change!("start cap", |v: &mut Stroke| v.start_cap = Cap::Butt);
        stroke_change!("end cap", |v: &mut Stroke| v.end_cap = Cap::Square);
        stroke_change!("dash length", |v: &mut Stroke| {
            v.dash_pattern.push(5.0);
        });
        stroke_change!("dash value", |v: &mut Stroke| {
            v.dash_pattern[0] = 3.25;
        });
        stroke_change!("dash offset", |v: &mut Stroke| v.dash_offset = 1.0);
        changes_key!("scene item count", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items.pop();
        });
        changes_key!("scene item order", |s: &mut VectorSource| {
            let VectorSource::Scene { scene, .. } = s else {
                unreachable!()
            };
            scene.items.swap(0, 1);
        });
        for field in 0..4 {
            let mut changed = source.clone();
            let VectorSource::Scene { rect, .. } = &mut changed else {
                unreachable!()
            };
            match field {
                0 => rect.x += 1,
                1 => rect.y += 1,
                2 => rect.width += 1,
                3 => rect.height += 1,
                _ => unreachable!(),
            }
            assert_ne!(changed.key(), key, "rect field {field}");
        }
    }
}
