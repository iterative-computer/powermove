//! Layer rasters: the base pixels of today's `render.raster(L, scale, T,
//! uploaded, crop)` (`PM.raster`, `legacy/gl/raster.ts`) for the Phase 3c
//! render transport (RFC 16), so extensions and raster wrappers get the
//! engine's pixels with today's geometry.
//!
//! As `PM.raster` dispatches:
//!
//! - captions layers: the caption showing at `T` (`rasterCaptions`; the
//!   crop is ignored, as today);
//! - layers with editable paths: `rasterPaths` (crop ignored);
//! - text layers: `rasterText`, or `rasterAnimatedText` with animators or
//!   styles (the crop applies to plain text only, as today);
//! - every other layer: `rasterShape` of its resolved content (the crop
//!   applies).
//!
//! The layer may be the document's (by id) or an object the caller passed
//! (`content`: a wrapper's modified copy, `{ ...L, d: { ...L.d, text } }`),
//! evaluated in the composition as the document's layer would be.

use pm_eval::{Comp, Scope};
use pm_model::{Document, Node};
use pm_protocol::color::Precision;

use super::Renderer;
use crate::capability::{self, Feature, Unsupported};
use crate::error::RenderError;

/// A layer raster's pixels.
#[derive(Debug, Clone)]
pub enum LayerPixels {
    /// Premultiplied working values (top-down) of `crop` (`[x, y, w, h]`)
    /// inside the canvas; the rest of the canvas is transparent (text).
    Float { pixels: Vec<f32>, crop: [u32; 4] },
    /// The whole canvas, premultiplied RGBA8, top-down (shapes, paths).
    Rgba8(Vec<u8>),
}

/// One layer raster with today's geometry.
#[derive(Debug, Clone)]
pub struct LayerRaster {
    pub pixels: LayerPixels,
    /// The canvas in device pixels.
    pub canvas: (u32, u32),
    /// The raster's box in layer pixels (`w`, `h`) and the layer origin in it
    /// (`anchorX`, `anchorY`).
    pub box_size: (f64, f64),
    pub anchor: (f64, f64),
    /// Today's `selection` (`x0, y0, x1, y1`, layer pixels).
    pub selection: Option<[f64; 4]>,
    /// Today's `fontOffset` (a text layer with typography anchoring).
    pub font_offset: Option<(f64, f64)>,
    /// Whether today's raster honors a crop window for this layer.
    pub crops: bool,
}

/// `min(scale, 8192 / w, 8192 / h)`: today's raster density.
fn js_density(w: f64, h: f64, scale: f64) -> f64 {
    let min = |a: f64, b: f64| {
        if a.is_nan() || b.is_nan() {
            f64::NAN
        } else {
            a.min(b)
        }
    };
    min(min(scale, 8192.0 / w), 8192.0 / h)
}

impl Renderer {
    /// The raster of layer `layer` (or `content`, a layer object the caller
    /// passed) of `composition` at composition time `seconds`, at raster
    /// scale `scale`. `None` when it draws nothing (no caption showing).
    #[allow(clippy::too_many_arguments)]
    pub fn layer_raster(
        &mut self,
        document: &Document,
        composition: &str,
        layer: &str,
        content: Option<&Node>,
        seconds: f64,
        scale: f64,
        precision: Precision,
    ) -> Result<Option<LayerRaster>, RenderError> {
        let comp = Comp::resolve(document, composition)
            .map_err(|_| RenderError::Composition(composition.to_owned()))?;
        let root = document
            .root()
            .ok_or_else(|| RenderError::Composition(composition.to_owned()))?;
        let open = capability::open_composition_id(root);
        let project = open
            .as_deref()
            .and_then(|id| Comp::resolve(document, id).ok())
            .unwrap_or_else(|| comp.clone());
        let comp_id = comp.id().to_string_lossy();
        let mut scope = Scope::new(&mut self.evaluator, &comp, &project);
        let found = scope
            .layers()
            .iter()
            .find(|l| crate::js::layer_id(l) == layer);
        let node: &Node = match (content, found) {
            (Some(content), _) => content,
            (None, Some(found)) => found,
            (None, None) => return Err(RenderError::Request(format!("no layer {layer}"))),
        };
        let ty = crate::js::layer_type(node);
        let has_paths = pm_eval::node::array(pm_eval::node::member(
            pm_eval::node::member(Some(node), "d"),
            "paths",
        ))
        .is_some_and(|p| !p.is_empty());
        if ty == "captions" || (ty == "text" && !has_paths) {
            if !self.text.has_fonts() {
                return Err(RenderError::Unsupported(vec![Unsupported::request(
                    "text without configured fonts",
                )]));
            }
            let animated = ty == "text" && {
                let stored = pm_eval::node::member(Some(node), "d");
                ["animators", "styles"].iter().any(|key| {
                    pm_eval::node::member(stored, key)
                        .map(|n| pm_eval::node::get(n, b"length").truthy())
                        .unwrap_or(false)
                })
            };
            let quad = if ty == "text" {
                self.text.text_layer(&mut scope, node, seconds, scale)
            } else {
                self.text.captions_layer(&mut scope, node, seconds, scale)
            };
            let refused: Vec<Unsupported> = self
                .text
                .take_unsupported()
                .into_iter()
                .map(|what| Unsupported::layer(&comp_id, node, Feature::Text(what)))
                .collect();
            let quad = quad.map_err(|e| RenderError::Request(e.to_string()))?;
            if !refused.is_empty() {
                return Err(RenderError::Unsupported(refused));
            }
            // Today's placement (`PM.raster`'s anchor, selection, and font
            // offset): pm-eval's, as pm-wasm answers it; captions have no
            // selection or font offset.
            let mut whole: Option<((f64, f64), (f64, f64))> = None;
            let (selection, font_offset) = if ty == "text" {
                let fonts = self.text.fonts.as_deref_mut().expect("checked above");
                let placed = scope
                    .text_raster(node, seconds, fonts.measure())
                    .map_err(|e| RenderError::Request(e.to_string()))?;
                // Today's raster box and anchor: the whole canvas's.
                whole = Some(((placed.w, placed.h), (placed.anchor_x, placed.anchor_y)));
                let s = placed.selection;
                // `L.d.fontAnchorBounds`, as `PM.raster` tests it.
                let anchored = pm_eval::node::truthy(pm_eval::node::member(
                    pm_eval::node::member(Some(node), "d"),
                    "fontAnchorBounds",
                ));
                let offset = if anchored {
                    let free = scope
                        .text_raster_with(node, seconds, fonts.measure(), true, false)
                        .map_err(|e| RenderError::Request(e.to_string()))?;
                    Some((
                        free.anchor_x - placed.anchor_x,
                        free.anchor_y - placed.anchor_y,
                    ))
                } else {
                    None
                };
                (Some([s.x0, s.y0, s.x1, s.y1]), offset)
            } else {
                (None, None)
            };
            let Some(quad) = quad else {
                if ty != "text" {
                    return Ok(None);
                }
                // Text with no ink (blank, transparent): today's raster is a
                // transparent canvas of the text's geometry, never null.
                let fonts = self.text.fonts.as_deref_mut().expect("checked above");
                let placed = scope
                    .text_raster(node, seconds, fonts.measure())
                    .map_err(|e| RenderError::Request(e.to_string()))?;
                let density = js_density(placed.w, placed.h, scale);
                let canvas = (
                    (placed.w * density).ceil().max(1.0) as u32,
                    (placed.h * density).ceil().max(1.0) as u32,
                );
                return Ok(Some(LayerRaster {
                    pixels: LayerPixels::Float {
                        pixels: Vec::new(),
                        crop: [0, 0, 0, 0],
                    },
                    canvas,
                    box_size: (placed.w, placed.h),
                    anchor: (placed.anchor_x, placed.anchor_y),
                    selection,
                    font_offset,
                    crops: !animated,
                }));
            };
            let mut encoder = self
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("pm layer raster"),
                });
            let mut taken = Vec::new();
            let view = self
                .text_gpu
                .render(
                    &self.device,
                    &self.queue,
                    &mut encoder,
                    &mut self.pool,
                    &mut taken,
                    &quad.plan,
                    precision,
                )
                .map_err(RenderError::Media)?;
            self.queue.submit([encoder.finish()]);
            for t in taken {
                self.pool.give(t);
            }
            let pixels = pm_gpu::readback::read_f32(&self.device, &self.queue, view.texture())?;
            // The pixels are placed in the whole canvas (only its inked part
            // is painted, and the quad shrinks to it for compositing), so
            // the surface's box and anchor are the whole canvas's, as
            // today's `RasterSurface` has them: a wrapper drawing `cv` at
            // `w` × `h` from `-anchorX, -anchorY` must not squeeze it.
            let crop = quad.plan.crop;
            let canvas = (quad.plan.scene.width, quad.plan.scene.height);
            let (box_size, anchor) = whole.unwrap_or_else(|| {
                // The quad is the ink crop `[x0, y0, x1, y1]` of a canvas
                // stretched over its box (`TextContext::quad`): undo it.
                let [x0, y0, x1, y1] = crop.map(f64::from);
                let sx = if x1 > x0 { quad.w / (x1 - x0) } else { 1.0 / scale };
                let sy = if y1 > y0 { quad.h / (y1 - y0) } else { 1.0 / scale };
                (
                    (f64::from(canvas.0) * sx, f64::from(canvas.1) * sy),
                    (quad.ax + x0 * sx, quad.ay + y0 * sy),
                )
            });
            return Ok(Some(LayerRaster {
                pixels: LayerPixels::Float {
                    pixels,
                    crop,
                },
                canvas,
                box_size,
                anchor,
                selection,
                font_offset,
                crops: ty == "text" && !animated,
            }));
        }
        if has_paths {
            let (_, colors, layer_paths) = crate::vector::paths(&mut scope, node, seconds)
                .map_err(|e| RenderError::Request(e.to_string()))?;
            let (bitmap, raster) =
                pm_vector::canvas2d::shape::raster_paths(&layer_paths, &colors, scale)
                    .map_err(|e| RenderError::Request(e.to_string()))?;
            let b = layer_paths.bounds;
            return Ok(Some(LayerRaster {
                canvas: (bitmap.width, bitmap.height),
                pixels: LayerPixels::Rgba8(bitmap.rgba),
                box_size: (b.w, b.h),
                anchor: (raster.anchor_x, raster.anchor_y),
                selection: Some([b.x0, b.y0, b.x1, b.y1]),
                font_offset: None,
                crops: false,
            }));
        }
        let d = scope
            .ev
            .resolve_content(scope.comp, node, seconds)
            .map_err(|e| RenderError::Request(e.to_string()))?;
        let (content, colors) = crate::vector::shape_content(&d);
        let g = pm_vector::shape::ShapeRaster::new(&content, scale);
        let bitmap = pm_vector::canvas2d::shape::raster_shape(&content, &colors, scale)
            .map_err(|e| RenderError::Request(e.to_string()))?;
        let s = g.selection;
        Ok(Some(LayerRaster {
            canvas: (bitmap.width, bitmap.height),
            pixels: LayerPixels::Rgba8(bitmap.rgba),
            box_size: (g.w, g.h),
            anchor: (g.pad, g.pad),
            selection: Some([s.x0, s.y0, s.x1, s.y1]),
            font_offset: None,
            crops: true,
        }))
    }
}
