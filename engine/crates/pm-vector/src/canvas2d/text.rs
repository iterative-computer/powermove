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
    color::Rgba,
    scene::{Item, ItemContent, PathGlyph, PathRasterizer},
};
use std::sync::Arc;

pub struct Rasterizer;
pub static RASTERIZER: Rasterizer = Rasterizer;

impl PathRasterizer for Rasterizer {
    fn raster(
        &self,
        paths: &[PathGlyph],
        viewport: (u32, u32),
        color: Rgba,
        opacity: f64,
        blur: f64,
    ) -> Option<Item> {
        let mut cx = Context::new(viewport.0, viewport.1);
        let alpha = (color[3] * opacity) as f32;
        let paint = Paint::Solid([
            color[0] as f32 * alpha,
            color[1] as f32 * alpha,
            color[2] as f32 * alpha,
            alpha,
        ]);
        cx.set_fill_paint(paint.clone());
        cx.set_stroke_paint(paint);
        cx.set_line_join(LineJoin::Round);
        cx.set_miter_limit(2.0);
        let mut bounds: Option<Rect> = None;
        for glyph in paths {
            let mut p = SkPath::new();
            // Verbs/points have identical storage; no conversion to cubic
            // or flattening occurs at this boundary (conics retain weights).
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
            cx.set_transform(glyph.matrix.map(f64::from));
            let m = Affine::new(glyph.matrix.map(f64::from));
            let b = glyph.path.bounds();
            let b = Rect::new(
                b.left() as f64,
                b.top() as f64,
                b.right() as f64,
                b.bottom() as f64,
            );
            let pad = glyph.stroke.map_or(0.0, |w| w as f64 * 0.5);
            let b = m.transform_rect_bbox(b.inflate(pad, pad)).inflate(2.0, 2.0);
            bounds = Some(bounds.map_or(b, |a| a.union(b)));
            if let Some(width) = glyph.stroke {
                cx.set_line_width(width as f64);
                cx.stroke_path(&p).ok()?;
            } else {
                cx.fill_path(&p).ok()?;
            }
        }
        let b = bounds?.intersect(Rect::new(0.0, 0.0, viewport.0 as f64, viewport.1 as f64));
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
            content: ItemContent::Image {
                image: Arc::new(Rgba8 {
                    width: (r[2] - r[0]) as u32,
                    height: (r[3] - r[1]) as u32,
                    data,
                }),
                transform: Affine::translate((r[0] as f64, r[1] as f64)),
            },
            color: [1.0; 4],
            opacity: 1.0,
            blur,
            bounds: Rect::new(r[0] as f64, r[1] as f64, r[2] as f64, r[3] as f64),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::skpath::SkPath;
    use super::*;

    // A crop must not act like a smaller backing canvas. In particular,
    // near-grid vertices retain the original viewport's rounding.
    #[test]
    fn crop_preserves_viewport_corners_and_analytic_coverage() {
        for stroked in [false, true] {
            let mut cx = Context::new(1919, 1079);
            cx.set_fill_paint(Paint::Solid([0.3, 0.1, 0.7, 0.8]));
            cx.set_stroke_paint(Paint::Solid([0.3, 0.1, 0.7, 0.8]));
            cx.set_line_width(7.0);
            cx.set_line_join(LineJoin::Round);
            cx.set_transform([1.137, 0.03, -0.07, 1.13, 300.004, 250.002]);
            let mut p = SkPath::new();
            p.move_to([0.0, 0.0]);
            p.quad_to([60.003, 140.001], [120.0, 0.0]);
            p.line_to([120.0, 80.0]);
            p.line_to([0.0, 80.0]);
            p.close();
            if stroked {
                cx.stroke_path(&p).unwrap();
            } else {
                cx.fill_path(&p).unwrap();
            }
            let mut rect = SkPath::new();
            rect.move_to([5.0625, 10.0]);
            rect.line_to([9.0625, 10.0]);
            rect.line_to([9.0625, 70.0]);
            rect.line_to([5.0625, 70.0]);
            rect.close();
            cx.fill_path(&rect).unwrap();
            let full = cx.finish();
            let r = [280, 230, 460, 410];
            let crop = cx.finish_region(r).unwrap();
            for y in 0..180usize {
                let a = ((y + 230) * 1919 + 280) * 4;
                assert_eq!(
                    &crop[y * 180 * 4..(y + 1) * 180 * 4],
                    &full[a..a + 180 * 4],
                    "row {y}, stroke {stroked}"
                );
            }
        }
    }
}
