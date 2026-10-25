//! Platform glyph paths use the same Device::drawPath dispatch, stroker,
//! tessellation, viewport and per-sample N32 paint as Canvas2D shapes.
use super::{
    context::{Context, LineJoin},
    paint::Paint,
    skpath::SkPath,
};
use kurbo::{Affine, Rect};
use pm_text::render::{
    bitmap::Rgba8,
    scene::{Item, ItemContent, PathDraw, PathGlyph, PathRasterizer},
};
use std::sync::Arc;

pub struct Rasterizer;
pub static RASTERIZER: Rasterizer = Rasterizer;

impl PathRasterizer for Rasterizer {
    fn raster_pass(&self, draws: &[PathDraw], multisample_pass: bool) -> Option<Item> {
        let first = draws.first()?;
        let (viewport, blur) = (first.viewport, first.blur);
        debug_assert!(
            draws
                .iter()
                .all(|d| d.viewport == viewport && d.blur == blur),
            "one pass: one canvas, and filtered draws are never merged"
        );
        // A filtered draw can contribute ink from beyond the canvas edge.
        // Its save layer covers the canvas outset by the kernel bounds.
        let pad = pm_text::render::scene::blur_radius(blur);
        let mut cx = Context::new(viewport.0 + 2 * pad, viewport.1 + 2 * pad);
        cx.retain_native_paths();
        if multisample_pass {
            cx.multisample_pass();
        }
        cx.set_line_join(LineJoin::Round);
        cx.set_miter_limit(2.0);
        let mut bounds: Option<Rect> = None;
        for draw in draws {
            let alpha = (draw.color[3] * draw.opacity) as f32;
            let paint = Paint::Solid([
                draw.color[0] as f32 * alpha,
                draw.color[1] as f32 * alpha,
                draw.color[2] as f32 * alpha,
                alpha,
            ]);
            cx.set_fill_paint(paint.clone());
            cx.set_stroke_paint(paint);
            for glyph in &draw.glyphs {
                let p = sk_path(glyph);
                let mut matrix = glyph.matrix;
                matrix[4] += pad as f32;
                matrix[5] += pad as f32;
                cx.set_transform(matrix.map(f64::from));
                let m = Affine::new(matrix.map(f64::from));
                let b = glyph.path.bounds();
                let b = Rect::new(
                    b.left() as f64,
                    b.top() as f64,
                    b.right() as f64,
                    b.bottom() as f64,
                );
                let half = glyph.stroke.map_or(0.0, |w| w as f64 * 0.5);
                let b = m
                    .transform_rect_bbox(b.inflate(half, half))
                    .inflate(2.0, 2.0);
                bounds = Some(bounds.map_or(b, |a| a.union(b)));
                if let Some(width) = glyph.stroke {
                    cx.set_line_width(width as f64);
                    cx.stroke_path(&p).ok()?;
                } else {
                    cx.fill_path(&p).ok()?;
                }
            }
        }
        let b = bounds?.intersect(Rect::new(0.0, 0.0, cx.width() as f64, cx.height() as f64));
        let r = [
            b.x0.floor() as i32,
            b.y0.floor() as i32,
            b.x1.ceil() as i32,
            b.y1.ceil() as i32,
        ];
        if r[2] <= r[0] || r[3] <= r[1] {
            return None;
        }
        let data = cx.finish_region(r)?;
        Some(Item {
            content: ItemContent::Graphite {
                raster: Arc::new(cx.graphite_region(r)?),
                paths: Some(draws.into()),
                image: Arc::new(Rgba8 {
                    width: (r[2] - r[0]) as u32,
                    height: (r[3] - r[1]) as u32,
                    data,
                }),
                transform: Affine::translate((r[0] as f64 - pad as f64, r[1] as f64 - pad as f64)),
            },
            color: [1.0; 4],
            opacity: 1.0,
            blur,
            bounds: Rect::new(
                r[0] as f64 - pad as f64,
                r[1] as f64 - pad as f64,
                r[2] as f64 - pad as f64,
                r[3] as f64 - pad as f64,
            ),
        })
    }
}

/// A canonical outline as an `SkPath`. Verbs/points have identical
/// storage; no conversion to cubic or flattening occurs at this boundary
/// (conics retain weights).
fn sk_path(glyph: &PathGlyph) -> SkPath {
    let mut p = SkPath::new();
    let mut i = 0;
    let mut wi = 0;
    let points = glyph.path.points();
    let point = |i: usize| [points[i].x, points[i].y];
    use pm_text::render::scene::CanonicalVerb as V;
    for v in glyph.path.verbs() {
        match v {
            V::Move => {
                p.move_to(point(i));
                i += 1;
            }
            V::Line => {
                p.line_to(point(i));
                i += 1;
            }
            V::Quad => {
                p.quad_to(point(i), point(i + 1));
                i += 2;
            }
            V::Conic => {
                p.conic_to(point(i), point(i + 1), glyph.path.conic_weights()[wi]);
                i += 2;
                wi += 1;
            }
            V::Cubic => {
                p.cubic_to(point(i), point(i + 1), point(i + 2));
                i += 3;
            }
            V::Close => p.close(),
        }
    }
    p
}
