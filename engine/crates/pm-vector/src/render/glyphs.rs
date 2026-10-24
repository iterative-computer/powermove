//! Glyph paths a text canvas draws as paths (glyphs beyond the mask atlas):
//! covered analytically like every vector item, at their exact device
//! positions ([`pm_text::render::scene::PathRasterizer`]).
//!
//! Each `fillText`/`strokeText` call becomes one coverage item of the text
//! canvas: its glyph outlines placed by their matrices, filled nonzero or
//! stroked with round joins (Canvas2D's text stroke, miter limit 2), with
//! the call's paint, opacity and blur. Conics (Skia keeps them in canonical
//! glyph paths) are split into quadratics well below the coverage
//! tolerance.

use std::sync::Arc;

use kurbo::{Affine, BezPath, Join, Point, Rect, Shape, Stroke};
use pm_text::render::profile::LINEAR;
use pm_text::render::scene::{
    CanonicalVerb, Item, ItemContent, PathDraw, PathGlyph, PathRasterizer, PathStyle, PlacedPath,
};

/// The analytic glyph path renderer.
pub struct GlyphPaths;

pub static GLYPH_PATHS: GlyphPaths = GlyphPaths;

impl PathRasterizer for GlyphPaths {
    /// One item per paint. A text call's glyphs share one paint; only
    /// Graphite items are regrouped across calls, so several draws reach
    /// here only with the paint of their first.
    fn raster_pass(&self, draws: &[PathDraw], _multisample_pass: bool) -> Option<Item> {
        let first = draws.first()?;
        debug_assert!(draws.iter().all(|d| d.color == first.color
            && d.opacity == first.opacity
            && d.blur == first.blur));
        let mut paths = Vec::new();
        let mut bounds: Option<Rect> = None;
        for glyph in draws.iter().flat_map(|d| &d.glyphs) {
            let path = outline(glyph);
            let transform = Affine::new(glyph.matrix.map(f64::from));
            let style = match glyph.stroke {
                None => PathStyle::Fill,
                Some(width) => PathStyle::Stroke(
                    Stroke::new(f64::from(width)).with_join(Join::Round).with_miter_limit(2.0),
                ),
            };
            let half = glyph.stroke.map_or(0.0, |w| f64::from(w) / 2.0);
            let b = transform.transform_rect_bbox(path.bounding_box().inflate(half, half));
            if !(b.x0.is_finite() && b.y0.is_finite() && b.x1.is_finite() && b.y1.is_finite()) {
                continue;
            }
            bounds = Some(bounds.map_or(b, |a| a.union(b)));
            paths.push(PlacedPath {
                path: Arc::new(path),
                transform,
                style,
            });
        }
        Some(Item {
            content: ItemContent::Coverage {
                paths,
                curve: LINEAR,
            },
            color: first.color,
            opacity: first.opacity,
            blur: first.blur,
            bounds: bounds?,
        })
    }
}

/// A canonical glyph path as a kurbo path (conics as quadratics).
fn outline(glyph: &PathGlyph) -> BezPath {
    let points = glyph.path.points();
    let weights = glyph.path.conic_weights();
    let p = |i: usize| Point::new(f64::from(points[i].x), f64::from(points[i].y));
    let mut out = BezPath::new();
    let (mut i, mut wi) = (0, 0);
    let mut last = Point::ZERO;
    for verb in glyph.path.verbs() {
        match verb {
            CanonicalVerb::Move => {
                last = p(i);
                out.move_to(last);
                i += 1;
            }
            CanonicalVerb::Line => {
                last = p(i);
                out.line_to(last);
                i += 1;
            }
            CanonicalVerb::Quad => {
                out.quad_to(p(i), p(i + 1));
                last = p(i + 1);
                i += 2;
            }
            CanonicalVerb::Conic => {
                conic_to(&mut out, last, p(i), p(i + 1), f64::from(weights[wi]));
                last = p(i + 1);
                i += 2;
                wi += 1;
            }
            CanonicalVerb::Cubic => {
                out.curve_to(p(i), p(i + 1), p(i + 2));
                last = p(i + 2);
                i += 3;
            }
            CanonicalVerb::Close => out.close_path(),
        }
    }
    out
}

/// Appends the conic `p0 p1 p2` with weight `w` as 16 quadratics: each a
/// sixteenth of the conic by exact rational subdivision, its control point
/// where the conic's tangents at the piece's ends meet. For a quarter
/// circle of radius 1000 px the pieces stay within 1e-4 px of the arc.
fn conic_to(out: &mut BezPath, p0: Point, p1: Point, p2: Point, w: f64) {
    const PIECES: usize = 16;
    // The conic in homogeneous form.
    let at = |t: f64| {
        let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t) * w, t * t);
        let d = a + b + c;
        Point::new((a * p0.x + b * p1.x + c * p2.x) / d, (a * p0.y + b * p1.y + c * p2.y) / d)
    };
    // The derivative's direction (numerator of the quotient rule, scaled).
    let tangent = |t: f64| {
        let (a, b, c) = ((1.0 - t) * (1.0 - t), 2.0 * t * (1.0 - t) * w, t * t);
        let (da, db, dc) = (-2.0 * (1.0 - t), 2.0 * w * (1.0 - 2.0 * t), 2.0 * t);
        let d = a + b + c;
        let dd = da + db + dc;
        let n = a * p0.to_vec2() + b * p1.to_vec2() + c * p2.to_vec2();
        let dn = da * p0.to_vec2() + db * p1.to_vec2() + dc * p2.to_vec2();
        dn * d - n * dd
    };
    let mut start = p0;
    for k in 0..PIECES {
        let (t0, t1) = (k as f64 / PIECES as f64, (k + 1) as f64 / PIECES as f64);
        let end = if k + 1 == PIECES { p2 } else { at(t1) };
        let (d0, d1) = (tangent(t0), tangent(t1));
        let cross = d0.cross(d1);
        let control = if cross.abs() > 1e-12 * d0.hypot() * d1.hypot() {
            let s = (end - start).cross(d1) / cross;
            start + d0 * s
        } else {
            start.midpoint(end)
        };
        out.quad_to(control, end);
        start = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A quarter circle as a conic (weight √2/2) stays on the circle.
    #[test]
    fn conics_follow_the_circle() {
        let mut path = BezPath::new();
        path.move_to((1000.0, 0.0));
        conic_to(
            &mut path,
            Point::new(1000.0, 0.0),
            Point::new(1000.0, 1000.0),
            Point::new(0.0, 1000.0),
            core::f64::consts::FRAC_1_SQRT_2,
        );
        let mut worst: f64 = 0.0;
        kurbo::flatten(path.iter(), 1e-6, |el| {
            if let kurbo::PathEl::LineTo(q) = el {
                worst = worst.max((q.to_vec2().hypot() - 1000.0).abs());
            }
        });
        assert!(worst < 1e-4, "{worst}");
    }
}
