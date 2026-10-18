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
    /// Whether today's raster honors a crop window for this layer.
    pub crops: bool,
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
            let Some(quad) = quad else {
                if ty != "text" {
                    return Ok(None);
                }
                // Text with no ink (blank, transparent): today's raster is a
                // transparent canvas of the text's geometry, never null.
                let fonts = self.text.fonts.as_deref_mut().expect("checked above");
                let g = scope
                    .text_geometry(node, seconds, fonts.measure())
                    .map_err(|e| RenderError::Request(e.to_string()))?;
                let density = g.density(scale);
                let canvas = (
                    (g.w * density).ceil().max(1.0) as u32,
                    (g.h * density).ceil().max(1.0) as u32,
                );
                return Ok(Some(LayerRaster {
                    pixels: LayerPixels::Float {
                        pixels: Vec::new(),
                        crop: [0, 0, 0, 0],
                    },
                    canvas,
                    box_size: (g.w, g.h),
                    anchor: (g.anchor_x, g.anchor_y),
                    selection: Some([
                        g.selection.x0,
                        g.selection.y0,
                        g.selection.x1,
                        g.selection.y1,
                    ]),
                    crops: !animated,
                }));
            };
            // Today's selection: the plain text geometry (moved with the
            // raster by a font anchor), or the raster's box for styled
            // animated text; captions have none.
            let selection = if ty != "text" {
                None
            } else if animated
                && pm_eval::node::member(pm_eval::node::member(Some(node), "d"), "styles")
                    .map(|n| pm_eval::node::get(n, b"length").truthy())
                    .unwrap_or(false)
            {
                Some([-quad.ax, -quad.ay, quad.w - quad.ax, quad.h - quad.ay])
            } else {
                let fonts = self.text.fonts.as_deref_mut().expect("checked above");
                let g = scope
                    .text_geometry(node, seconds, fonts.measure())
                    .map_err(|e| RenderError::Request(e.to_string()))?;
                let (dx, dy) = if animated {
                    (0.0, 0.0)
                } else {
                    (g.anchor_x - quad.ax, g.anchor_y - quad.ay)
                };
                Some([
                    g.selection.x0 + dx,
                    g.selection.y0 + dy,
                    g.selection.x1 + dx,
                    g.selection.y1 + dy,
                ])
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
            return Ok(Some(LayerRaster {
                pixels: LayerPixels::Float {
                    pixels,
                    crop: quad.plan.crop,
                },
                canvas: (quad.plan.scene.width, quad.plan.scene.height),
                box_size: (quad.w, quad.h),
                anchor: (quad.ax, quad.ay),
                selection,
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
            crops: true,
        }))
    }
}
