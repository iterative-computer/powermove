//! Today's shape and path rasters (`rasterShape` in `legacy/gl/raster.ts`,
//! `rasterPaths` in `legacy/core/vector-paths.ts`), replayed call for call
//! on the emulated Canvas2D context: the exact bitmap the legacy compositor
//! uploads for a shape layer at a raster scale (`compat` level 1).

use super::blink::path_from_svg;
use super::context::{Context, LineCap, LineJoin, Unsupported};
use crate::corner;
use crate::js;
use crate::path::{PathLayer, PathRaster};
use crate::shape::{ShapeContent, ShapeKind, ShapeRaster, STAR_INNER_RATIO};

/// A rasterized canvas: premultiplied RGBA8, top-down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Why a raster could not be produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RasterError {
    /// Canvas2D throws for this content (a negative radius); today the
    /// layer's raster fails.
    Throws(&'static str),
    Unsupported(Unsupported),
}

impl From<Unsupported> for RasterError {
    fn from(u: Unsupported) -> Self {
        Self::Unsupported(u)
    }
}

impl core::fmt::Display for RasterError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Throws(what) => write!(f, "Canvas2D throws: {what}"),
            Self::Unsupported(u) => u.fmt(f),
        }
    }
}

impl std::error::Error for RasterError {}

/// The string-valued shape content (`d.color`, `d.strokeColor`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShapeColors {
    pub color: String,
    pub stroke_color: String,
}

/// `rasterShape(d, scale)` without a preview crop.
pub fn raster_shape(d: &ShapeContent, colors: &ShapeColors, scale: f64) -> Result<Bitmap, RasterError> {
    raster_shape_window(d, colors, scale, None)
}

/// `rasterShape(d, scale, crop)`: with a preview crop `[x, y, width,
/// height]` (raster pixels), the canvas is the crop's size and is translated
/// by `(-x, -y)` before the shape's own transform, as today's viewer asks
/// for the visible window of a large shape (`previewShapeRaster`).
pub fn raster_shape_window(
    d: &ShapeContent,
    colors: &ShapeColors,
    scale: f64,
    crop: Option<[u32; 4]>,
) -> Result<Bitmap, RasterError> {
    let g = ShapeRaster::new(d, scale);
    let (width, height) = match crop {
        Some([_, _, w, h]) => (w, h),
        None => (g.width as u32, g.height as u32),
    };
    let mut c = Context::new(width, height);
    if let Some([x, y, _, _]) = crop {
        c.translate(-f64::from(x), -f64::from(y));
    }
    c.scale(g.density, g.density);
    c.translate(g.pad, g.pad);
    c.set_fill_style(&colors.color);
    c.set_stroke_style(if colors.stroke_color.is_empty() { "#fff" } else { &colors.stroke_color });
    c.set_line_width(js::or0(d.stroke));
    c.set_line_join(LineJoin::Round);
    let (w, h) = (d.w, d.h);
    match d.shape {
        ShapeKind::Ellipse => {
            c.begin_path();
            c.path
                .ellipse(w / 2.0, h / 2.0, w / 2.0, h / 2.0, 0.0, 7.0, false)
                .map_err(|_| RasterError::Throws("negative ellipse radius"))?;
        }
        ShapeKind::Polygon | ShapeKind::Star => {
            let star = d.shape == ShapeKind::Star;
            let n = js::max(3.0, js::or(d.points, 5.0));
            let outer = js::min(w, h) / 2.0;
            let inner = if star { outer * STAR_INNER_RATIO } else { outer };
            let total = n * if star { 2.0 } else { 1.0 };
            let count = js::min(total, crate::shape::MAX_POLYGON_VERTICES);
            c.begin_path();
            let mut i = 0u32;
            while f64::from(i) < count {
                let fi = f64::from(i);
                let a = -core::f64::consts::PI / 2.0 + (fi * core::f64::consts::PI * 2.0) / total;
                let rad = if star && i % 2 == 1 { inner } else { outer };
                let (x, y) = (w / 2.0 + js::cos(a) * rad, h / 2.0 + js::sin(a) * rad);
                if i == 0 { c.path.move_to(x, y) } else { c.path.line_to(x, y) }
                i += 1;
            }
            c.path.close_path();
        }
        ShapeKind::Line => {
            c.begin_path();
            c.path.move_to(0.0, h / 2.0);
            c.path.line_to(w, h / 2.0);
            c.set_line_width(js::max(1.0, js::or(d.stroke, 6.0)));
            c.set_stroke_style(&colors.color);
            c.set_line_cap(LineCap::Round);
            c.stroke()?;
            return Ok(finish(&c));
        }
        ShapeKind::Rect => {
            let corners = d.corners();
            if corner::is_uniform_circular(&corners) {
                rr(&mut c, 0.0, 0.0, w, h, js::or0(d.radius))?;
            } else {
                let outline = path_from_svg(&corner::rounded_rect_svg(&corners));
                c.fill_path(&outline)?;
                if d.stroke > 0.0 {
                    c.stroke_path(&outline)?;
                }
                return Ok(finish(&c));
            }
        }
    }
    c.fill()?;
    if d.stroke > 0.0 {
        c.stroke()?;
    }
    Ok(finish(&c))
}

/// `rr(c, x, y, w, h, r)`.
fn rr(c: &mut Context, x: f64, y: f64, w: f64, h: f64, r: f64) -> Result<(), RasterError> {
    let r = js::min(js::min(r, w.abs() / 2.0), h.abs() / 2.0);
    c.begin_path();
    c.path.move_to(x + r, y);
    let neg = |_| RasterError::Throws("negative arcTo radius");
    c.path.arc_to(x + w, y, x + w, y + h, r).map_err(neg)?;
    c.path.arc_to(x + w, y + h, x, y + h, r).map_err(neg)?;
    c.path.arc_to(x, y + h, x, y, r).map_err(neg)?;
    c.path.arc_to(x, y, x + w, y, r).map_err(neg)?;
    c.path.close_path();
    Ok(())
}

fn finish(c: &Context) -> Bitmap {
    Bitmap {
        width: c.width(),
        height: c.height(),
        rgba: c.finish(),
    }
}

/// The string-valued path content (`path.fill`, `path.stroke`), per path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PathColors {
    pub fill: String,
    pub stroke: String,
}

/// `rasterPaths(PM, layer, time, scale)`.
pub fn raster_paths(layer: &PathLayer, colors: &[PathColors], scale: f64) -> Result<(Bitmap, PathRaster), RasterError> {
    raster_paths_window(layer, colors, scale, None)
}

/// [`raster_paths`] for only the window `[x, y, width, height]` of the
/// bitmap: a canvas of the window's size translated by `(-x, -y)` before
/// the raster's own transform (the engine draws the part of a large paths
/// raster a frame samples; its pixels are the whole bitmap's there).
pub fn raster_paths_window(
    layer: &PathLayer,
    colors: &[PathColors],
    scale: f64,
    crop: Option<[u32; 4]>,
) -> Result<(Bitmap, PathRaster), RasterError> {
    let raster = layer.raster(scale);
    let (width, height) = match crop {
        Some([_, _, w, h]) => (w, h),
        None => (raster.width as u32, raster.height as u32),
    };
    let mut c = Context::new(width.max(1), height.max(1));
    if let Some([x, y, _, _]) = crop {
        c.translate(-f64::from(x), -f64::from(y));
    }
    c.scale(raster.density, raster.density);
    c.translate(-layer.bounds.x0, -layer.bounds.y0);
    let empty = PathColors::default();
    for draw in &layer.draws {
        let col = colors.get(draw.path).unwrap_or(&empty);
        c.save();
        for op in draw.ops {
            match op {
                crate::canvas::TransformOp::Transform(m) => c.transform(m),
                crate::canvas::TransformOp::Translate(x, y) => c.translate(x, y),
                crate::canvas::TransformOp::Rotate(r) => c.rotate(r),
                crate::canvas::TransformOp::Scale(x, y) => c.scale(x, y),
            }
        }
        trace(&mut c, &layer.traced[draw.path]);
        c.set_fill_style(&col.fill);
        c.set_stroke_style(&col.stroke);
        if let Some(stroke) = &draw.stroke {
            c.set_line_width(js::max(0.0, stroke.style.width));
        } else {
            // `ctx.lineWidth = Math.max(0, strokeWidth)`: 0 is ignored.
        }
        if let Some(alpha) = draw.fill {
            c.set_global_alpha(alpha);
            c.fill()?;
        }
        if let Some(stroke) = &draw.stroke {
            c.set_global_alpha(stroke.alpha);
            c.stroke()?;
        }
        c.restore();
    }
    Ok((
        finish(&c),
        raster,
    ))
}

/// `tracePath(ctx, values)` replayed from the traced geometry (the same
/// calls: moveTo, bezierCurveTo or lineTo, closePath).
pub(crate) fn trace(c: &mut Context, path: &kurbo::BezPath) {
    use kurbo::PathEl;
    c.begin_path();
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => c.path.move_to(p.x, p.y),
            PathEl::LineTo(p) => c.path.line_to(p.x, p.y),
            PathEl::CurveTo(a, b, p) => c.path.bezier_curve_to(a.x, a.y, b.x, b.y, p.x, p.y),
            PathEl::QuadTo(a, p) => c.path.bezier_curve_to(a.x, a.y, a.x, a.y, p.x, p.y),
            PathEl::ClosePath => c.path.close_path(),
        }
    }
}
