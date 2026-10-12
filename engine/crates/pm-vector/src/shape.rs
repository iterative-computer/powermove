//! Shape primitives: rect (per-corner radius and smoothing), ellipse, polygon,
//! star, and line.
//!
//! A port of `rasterShape` and `rr` in `legacy/gl/raster.ts` and of
//! `shapeRasterGeometry` in `legacy/gl/shape-raster-window.ts`. Geometry is in
//! shape space: the origin is the top-left corner of the `w` × `h` box, as on
//! today's canvas after its pad translation. [`ShapeRaster`] reproduces
//! today's bitmap sizing and how the compositor maps the bitmap onto the layer.

use core::f64::consts::PI;

use kurbo::{Affine, BezPath, Cap, Join, Rect, Stroke};

use crate::canvas::{CanvasPath, NegativeRadius};
use crate::corner::{self, CornerContent};
use crate::js;

/// Star inner radius as a fraction of the outer radius.
pub const STAR_INNER_RATIO: f64 = 0.46;

/// Today's maximum raster dimension for shapes and paths, in pixels.
pub const MAX_RASTER_SIZE: f64 = 8192.0;

/// Most polygon or star vertices drawn. Today an enormous or infinite point
/// count loops until the renderer hangs; the engine stops here instead.
pub const MAX_POLYGON_VERTICES: f64 = 1_048_576.0;

/// Canvas2D's default miter limit.
pub const CANVAS_MITER_LIMIT: f64 = 10.0;

/// `d.shape`. Anything other than the four named shapes draws a rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShapeKind {
    Rect,
    Ellipse,
    Polygon,
    Star,
    Line,
}

impl ShapeKind {
    /// The kind `rasterShape` draws for a `d.shape` string.
    pub fn from_name(name: &str) -> Self {
        match name {
            "ellipse" => Self::Ellipse,
            "polygon" => Self::Polygon,
            "star" => Self::Star,
            "line" => Self::Line,
            _ => Self::Rect,
        }
    }
}

/// Evaluated shape content (`resolveContent` output) as numbers. Missing
/// numeric fields are NaN, which is what `Number(undefined)` gives.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeContent {
    pub shape: ShapeKind,
    pub w: f64,
    pub h: f64,
    pub radius: f64,
    pub independent_corners: bool,
    /// `radiusTL`, `radiusTR`, `radiusBR`, `radiusBL`; `None` for null or undefined.
    pub corner_radii: [Option<f64>; 4],
    /// Smoothing percent.
    pub smoothing: f64,
    /// Stroke width (`d.stroke`).
    pub stroke: f64,
    /// Polygon and star points (`d.points`).
    pub points: f64,
}

impl Default for ShapeContent {
    fn default() -> Self {
        Self {
            shape: ShapeKind::Rect,
            w: f64::NAN,
            h: f64::NAN,
            radius: f64::NAN,
            independent_corners: false,
            corner_radii: [None; 4],
            smoothing: f64::NAN,
            stroke: f64::NAN,
            points: f64::NAN,
        }
    }
}

impl ShapeContent {
    /// The corner fields.
    pub fn corners(&self) -> CornerContent {
        CornerContent {
            w: self.w,
            h: self.h,
            radius: self.radius,
            independent_corners: self.independent_corners,
            corner_radii: self.corner_radii,
            smoothing: self.smoothing,
        }
    }
}

/// Which content color a stroke uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrokePaint {
    /// `d.strokeColor || '#fff'`.
    StrokeColor,
    /// `d.color` (lines are stroked with the fill color).
    Color,
}

/// A stroke and the content color it uses.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeStroke {
    pub style: Stroke,
    pub paint: StrokePaint,
}

/// Shape geometry in shape space, ready for Vello: fill `path` with `d.color`
/// when `fill` is set, then stroke it when `stroke` is present.
#[derive(Debug, Clone, PartialEq)]
pub struct ShapeGeometry {
    pub path: BezPath,
    pub fill: bool,
    pub stroke: Option<ShapeStroke>,
}

/// Why a shape has no geometry. Canvas2D throws for these inputs, so today the
/// layer's raster fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShapeError {
    /// A negative uniform radius (`arcTo`) or negative ellipse size (`ellipse`).
    NegativeRadius,
}

impl core::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NegativeRadius => f.write_str("negative shape radius (Canvas2D IndexSizeError)"),
        }
    }
}

impl std::error::Error for ShapeError {}

impl From<NegativeRadius> for ShapeError {
    fn from(_: NegativeRadius) -> Self {
        Self::NegativeRadius
    }
}

/// `rr(c, x, y, w, h, r)`: a rounded rectangle traced with `arcTo`.
fn rr(path: &mut CanvasPath, x: f64, y: f64, w: f64, h: f64, r: f64) -> Result<(), NegativeRadius> {
    let r = js::min(js::min(r, w.abs() / 2.0), h.abs() / 2.0);
    path.move_to(x + r, y);
    path.arc_to(x + w, y, x + w, y + h, r)?;
    path.arc_to(x + w, y + h, x, y + h, r)?;
    path.arc_to(x, y + h, x, y, r)?;
    path.arc_to(x, y, x + w, y, r)?;
    path.close_path();
    Ok(())
}

fn shape_stroke(d: &ShapeContent) -> Option<ShapeStroke> {
    // `c.lineWidth = d.stroke || 0` keeps the default width 1 for values the
    // setter rejects; only `d.stroke > 0` strokes.
    (d.stroke > 0.0).then(|| ShapeStroke {
        style: Stroke::new(if d.stroke.is_finite() { d.stroke } else { 1.0 })
            .with_join(Join::Round)
            .with_miter_limit(CANVAS_MITER_LIMIT),
        paint: StrokePaint::StrokeColor,
    })
}

/// The geometry `rasterShape` draws for this content.
pub fn shape_geometry(d: &ShapeContent) -> Result<ShapeGeometry, ShapeError> {
    let (w, h) = (d.w, d.h);
    let mut path = CanvasPath::new();
    match d.shape {
        ShapeKind::Ellipse => {
            path.ellipse(w / 2.0, h / 2.0, w / 2.0, h / 2.0, 0.0, 0.0, 7.0)?;
        }
        ShapeKind::Polygon | ShapeKind::Star => {
            let star = d.shape == ShapeKind::Star;
            let n = js::max(3.0, js::or(d.points, 5.0));
            let outer = js::min(w, h) / 2.0;
            let inner = if star { outer * STAR_INNER_RATIO } else { outer };
            let count = js::min(n * if star { 2.0 } else { 1.0 }, MAX_POLYGON_VERTICES);
            let mut i = 0u32;
            while f64::from(i) < count {
                let fi = f64::from(i);
                let a = -PI / 2.0 + (fi * PI * 2.0) / count;
                let radius = if star && i % 2 == 1 { inner } else { outer };
                let (x, y) = (w / 2.0 + js::cos(a) * radius, h / 2.0 + js::sin(a) * radius);
                if i == 0 { path.move_to(x, y) } else { path.line_to(x, y) }
                i += 1;
            }
            path.close_path();
        }
        ShapeKind::Line => {
            path.move_to(0.0, h / 2.0);
            path.line_to(w, h / 2.0);
            let width = js::max(1.0, js::or(d.stroke, 6.0));
            return Ok(ShapeGeometry {
                path: path.finish(),
                fill: false,
                stroke: Some(ShapeStroke {
                    style: Stroke::new(if width.is_finite() { width } else { 1.0 })
                        .with_join(Join::Round)
                        .with_caps(Cap::Round)
                        .with_miter_limit(CANVAS_MITER_LIMIT),
                    paint: StrokePaint::Color,
                }),
            });
        }
        ShapeKind::Rect => {
            let corners = d.corners();
            if corner::is_uniform_circular(&corners) {
                rr(&mut path, 0.0, 0.0, w, h, js::or0(d.radius))?;
            } else {
                return Ok(ShapeGeometry {
                    path: corner::rounded_rect_path(&corners),
                    fill: true,
                    stroke: shape_stroke(d),
                });
            }
        }
    }
    Ok(ShapeGeometry { path: path.finish(), fill: true, stroke: shape_stroke(d) })
}

/// Today's selection box in layer space (`shapeRasterGeometry().selection`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
    pub w: f64,
    pub h: f64,
}

impl Selection {
    pub fn rect(&self) -> Rect {
        Rect::new(self.x0, self.y0, self.x1, self.y1)
    }
}

/// `shapeRasterGeometry(d, scale)`: the bitmap today's renderer allocates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapeRaster {
    /// Transparent margin around the shape box, in shape units.
    pub pad: f64,
    /// Bitmap extent in shape units (`d.w + 2 pad`, `d.h + 2 pad`).
    pub w: f64,
    pub h: f64,
    /// Pixels per unit, capped so neither side exceeds 8,192 pixels.
    pub density: f64,
    /// Bitmap size in pixels.
    pub width: f64,
    pub height: f64,
    pub selection: Selection,
}

impl ShapeRaster {
    pub fn new(d: &ShapeContent, scale: f64) -> Self {
        let pad = libm::ceil(js::or0(d.stroke) / 2.0) + 4.0;
        let w = d.w + pad * 2.0;
        let h = d.h + pad * 2.0;
        let density = js::min(js::min(scale, MAX_RASTER_SIZE / w), MAX_RASTER_SIZE / h);
        let stroke = js::max(0.0, js::or0(d.stroke)) / 2.0;
        let selection = Selection {
            x0: -d.w / 2.0 - stroke,
            y0: -d.h / 2.0 - stroke,
            x1: d.w / 2.0 + stroke,
            y1: d.h / 2.0 + stroke,
            w: d.w + stroke * 2.0,
            h: d.h + stroke * 2.0,
        };
        Self {
            pad,
            w,
            h,
            density,
            width: js::max(1.0, libm::ceil(w * density)),
            height: js::max(1.0, libm::ceil(h * density)),
            selection,
        }
    }

    /// Shape space to bitmap pixels (the canvas transform `scale(density)`
    /// then `translate(pad, pad)`).
    pub fn shape_to_pixels(&self) -> Affine {
        Affine::new([self.density, 0.0, 0.0, self.density, self.density * self.pad, self.density * self.pad])
    }

    /// Shape space to layer space as today's compositor presents the bitmap:
    /// the `width` × `height` pixel bitmap is stretched over `w` × `h` units
    /// centered on the layer origin, so content shrinks by
    /// `w · density / width` when the pixel size rounds up.
    pub fn shape_to_layer(&self) -> Affine {
        let sx = self.w / self.width;
        let sy = self.h / self.height;
        Affine::new([sx, 0.0, 0.0, sy, -self.w / 2.0, -self.h / 2.0]) * self.shape_to_pixels()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::{PathEl, Point, Shape};

    fn shape(kind: ShapeKind, w: f64, h: f64) -> ShapeContent {
        ShapeContent { shape: kind, w, h, ..ShapeContent::default() }
    }

    #[test]
    fn star_alternates_radii() {
        let g = shape_geometry(&ShapeContent { points: 5.0, ..shape(ShapeKind::Star, 100.0, 100.0) }).unwrap();
        let els = g.path.elements();
        assert_eq!(els.len(), 11);
        assert_eq!(els[0], PathEl::MoveTo(Point::new(50.0 + js::cos(-PI / 2.0) * 50.0, 0.0)));
        let PathEl::LineTo(p) = els[1] else { panic!() };
        assert!(((p - Point::new(50.0, 50.0)).hypot() - 23.0).abs() < 1e-12);
    }

    #[test]
    fn polygon_defaults_to_five_points_and_at_least_three() {
        let five = shape_geometry(&shape(ShapeKind::Polygon, 10.0, 10.0)).unwrap();
        assert_eq!(five.path.elements().len(), 6);
        let three = shape_geometry(&ShapeContent { points: 1.0, ..shape(ShapeKind::Polygon, 10.0, 10.0) }).unwrap();
        assert_eq!(three.path.elements().len(), 4);
    }

    #[test]
    fn fractional_points_draw_the_extra_vertex() {
        // `for (i = 0; i < 5.5; i++)` runs six times.
        let g = shape_geometry(&ShapeContent { points: 5.5, ..shape(ShapeKind::Polygon, 10.0, 10.0) }).unwrap();
        assert_eq!(g.path.elements().len(), 7);
    }

    #[test]
    fn line_strokes_with_round_caps_and_the_fill_color() {
        let g = shape_geometry(&shape(ShapeKind::Line, 200.0, 20.0)).unwrap();
        assert!(!g.fill);
        let stroke = g.stroke.unwrap();
        assert_eq!(stroke.style.width, 6.0);
        assert_eq!(stroke.style.start_cap, Cap::Round);
        assert_eq!(stroke.paint, StrokePaint::Color);
    }

    #[test]
    fn negative_uniform_radius_fails_like_canvas() {
        let d = ShapeContent { radius: -5.0, ..shape(ShapeKind::Rect, 10.0, 10.0) };
        assert_eq!(shape_geometry(&d), Err(ShapeError::NegativeRadius));
    }

    #[test]
    fn square_rect_is_four_lines() {
        let g = shape_geometry(&shape(ShapeKind::Rect, 30.0, 20.0)).unwrap();
        assert_eq!(g.path.bounding_box(), Rect::new(0.0, 0.0, 30.0, 20.0));
        assert!(g.path.elements().iter().all(|e| !matches!(e, PathEl::CurveTo(..))));
    }

    #[test]
    fn raster_geometry_caps_density() {
        let r = ShapeRaster::new(&ShapeContent { stroke: 0.0, ..shape(ShapeKind::Rect, 100.1, 50.2) }, 1.25);
        assert_eq!(r.pad, 4.0);
        assert_eq!(r.width, libm::ceil(108.1 * 1.25));
        let big = ShapeRaster::new(&shape(ShapeKind::Rect, 1000.0, 1000.0), 32.0);
        assert_eq!(big.width, 8192.0);
        // The layer mapping keeps the box centered when the bitmap is exact.
        let exact = ShapeRaster::new(&ShapeContent { stroke: 0.0, ..shape(ShapeKind::Rect, 92.0, 42.0) }, 2.0);
        let m = exact.shape_to_layer();
        assert_eq!(m * Point::new(0.0, 0.0), Point::new(-46.0, -21.0));
        assert_eq!(m * Point::new(92.0, 42.0), Point::new(46.0, 21.0));
    }
}
