//! Shape layers (plan section 10; design notes in docs/engine/vector.md).
//!
//! `compat` level 1 shape content is today's Canvas2D raster: the bitmap
//! `rasterShape` or `rasterPaths` draws at the continuous raster scale,
//! produced by `pm_vector::canvas2d` (an emulation of Chromium's Graphite
//! Canvas2D) and drawn exactly as the compositor draws the uploaded canvas.
//! Level 2 content is rendered at output resolution with analytic area
//! coverage ([`level2`]); level 2 SVG images are vectors or resvg rasters
//! at output resolution ([`svg`]). Level 1 SVG images are Chromium's
//! rasters at today's buckets ([`svg`], [`cache`]).

pub mod cache;
pub mod level2;
pub mod svg;
pub mod svg_rasters;

use pm_eval::transform::Mat2d;
use pm_eval::{JsResult, JsValue, Scope};
use pm_model::Node;
use pm_vector::canvas2d::shape::{PathColors, ShapeColors};
use pm_vector::path::{PathLayer, PathNode};
use pm_vector::render::scene::{CoverageScene, PixelRect};
use pm_vector::shape::{ShapeContent, ShapeKind, ShapeRaster};
use sha2::{Digest, Sha256};

use crate::js;

/// Vector content the executor resolves to a texture.
#[derive(Debug, Clone, PartialEq)]
pub enum VectorSource {
    /// A level 1 shape raster (`rasterShape(d, scale, crop)`): with a
    /// preview crop `[x, y, width, height]`, only that window of the raster
    /// (today's viewer, `previewShapeRaster`; `crate::compat_crop`).
    Shape {
        content: ShapeContent,
        colors: ShapeColors,
        scale: f64,
        crop: Option<[u32; 4]>,
    },
    /// A level 1 editable-paths raster (`rasterPaths(PM, L, T, scale)`):
    /// with `crop`, only that window of the bitmap (the part a frame
    /// samples, `crate::compat_crop::paths_plan`).
    Paths {
        nodes: Vec<PathNode>,
        colors: Vec<PathColors>,
        scale: f64,
        crop: Option<[u32; 4]>,
    },
    /// Level 2: a coverage scene in target pixels, rendered into `rect`.
    Scene {
        scene: CoverageScene,
        rect: PixelRect,
    },
    /// Level 2: an SVG drawn by resvg under `transform` (user units to
    /// target pixels) into `rect`, clipped to the polygon `clip`.
    SvgRaster {
        path: std::path::PathBuf,
        stamp: svg::Stamp,
        transform: [f64; 6],
        rect: PixelRect,
        clip: [[f64; 2]; 4],
    },
    /// Level 1: an SVG image's raster at today's bucket
    /// (`rasterSvgAsset`), drawn by Chromium in the renderer that owns the
    /// document (decision S1; `cache`, "Level 1 SVG rasters"). Only the
    /// content and the bucket decide the pixels; the asset and its file
    /// say where the renderer and `pm-cli` find the SVG.
    SvgImage {
        key: pm_protocol::svg_raster::SvgRasterKey,
        asset: String,
        path: std::path::PathBuf,
    },
}

impl VectorSource {
    /// A content key over every input that changes the pixels (section 8).
    pub fn key(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(b"pm-vector/1");
        match self {
            VectorSource::Shape {
                content,
                colors,
                scale,
                crop,
            } => {
                h.update(b"shape");
                h.update(format!("{content:?}").as_bytes());
                h.update(colors.color.as_bytes());
                h.update([0]);
                h.update(colors.stroke_color.as_bytes());
                h.update(scale.to_bits().to_le_bytes());
                // The whole raster keeps its key; a window is another one.
                if let Some(c) = crop {
                    h.update(b"crop");
                    for v in c {
                        h.update(v.to_le_bytes());
                    }
                }
            }
            VectorSource::Paths {
                nodes,
                colors,
                scale,
                crop,
            } => {
                h.update(b"paths");
                for (n, c) in nodes.iter().zip(colors) {
                    h.update(format!("{n:?}").as_bytes());
                    h.update(c.fill.as_bytes());
                    h.update([0]);
                    h.update(c.stroke.as_bytes());
                    h.update([0]);
                }
                h.update(scale.to_bits().to_le_bytes());
                if let Some(c) = crop {
                    h.update(b"crop");
                    for v in c {
                        h.update(v.to_le_bytes());
                    }
                }
            }
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
            VectorSource::SvgImage { key, .. } => {
                h.update(b"svgimage");
                h.update(key.content.as_bytes());
                h.update(key.width.to_le_bytes());
                h.update(key.height.to_le_bytes());
            }
        }
        h.finalize().into()
    }
}

/// How a vector layer's content is drawn.
#[derive(Debug, Clone, PartialEq)]
pub enum VectorQuad {
    /// A bitmap stretched over `w` × `h` layer units with its anchor at
    /// (`ax`, `ay`) (the legacy content quad).
    Layer {
        source: VectorSource,
        w: f64,
        h: f64,
        ax: f64,
        ay: f64,
        /// The bitmap's texture coordinates over the quad (today's `r.uv`:
        /// a cropped raster maps its window into the whole quad).
        uv: [f64; 4],
    },
    /// Pixels in target space covering `rect` exactly.
    Device {
        source: VectorSource,
        rect: PixelRect,
    },
}

/// `continuousRasterScale(matrix, density = 1, maxScale = 32)`: the largest
/// singular value, clamped to 0.25..32, rounded up to quarter steps.
pub fn continuous_raster_scale(m: &Mat2d) -> f64 {
    let n = |v: f64| if v.is_nan() || v == 0.0 { 0.0 } else { v };
    let (m0, m1, m2, m3) = (n(m[0]), n(m[1]), n(m[2]), n(m[3]));
    let aa = m0 * m0 + m1 * m1;
    let bb = m0 * m2 + m1 * m3;
    let cc = m2 * m2 + m3 * m3;
    let discriminant = js::max(0.0, (aa - cc) * (aa - cc) + 4.0 * bb * bb).sqrt();
    let largest = js::max(0.0, (aa + cc + discriminant) / 2.0).sqrt();
    let requested = largest * js::max(0.01, 1.0);
    let bounded = js::max(0.25, js::min(js::max(0.25, 32.0), requested));
    (bounded * 4.0 - 1e-9).ceil() / 4.0
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

/// Plans a shape layer's content. `world` maps layer units to target
/// pixels; `target` is the target's pixel rectangle.
pub fn plan_shape<'d>(
    scope: &mut Scope<'_, 'd>,
    layer: &'d Node,
    d: &pm_eval::Content,
    time: f64,
    world: &Mat2d,
    target: PixelRect,
    crop: Option<pm_protocol::common::PixelRect>,
    paths_view: Option<([f64; 6], f64, f64)>,
) -> JsResult<Option<VectorQuad>> {
    let has_paths = pm_eval::node::array(pm_eval::node::member(
        pm_eval::node::member(Some(layer), "d"),
        "paths",
    ))
    .is_some_and(|p| !p.is_empty());
    if level2(layer) {
        let scene = if has_paths {
            let (nodes, colors, layer_paths) = paths(scope, layer, time)?;
            let _ = nodes;
            level2::paths_scene(&layer_paths, &colors, world)
        } else {
            let (content, colors) = shape_content(d);
            match level2::shape_scene(&content, &colors, world) {
                Some(s) => s,
                None => return Ok(None),
            }
        };
        return Ok(scene.pixel_bounds(target).map(|rect| VectorQuad::Device {
            source: VectorSource::Scene { scene, rect },
            rect,
        }));
    }
    let scale = continuous_raster_scale(world);
    if has_paths {
        let (nodes, colors, layer_paths) = paths(scope, layer, time)?;
        let b = layer_paths.bounds;
        // Only the window a frame samples of a large paths bitmap (the
        // engine's own draw; `crate::compat_crop::paths_plan`).
        let raster = layer_paths.raster(scale);
        let crop = match paths_view.map(|(m, vw, vh)| {
            crate::compat_crop::paths_plan(b.x0, b.y0, b.w, b.h, raster.width, raster.height, &m, vw, vh)
        }) {
            Some(crate::compat_crop::CropPlan::Outside) => return Ok(None),
            Some(crate::compat_crop::CropPlan::Crop(c)) if c.width > 0 && c.height > 0 => Some(c),
            _ => None,
        };
        let uv = match crop {
            Some(c) => {
                let (cw, ch) = (f64::from(c.width), f64::from(c.height));
                [
                    -f64::from(c.x) / cw,
                    -f64::from(c.y) / ch,
                    raster.width / cw,
                    raster.height / ch,
                ]
            }
            None => [0.0, 0.0, 1.0, 1.0],
        };
        return Ok(Some(VectorQuad::Layer {
            source: VectorSource::Paths {
                nodes,
                colors,
                scale,
                crop: crop.map(|c| [c.x, c.y, c.width, c.height]),
            },
            w: b.w,
            h: b.h,
            ax: -b.x0,
            ay: -b.y0,
            uv,
        }));
    }
    let (content, colors) = shape_content(d);
    let g = ShapeRaster::new(&content, scale);
    // Today's `rasterShape(d, scale, crop)`: the quad stays the layer's, the
    // window's texture coordinates are
    // `[-x / cw, -y / ch, width / cw, height / ch]`.
    let crop = crop.filter(|c| c.width > 0 && c.height > 0);
    let uv = match crop {
        Some(c) => {
            let (cw, ch) = (f64::from(c.width), f64::from(c.height));
            [
                -f64::from(c.x) / cw,
                -f64::from(c.y) / ch,
                g.width / cw,
                g.height / ch,
            ]
        }
        None => [0.0, 0.0, 1.0, 1.0],
    };
    Ok(Some(VectorQuad::Layer {
        source: VectorSource::Shape {
            content,
            colors,
            scale,
            crop: crop.map(|c| [c.x, c.y, c.width, c.height]),
        },
        w: g.w,
        h: g.h,
        ax: g.w / 2.0,
        ay: g.h / 2.0,
        uv,
    }))
}

/// A layer's editable paths: nodes, colors, and geometry.
pub(crate) fn paths<'d>(
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
mod tests {
    use super::*;

    #[test]
    fn raster_scale_follows_the_compositor() {
        assert_eq!(
            continuous_raster_scale(&[1.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            1.0
        );
        assert_eq!(
            continuous_raster_scale(&[1.1, 0.0, 0.0, 1.0, 0.0, 0.0]),
            1.25
        );
        assert_eq!(
            continuous_raster_scale(&[0.1, 0.0, 0.0, 0.1, 0.0, 0.0]),
            0.25
        );
        assert_eq!(
            continuous_raster_scale(&[100.0, 0.0, 0.0, 1.0, 0.0, 0.0]),
            32.0
        );
        // A rotation by 45 degrees at scale 2.
        let s = core::f64::consts::FRAC_1_SQRT_2 * 2.0;
        assert_eq!(continuous_raster_scale(&[s, s, -s, s, 0.0, 0.0]), 2.0);
        assert_eq!(
            continuous_raster_scale(&[f64::NAN, 0.0, 0.0, 0.0, 0.0, 0.0]),
            0.25
        );
    }
}

/// Every level 1 shape and paths source a plan draws, in drawing order
/// (nested compositions, motion-blur samples, layer targets and matte
/// passes included): what [`cache::VectorCache::prefetch`] rasterizes ahead.
pub(crate) fn level1_sources<'p>(plan: &'p crate::plan::CompPlan, out: &mut Vec<&'p VectorSource>) {
    for draw in &plan.draws {
        level1_in(&draw.content, out);
    }
}

fn level1_in<'p>(content: &'p crate::plan::Content, out: &mut Vec<&'p VectorSource>) {
    use crate::plan::{Content, Source};
    match content {
        Content::Solid(_) => {}
        Content::Placed(p) => level1_in(&p.content, out),
        Content::Composite(node) => {
            if let crate::composite::Node::Layer(layer) = &**node {
                for draw in &layer.content {
                    level1_in(&draw.content, out);
                }
                if let Some(matte) = &layer.matte {
                    level1_sources(&matte.source, out);
                }
            }
        }
        Content::Texture { source, .. } => match source {
            Source::Vector(v) if matches!(**v, VectorSource::Shape { .. } | VectorSource::Paths { .. }) => out.push(v),
            Source::Comp(child) | Source::Accumulate(child) => level1_sources(child, out),
            _ => {}
        },
    }
}
