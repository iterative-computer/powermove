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
            draws.iter().all(|d| d.viewport == viewport && d.blur == blur),
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
                let b = m.transform_rect_bbox(b.inflate(half, half)).inflate(2.0, 2.0);
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

#[cfg(test)]
mod tests {
    use super::super::skpath::SkPath;
    use super::*;

    fn polygon(points: &[(f32, f32)]) -> Arc<pm_skia_path::Path> {
        let mut b = pm_skia_path::PathBuilder::new();
        b.move_to(points[0].0, points[0].1);
        for p in &points[1..] {
            b.line_to(p.0, p.1);
        }
        b.close();
        Arc::new(b.finish().unwrap())
    }

    /// The premultiplied pixels of unfiltered items composited source-over
    /// into an RGBA8 canvas (the items here never overlap).
    fn composite(items: &[Item], size: (u32, u32)) -> Vec<u8> {
        let mut out = vec![0u8; (size.0 * size.1 * 4) as usize];
        for item in items {
            let ItemContent::Graphite { image, transform, .. } = &item.content else {
                panic!("a Graphite item")
            };
            let [.., tx, ty] = transform.as_coeffs();
            for y in 0..image.height as usize {
                for x in 0..image.width as usize {
                    let s = &image.data[(y * image.width as usize + x) * 4..][..4];
                    if s[3] == 0 {
                        continue;
                    }
                    let (dx, dy) = (x + tx as usize, y + ty as usize);
                    let d = &mut out[(dy * size.0 as usize + dx) * 4..][..4];
                    assert_eq!(d[3], 0, "items overlap at {dx},{dy}");
                    d.copy_from_slice(s);
                }
            }
        }
        out
    }

    fn multisampled(item: &Item) -> bool {
        match &item.content {
            ItemContent::Graphite { raster, .. } => raster.multisampled,
            _ => unreachable!(),
        }
    }

    const INK: [f64; 4] = [0.0627, 0.0627, 0.0627, 1.0];

    fn call(glyphs: &[PathGlyph], viewport: (u32, u32), blur: f64) -> Item {
        RASTERIZER.raster(glyphs, viewport, INK, 1.0, blur).unwrap()
    }

    /// Source-over of premultiplied RGBA8 items, in order (overlaps allowed).
    fn composite_over(items: &[Item], size: (u32, u32)) -> Vec<u8> {
        let mut out = vec![0f32; (size.0 * size.1 * 4) as usize];
        for item in items {
            let (ItemContent::Graphite { image, transform, .. }
            | ItemContent::Image { image, transform }) = &item.content
            else {
                panic!("an image item")
            };
            let [.., tx, ty] = transform.as_coeffs();
            for y in 0..image.height as usize {
                for x in 0..image.width as usize {
                    let s = &image.data[(y * image.width as usize + x) * 4..][..4];
                    let (dx, dy) = (x + tx as usize, y + ty as usize);
                    let d = &mut out[(dy * size.0 as usize + dx) * 4..][..4];
                    let inv = 1.0 - f32::from(s[3]) / 255.0;
                    for c in 0..4 {
                        d[c] = f32::from(s[c]) / 255.0 + d[c] * inv;
                    }
                }
            }
        }
        out.iter().map(|v| (v * 255.0).round() as u8).collect()
    }

    // Graphite multisamples a whole render pass once one of its draws needs
    // MSAA. A skewed rectangular glyph (Arial `l`, an analytic draw) drawn by
    // its own `fillText` next to a tessellated glyph (`H`) must cover samples
    // at its acute corners, exactly as when both are in one call.
    #[test]
    fn a_multisampled_pass_draws_earlier_analytic_glyphs_per_sample() {
        use pm_text::render::scene::resolve_canvas_pass;
        let viewport = (420, 120);
        let skew = [1.875f32, 0.0, -3.247_595_3, 1.875, 0.0, 0.0];
        let at = |x: f32| {
            let mut m = skew;
            m[4] = x;
            m[5] = 100.0;
            m
        };
        // `l`: a rectangle (analytic). `H`: a concave outline (stencil and
        // cover, MSAA). Canonical 64 px units, y down.
        let l = PathGlyph {
            path: polygon(&[(4.4, -45.8), (10.1, -45.8), (10.1, 0.0), (4.4, 0.0)]),
            matrix: at(240.3),
            stroke: None,
        };
        let h = PathGlyph {
            path: polygon(&[
                (5.0, -45.8),
                (11.1, -45.8),
                (11.1, -27.0),
                (34.9, -27.0),
                (34.9, -45.8),
                (41.0, -45.8),
                (41.0, 0.0),
                (34.9, 0.0),
                (34.9, -21.6),
                (11.1, -21.6),
                (11.1, 0.0),
                (5.0, 0.0),
            ]),
            matrix: at(10.1),
            stroke: None,
        };
        let one_call = composite(&[call(&[l.clone(), h.clone()], viewport, 0.0)], viewport);

        // Separate calls: the rectangle alone resolves pixel centers.
        let mut items = vec![
            call(std::slice::from_ref(&l), viewport, 0.0),
            call(std::slice::from_ref(&h), viewport, 0.0),
        ];
        assert!(!multisampled(&items[0]) && multisampled(&items[1]));
        let separate = composite(&items, viewport);
        let corners = one_call.iter().zip(&separate).filter(|(a, b)| a != b).count();
        assert!(corners > 0, "the corner samples must matter for this test");

        resolve_canvas_pass(&mut items, &RASTERIZER, false);
        assert_eq!(items.len(), 1);
        assert!(multisampled(&items[0]));
        assert_eq!(composite(&items, viewport), one_call);

        // A filtered draw renders in its own save layer's pass.
        let mut pair = vec![
            call(std::slice::from_ref(&l), viewport, 1.5),
            call(std::slice::from_ref(&h), viewport, 0.0),
        ];
        resolve_canvas_pass(&mut pair, &RASTERIZER, false);
        assert_eq!(pair.len(), 2);
        assert!(!multisampled(&pair[0]));
        // A pass made multisampled by a draw outside the items (a caption
        // box) redraws them too; alone, nothing changes.
        let mut alone = vec![call(std::slice::from_ref(&l), viewport, 0.0)];
        resolve_canvas_pass(&mut alone, &RASTERIZER, false);
        assert!(!multisampled(&alone[0]));
        resolve_canvas_pass(&mut alone, &RASTERIZER, true);
        assert!(multisampled(&alone[0]));
    }

    // Two tessellated draws of one pass that share a slanted edge blend per
    // sample: every sample along the seam is covered by one of them, so the
    // seam is solid. Resolving each draw first and compositing the pixels
    // leaves a lighter seam (rotated Zapfino `g`s of an animator).
    #[test]
    fn overlapping_draws_of_a_pass_blend_per_sample() {
        use pm_text::render::scene::resolve_canvas_pass;
        let viewport = (64, 64);
        let identity = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        let glyph = |points: &[(f32, f32)]| PathGlyph {
            path: polygon(points),
            matrix: identity,
            stroke: None,
        };
        // Two concave halves of a square split along a slanted seam.
        let a = glyph(&[(8.0, 8.0), (40.3, 8.0), (20.1, 56.0), (8.0, 56.0), (14.0, 30.0)]);
        let b = glyph(&[(40.3, 8.0), (56.0, 8.0), (56.0, 56.0), (20.1, 56.0), (50.0, 30.0)]);
        let one_call = composite_over(&[call(&[a.clone(), b.clone()], viewport, 0.0)], viewport);
        let mut items = vec![
            call(std::slice::from_ref(&a), viewport, 0.0),
            call(std::slice::from_ref(&b), viewport, 0.0),
        ];
        assert!(multisampled(&items[0]) && multisampled(&items[1]));
        let separate = composite_over(&items, viewport);
        assert_ne!(separate, one_call, "the seam must matter for this test");
        resolve_canvas_pass(&mut items, &RASTERIZER, false);
        assert_eq!(items.len(), 1);
        assert_eq!(composite_over(&items, viewport), one_call);
    }

    // The merged pass takes the first draw's place. An item drawn between
    // two path draws keeps its order with the later one where their ink
    // meets; elsewhere the later draw joins the pass.
    #[test]
    fn a_pass_keeps_the_order_of_items_between_its_draws() {
        use pm_text::render::scene::resolve_canvas_pass;
        let viewport = (64, 64);
        let identity = [1.0f32, 0.0, 0.0, 1.0, 0.0, 0.0];
        let glyph = |points: &[(f32, f32)]| PathGlyph {
            path: polygon(points),
            matrix: identity,
            stroke: None,
        };
        // The two concave halves of a square with a slanted seam.
        let a = glyph(&[(8.0, 8.0), (40.3, 8.0), (20.1, 56.0), (8.0, 56.0), (14.0, 30.0)]);
        let b = glyph(&[(40.3, 8.0), (56.0, 8.0), (56.0, 56.0), (20.1, 56.0), (50.0, 30.0)]);
        let square = |x: f64, y: f64| Item {
            content: ItemContent::Image {
                image: Arc::new(Rgba8 {
                    width: 4,
                    height: 4,
                    data: [0, 200, 0, 255].repeat(16),
                }),
                transform: Affine::translate((x, y)),
            },
            color: [1.0; 4],
            opacity: 1.0,
            blur: 0.0,
            bounds: Rect::new(x, y, x + 4.0, y + 4.0),
        };
        let draw = |g: &PathGlyph, opacity: f64| {
            RASTERIZER
                .raster(std::slice::from_ref(g), viewport, INK, opacity, 0.0)
                .unwrap()
        };
        let pass = |opacity: f64| {
            RASTERIZER
                .raster_pass(
                    &[
                        PathDraw { glyphs: vec![a.clone()], viewport, color: INK, opacity: 1.0, blur: 0.0 },
                        PathDraw { glyphs: vec![b.clone()], viewport, color: INK, opacity, blur: 0.0 },
                    ],
                    true,
                )
                .unwrap()
        };
        // Nothing between the draws touches `b`: one pass, solid seam.
        let mut items = vec![draw(&a, 1.0), square(0.0, 0.0), draw(&b, 1.0)];
        resolve_canvas_pass(&mut items, &RASTERIZER, false);
        assert_eq!(items.len(), 2);
        assert_eq!(composite_over(&items, viewport), composite_over(&[pass(1.0), square(0.0, 0.0)], viewport));

        // A square under `b`'s opaque interior: `b` still covers it, and the
        // seam is still drawn as one pass.
        let original = vec![draw(&a, 1.0), square(46.0, 40.0), draw(&b, 1.0)];
        let mut items = original.clone();
        resolve_canvas_pass(&mut items, &RASTERIZER, false);
        assert!(items.len() > 3, "split around the square: {}", items.len());
        let resolved = composite_over(&items, viewport);
        assert_eq!(resolved, composite_over(&[pass(1.0)], viewport));
        assert_ne!(resolved, composite_over(&original, viewport), "the seam must matter");

        // Under a translucent `b`, the square stays below it; the seam is
        // still one pass outside the square's box.
        let original = vec![draw(&a, 1.0), square(46.0, 40.0), draw(&b, 0.5)];
        let mut items = original.clone();
        resolve_canvas_pass(&mut items, &RASTERIZER, false);
        let resolved = composite_over(&items, viewport);
        let in_order = composite_over(&original, viewport);
        let merged = composite_over(&[pass(0.5)], viewport);
        for y in 0..64usize {
            for x in 0..64usize {
                let px = |v: &[u8]| v[(y * 64 + x) * 4..][..4].to_vec();
                let in_box = (45..51).contains(&x) && (39..45).contains(&y);
                let expected = if in_box { px(&in_order) } else { px(&merged) };
                assert_eq!(px(&resolved), expected, "pixel {x},{y}");
            }
        }
    }

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
