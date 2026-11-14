//! Integer local-grid regions. Changing a retained region must never change
//! the transform or origin of the original raster's pixel grid.
use kurbo::{Affine, Rect};
use pm_vector::render::scene::PixelRect;

/// Preserve the legacy resting glyph look near 100%, but do not magnify or
/// rotate a sampled bitmap. Reflections, nonuniform scales and skew use exact
/// device coverage. 3D callers retain their existing plane policy separately.
pub(crate) fn at_rest(m: &[f64; 6]) -> bool {
    m.iter().all(|v| v.is_finite())
        && m[1].abs() <= 1e-8
        && m[2].abs() <= 1e-8
        && (m[0] - m[3]).abs() <= 1e-6
        && (m[0] - 1.).abs() <= 0.05
}

/// The source texels read by a target rectangle through a LINEAR sampler.
/// Singular or invalid transforms retain the original source conservatively.
/// The extra two texels cover both the filter footprint and f32 vertex roundoff.
pub(crate) fn visible(
    content: PixelRect,
    pixels_to_target: Affine,
    target: Rect,
) -> Option<PixelRect> {
    if !pixels_to_target.as_coeffs().iter().all(|v| v.is_finite())
        || pixels_to_target.determinant() == 0.0
    {
        return Some(content);
    }
    let r = pixels_to_target
        .inverse()
        .transform_rect_bbox(target)
        .inflate(2., 2.);
    let x0 = r.x0.floor().max(f64::from(content.x));
    let y0 = r.y0.floor().max(f64::from(content.y));
    let x1 =
        r.x1.ceil()
            .min(f64::from(content.x) + f64::from(content.width));
    let y1 =
        r.y1.ceil()
            .min(f64::from(content.y) + f64::from(content.height));
    (x1 > x0 && y1 > y0)
        .then(|| PixelRect::new(x0 as i32, y0 as i32, (x1 - x0) as u32, (y1 - y0) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn crop_preserves_fractional_grid_and_bounds_work_at_large_zoom() {
        let full = PixelRect::new(0, 0, 8192, 8192);
        let crop = visible(
            full,
            Affine::translate((-3000.25, -2000.75)),
            Rect::new(0., 0., 1280., 720.),
        )
        .unwrap();
        assert_eq!(crop, PixelRect::new(2998, 1998, 1285, 725));
        assert!(crop.area() < full.area() / 60);
        assert_eq!(
            visible(full, Affine::scale(0.), Rect::new(0., 0., 1280., 720.)),
            Some(full)
        );
    }
    #[test]
    fn rotated_reflected_crop_contains_every_target_corner() {
        let full = PixelRect::new(-8192, -8192, 16384, 16384);
        let m = Affine::translate((200., 100.))
            * Affine::rotate(0.7)
            * Affine::scale_non_uniform(-2., 3.);
        let crop = visible(full, m, Rect::new(0., 0., 1280., 720.)).unwrap();
        let bounds = Rect::new(
            crop.x as f64,
            crop.y as f64,
            (crop.x as i64 + crop.width as i64) as f64,
            (crop.y as i64 + crop.height as i64) as f64,
        );
        for p in [(0., 0.), (1280., 0.), (0., 720.), (1280., 720.)] {
            assert!(bounds.contains(m.inverse() * kurbo::Point::from(p)));
        }
    }
    #[test]
    fn raster_effect_margin_is_mapped_back_at_the_original_density() {
        let content = PixelRect::new(0, 0, 8192, 8192);
        let map = Affine::translate((-3000., -2000.)) * Affine::scale(2.);
        let target = Rect::new(0., 0., 1280., 720.);
        let plain = visible(content, map, target).unwrap();
        let padded = visible(content, map, target.inflate(100., 100.)).unwrap();
        assert_eq!(padded.x, plain.x - 50);
        assert_eq!(padded.y, plain.y - 50);
        assert_eq!(padded.width, plain.width + 100);
        assert_eq!(padded.height, plain.height + 100);
    }
    #[test]
    fn raster_crop_keeps_ceil_canvas_anchor_and_every_pixel_center() {
        // Nonintegral layer extent with a ceil-rounded local canvas.
        let old = PixelRect::new(0, 0, 8192, 7777);
        let (sx, sy) = (8000.3 / 8192., 7000.7 / 7777.);
        let (ax, ay) = (4000.15, 3500.35);
        let world = Affine::translate((640.25, 360.75)) * Affine::scale(12.);
        let original = world * Affine::translate((-ax, -ay)) * Affine::scale_non_uniform(sx, sy);
        let r = visible(old, original, Rect::new(0., 0., 1280., 720.)).unwrap();
        let cropped = world
            * Affine::translate((-ax + r.x as f64 * sx, -ay + r.y as f64 * sy))
            * Affine::scale_non_uniform(sx, sy);
        for p in [(0.5, 0.5), (r.width as f64 - 0.5, r.height as f64 - 0.5)] {
            let a = original * kurbo::Point::new(p.0 + r.x as f64, p.1 + r.y as f64);
            let b = cropped * kurbo::Point::from(p);
            assert!((a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9);
        }
    }
    #[test]
    fn raster_gate_retains_resting_text_and_rejects_zoom_rotation_skew_and_flip() {
        assert!(at_rest(&[1., 0., 0., 1., 0.375, 12.5]));
        assert!(at_rest(&[1.025, 0., 0., 1.025, 0., 0.]));
        for m in [
            [12., 0., 0., 12., 0., 0.],
            [1., 0.12, -0.12, 1., 0., 0.],
            [1., 0., 0.01, 1., 0., 0.],
            [-1., 0., 0., 1., 0., 0.],
            [1., 0., 0., 1.01, 0., 0.],
        ] {
            assert!(!at_rest(&m));
        }
    }
}
