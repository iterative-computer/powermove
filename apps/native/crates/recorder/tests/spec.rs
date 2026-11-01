//! Spec-derived assertions. Expected pixels come from elementary area/compositing
//! arithmetic or independently clipped polygons; never captured from Chromium.
use pm_recorder::*;
fn canvas(w: u32, h: u32) -> Canvas {
    Canvas::new(1, w, h).unwrap()
}
fn pixel(c: &mut Canvas, x: i32, y: i32) -> [u8; 4] {
    c.get_image_data(x, y, 1, 1)
        .unwrap()
        .bytes()
        .try_into()
        .unwrap()
}
fn rgba(c: &mut Canvas) -> Vec<u8> {
    c.snapshot().unwrap().pixels.bytes().to_vec()
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-5, "{a} != {b}");
}
fn fill(c: &mut Canvas, color: &str, r: [f64; 4]) {
    c.set_fill_color(color).unwrap();
    c.fill_rect(r[0], r[1], r[2], r[3]).unwrap();
}

#[test]
fn default_state_and_context_attributes() {
    let c = canvas(2, 2);
    let a = c.context_attributes();
    assert!(a.alpha);
    assert_eq!((a.color_space, a.color_type), ("srgb", "unorm8"));
    assert!(!c.is_context_lost());
    assert_eq!(c.state().line.width, 1.);
    assert_eq!(c.state().line.miter_limit, 10.);
    assert_eq!(c.state().font.desc.size, 10.);
    assert_eq!(c.key(), SourceKey { id: 1, version: 0 });
}
#[test]
fn state_stack_preserves_current_path() {
    let mut c = canvas(4, 1);
    c.save();
    c.set_fill_color("red").unwrap();
    c.translate(2., 0.);
    c.rect(0., 0., 2., 1.);
    c.restore();
    c.restore();
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
    assert_eq!(pixel(&mut c, 2, 0), [0, 0, 0, 255]);
}
#[test]
fn transform_order_is_postmultiplication() {
    let mut c = canvas(12, 1);
    c.translate(2., 0.);
    c.scale(2., 1.);
    fill(&mut c, "red", [1., 0., 2., 1.]);
    assert_eq!(pixel(&mut c, 4, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&mut c, 8, 0), [0; 4]);
}
#[test]
fn path_points_capture_transform_at_construction() {
    let mut c = canvas(6, 2);
    c.rect(0., 0., 2., 2.);
    c.translate(4., 0.);
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&mut c, 4, 0), [0; 4]);
}
#[test]
fn rect_paint_does_not_change_current_path() {
    let mut c = canvas(2, 2);
    c.move_to(7., 8.);
    let p = c.path().elements().to_vec();
    c.fill_rect(0., 0., 2., 2.).unwrap();
    c.clear_rect(0., 0., 1., 1.).unwrap();
    c.stroke_rect(1., 1., 1., 1.).unwrap();
    assert_eq!(c.path().elements(), p);
}
#[test]
fn invalid_setters_and_nonfinite_geometry_are_ignored() {
    let mut c = canvas(2, 2);
    c.set_fill_color("red").unwrap();
    c.set_fill_color("bad color").unwrap();
    c.set_global_alpha(-1.);
    c.set_global_alpha(f64::NAN);
    c.set_line_width(0.);
    c.set_miter_limit(f64::INFINITY);
    c.set_composite("invalid");
    c.translate(f64::NAN, 1.);
    c.line_to(f64::NAN, 0.);
    c.fill_rect(0., 0., 2., 2.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
    assert_eq!(c.state().line.width, 1.);
}
#[test]
fn analytic_fractional_rectangle_area() {
    let mut c = canvas(2, 2);
    fill(&mut c, "red", [0.25, 0.25, 1., 1.]);
    assert_eq!(
        rgba(&mut c),
        vec![255, 0, 0, 143, 255, 0, 0, 48, 255, 0, 0, 48, 255, 0, 0, 16]
    );
}
#[test]
fn analytic_triangle_half_pixel() {
    let mut c = canvas(1, 1);
    c.move_to(0., 0.);
    c.line_to(1., 0.);
    c.line_to(0., 1.);
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0, 0, 0, 128]);
}
#[test]
fn all_windings_are_resolved_before_area_coverage() {
    let mut c = canvas(1, 1);
    c.rect(0., 0., 1., 1.);
    c.rect(0., 0., 1., 1.);
    c.fill(FillRule::EvenOdd).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0, 0, 0, 255]);
}
#[test]
fn fill_implicitly_closes_open_subpaths() {
    let mut c = canvas(1, 1);
    c.move_to(0., 0.);
    c.line_to(1., 0.);
    c.line_to(0., 1.);
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 0, 0)[3], 128);
}
#[test]
fn clip_is_intersection_not_multiplied_antialias_masks() {
    let mut c = canvas(1, 1);
    c.rect(0., 0., 0.5, 1.);
    c.clip(FillRule::NonZero);
    c.clip(FillRule::NonZero);
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 0, 0)[3], 128);
}
#[test]
fn clip_empty_path_clips_everything_and_restore_recovers() {
    let mut c = canvas(1, 1);
    c.save();
    c.clip(FillRule::NonZero);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
    c.restore();
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0, 0, 0, 255]);
}
#[test]
fn clear_obeys_transform_and_clip_ignores_alpha_composite() {
    let mut c = canvas(3, 1);
    fill(&mut c, "red", [0., 0., 3., 1.]);
    c.rect(1., 0., 1., 1.);
    c.clip(FillRule::NonZero);
    c.set_global_alpha(0.);
    c.set_composite("source-in");
    c.translate(1., 0.);
    c.clear_rect(0., 0., 2., 1.).unwrap();
    assert_eq!(pixel(&mut c, 1, 0), [0; 4]);
    assert_eq!(pixel(&mut c, 2, 0), [255, 0, 0, 255]);
}
#[test]
fn float_paint_accumulates_without_8bit_rounding_between_draws() {
    let mut c = canvas(1, 1);
    c.set_global_alpha(0.001);
    for _ in 0..100 {
        c.fill_rect(0., 0., 1., 1.).unwrap();
    }
    assert_eq!(pixel(&mut c, 0, 0)[3], 24);
}
#[test]
fn every_porter_duff_operator_has_expected_alpha_and_color() {
    // Opaque red destination, opaque blue source. All factors known exactly.
    for (op, want) in [
        ("source-over", [0, 0, 255, 255]),
        ("source-in", [0, 0, 255, 255]),
        ("source-out", [0; 4]),
        ("source-atop", [0, 0, 255, 255]),
        ("destination-over", [255, 0, 0, 255]),
        ("destination-in", [255, 0, 0, 255]),
        ("destination-out", [0; 4]),
        ("destination-atop", [255, 0, 0, 255]),
        ("copy", [0, 0, 255, 255]),
        ("xor", [0; 4]),
        ("lighter", [255, 0, 255, 255]),
    ] {
        let mut c = canvas(1, 1);
        fill(&mut c, "red", [0., 0., 1., 1.]);
        c.set_composite(op);
        fill(&mut c, "blue", [0., 0., 1., 1.]);
        assert_eq!(pixel(&mut c, 0, 0), want, "{op}");
    }
}
#[test]
fn unbounded_composites_clear_outside_shape_but_inside_clip() {
    for op in [
        "copy",
        "source-in",
        "source-out",
        "destination-in",
        "destination-atop",
    ] {
        let mut c = canvas(3, 1);
        fill(&mut c, "red", [0., 0., 3., 1.]);
        c.rect(0., 0., 2., 1.);
        c.clip(FillRule::NonZero);
        c.set_composite(op);
        fill(&mut c, "blue", [0., 0., 1., 1.]);
        assert_eq!(pixel(&mut c, 1, 0), [0; 4], "{op}");
        assert_eq!(pixel(&mut c, 2, 0), [255, 0, 0, 255], "{op}");
    }
}
#[test]
fn transparent_copy_clears_destination() {
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    c.set_composite("copy");
    fill(&mut c, "transparent", [0., 0., 1., 1.]);
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
}
#[test]
fn separable_blends_follow_independent_primary_color_reference() {
    for (op, want) in [
        ("multiply", [0, 0, 0, 255]),
        ("screen", [255, 0, 255, 255]),
        ("overlay", [255, 0, 0, 255]),
        ("darken", [0, 0, 0, 255]),
        ("lighten", [255, 0, 255, 255]),
        ("color-dodge", [255, 0, 0, 255]),
        ("color-burn", [255, 0, 0, 255]),
        ("hard-light", [0, 0, 255, 255]),
        ("soft-light", [255, 0, 0, 255]),
        ("difference", [255, 0, 255, 255]),
        ("exclusion", [255, 0, 255, 255]),
    ] {
        let mut c = canvas(1, 1);
        fill(&mut c, "red", [0., 0., 1., 1.]);
        c.set_composite(op);
        fill(&mut c, "blue", [0., 0., 1., 1.]);
        assert_eq!(pixel(&mut c, 0, 0), want, "{op}");
    }
}
#[test]
fn hsl_blends_are_finite_and_preserve_opaque_alpha() {
    for op in ["hue", "saturation", "color", "luminosity"] {
        let mut c = canvas(1, 1);
        fill(&mut c, "#804020", [0., 0., 1., 1.]);
        c.set_composite(op);
        fill(&mut c, "#2080c0", [0., 0., 1., 1.]);
        let p = pixel(&mut c, 0, 0);
        assert_eq!(p[3], 255);
        assert!(p[..3].iter().any(|v| *v > 0));
    }
}
#[test]
fn gradients_live_handle_and_draw_time_snapshot() {
    let g = Gradient::new(GradientKind::Linear(Point::new(0., 0.), Point::new(1., 0.))).unwrap();
    g.add_color_stop(0., "red").unwrap();
    let mut c = canvas(2, 1);
    c.set_fill(Paint::Gradient(g.clone()));
    c.save();
    c.fill_rect(0., 0., 1., 1.).unwrap();
    g.add_color_stop(1., "blue").unwrap();
    c.restore();
    c.fill_rect(1., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
    assert_eq!(pixel(&mut c, 1, 0), [0, 0, 255, 255]);
}
#[test]
fn gradient_uses_render_time_transform() {
    let g = Gradient::new(GradientKind::Linear(Point::ZERO, Point::new(1., 0.))).unwrap();
    g.add_color_stop(0., "red").unwrap();
    g.add_color_stop(1., "blue").unwrap();
    let mut c = canvas(2, 1);
    c.translate(1., 0.);
    c.set_fill(Paint::Gradient(g));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 1, 0), [128, 0, 128, 255]);
}
#[test]
fn gradient_alpha_interpolates_without_premultiplying() {
    let g = Gradient::new(GradientKind::Linear(Point::ZERO, Point::new(1., 0.))).unwrap();
    g.add_color_stop(0., "rgba(255,0,0,0)").unwrap();
    g.add_color_stop(1., "blue").unwrap();
    let mut c = canvas(1, 1);
    c.set_fill(Paint::Gradient(g));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [128, 0, 128, 128]);
}
#[test]
fn gradient_errors_are_atomic_and_zero_length_is_transparent() {
    let g = Gradient::new(GradientKind::Linear(Point::ZERO, Point::ZERO)).unwrap();
    assert_eq!(g.add_color_stop(-1., "red"), Err(Error::IndexSize));
    assert_eq!(g.add_color_stop(0., "bad"), Err(Error::Syntax));
    g.add_color_stop(0., "red").unwrap();
    let mut c = canvas(1, 1);
    c.set_fill(Paint::Gradient(g));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
}
#[test]
fn radial_and_conic_gradients_sample_defined_coordinates() {
    let mut c = canvas(1, 1);
    let g = Gradient::new(GradientKind::Radial {
        start: Point::new(0., 0.5),
        end: Point::new(0., 0.5),
        r0: 0.,
        r1: 1.,
    })
    .unwrap();
    g.add_color_stop(0., "black").unwrap();
    g.add_color_stop(1., "white").unwrap();
    c.set_fill(Paint::Gradient(g));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [128, 128, 128, 255]);
    let g = Gradient::new(GradientKind::Conic {
        angle: 0.,
        center: Point::new(0., 0.5),
    })
    .unwrap();
    g.add_color_stop(0., "red").unwrap();
    g.add_color_stop(1., "blue").unwrap();
    c.set_fill(Paint::Gradient(g));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
fn source(id: u64, version: u64, bytes: Vec<u8>, w: u32, h: u32) -> Source {
    Sources::default()
        .insert(
            SourceKey { id, version },
            ImageData::from_rgba(w, h, bytes).unwrap(),
        )
        .unwrap()
}
#[test]
fn draw_image_captures_source_version_before_repaint_and_retirement() {
    let mut a = Canvas::new(10, 1, 1).unwrap();
    fill(&mut a, "red", [0., 0., 1., 1.]);
    let old = a.snapshot().unwrap();
    let mut b = canvas(1, 1);
    b.draw_image(
        old.clone(),
        Rect::new(0., 0., 1., 1.),
        Rect::new(0., 0., 1., 1.),
    )
    .unwrap();
    fill(&mut a, "blue", [0., 0., 1., 1.]);
    assert_eq!(b.commands()[0].source_key(), Some(old.key));
    drop(old);
    drop(a);
    assert_eq!(pixel(&mut b, 0, 0), [255, 0, 0, 255]);
}
#[test]
fn registry_rejects_mutating_same_source_version() {
    let mut s = Sources::default();
    let key = SourceKey { id: 99, version: 1 };
    let a = ImageData::from_rgba(1, 1, vec![255, 0, 0, 255]).unwrap();
    let held = s.insert(key, a.clone()).unwrap();
    s.insert(key, a).unwrap();
    assert_eq!(
        s.insert(key, ImageData::new(1, 1).unwrap()).unwrap_err(),
        Error::VersionConflict
    );
    s.retire(key);
    assert_eq!(held.pixels.bytes(), [255, 0, 0, 255]);
    assert!(matches!(s.get(key), Err(Error::MissingSource)));
}
#[test]
fn draw_image_negative_extents_do_not_flip() {
    let s = source(5, 1, vec![255, 0, 0, 255, 0, 0, 255, 255], 2, 1);
    let mut c = canvas(2, 1);
    c.set_image_smoothing(false);
    c.draw_image(s, Rect::new(2., 1., 0., 0.), Rect::new(2., 1., 0., 0.))
        .unwrap();
    assert_eq!(rgba(&mut c), vec![255, 0, 0, 255, 0, 0, 255, 255]);
}
#[test]
fn draw_image_out_of_source_crop_adjusts_destination() {
    let s = source(2, 1, vec![255, 0, 0, 255], 1, 1);
    let mut c = canvas(2, 1);
    c.draw_image(s, Rect::new(-1., 0., 1., 1.), Rect::new(0., 0., 2., 1.))
        .unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
    assert_eq!(pixel(&mut c, 1, 0), [255, 0, 0, 255]);
}
#[test]
fn draw_image_bilinear_filters_premultiplied_pixels() {
    let s = source(2, 1, vec![255, 0, 0, 0, 0, 0, 255, 255], 2, 1);
    let mut c = canvas(1, 1);
    c.draw_image(s, Rect::new(0., 0., 2., 1.), Rect::new(0., 0., 1., 1.))
        .unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0, 0, 255, 128]);
}
#[test]
fn pattern_handle_transform_remains_live_and_paint_is_snapshotted() {
    let s = source(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255], 2, 1);
    let p = Pattern::new(s, Repetition::Repeat);
    let mut c = canvas(2, 1);
    c.set_image_smoothing(false);
    c.set_fill(Paint::Pattern(p.clone()));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    p.set_transform(Affine::translate((1., 0.))).unwrap();
    c.fill_rect(1., 0., 1., 1.).unwrap();
    assert_eq!(rgba(&mut c), vec![255, 0, 0, 255, 255, 0, 0, 255]);
}
#[test]
fn pattern_repeat_axes_are_independent() {
    for (repeat, right, bottom) in [
        (Repetition::Repeat, true, true),
        (Repetition::RepeatX, true, false),
        (Repetition::RepeatY, false, true),
        (Repetition::NoRepeat, false, false),
    ] {
        let s = source(2, 1, vec![255, 0, 0, 255], 1, 1);
        let mut c = canvas(2, 2);
        c.set_fill(Paint::Pattern(Pattern::new(s, repeat)));
        c.fill_rect(0., 0., 2., 2.).unwrap();
        assert_eq!(pixel(&mut c, 1, 0)[3] > 0, right);
        assert_eq!(pixel(&mut c, 0, 1)[3] > 0, bottom);
    }
}
#[test]
fn get_image_data_ignores_state_and_returns_transparent_out_of_bounds() {
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    c.scale(0., 0.);
    c.clip(FillRule::NonZero);
    c.set_global_alpha(0.);
    let data = c.get_image_data(-1, 0, 3, 1).unwrap();
    assert_eq!(data.bytes(), [0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0]);
    assert_eq!(c.get_image_data(0, 0, 0, 1), Err(Error::IndexSize));
}
#[test]
fn negative_get_image_data_sizes_extend_left_and_up() {
    let mut c = canvas(2, 2);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    assert_eq!(
        c.get_image_data(1, 1, -1, -1).unwrap().bytes(),
        [255, 0, 0, 255]
    );
}
#[test]
fn put_image_data_is_copy_ignoring_transform_alpha_composite_clip() {
    let mut c = canvas(2, 1);
    c.scale(0., 0.);
    c.clip(FillRule::NonZero);
    c.set_global_alpha(0.);
    c.set_composite("destination-in");
    c.put_image_data(
        ImageData::from_rgba(1, 1, vec![51, 102, 153, 128]).unwrap(),
        1,
        0,
        None,
    )
    .unwrap();
    assert_eq!(pixel(&mut c, 1, 0), [51, 102, 153, 128]);
}
#[test]
fn dirty_put_rect_is_normalized_and_clipped_without_overflow() {
    let mut c = canvas(2, 1);
    let im = ImageData::from_rgba(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 255]).unwrap();
    c.put_image_data(im.clone(), 0, 0, Some([2, 1, -1, -1]))
        .unwrap();
    assert_eq!(rgba(&mut c), vec![0, 0, 0, 0, 0, 0, 255, 255]);
    c.put_image_data(
        im,
        i32::MAX,
        i32::MIN,
        Some([i32::MAX, i32::MIN, i32::MIN, i32::MAX]),
    )
    .unwrap();
    assert_eq!(pixel(&mut c, 1, 0), [0, 0, 255, 255]);
}
#[test]
fn pixel_readback_is_a_copy_not_an_alias() {
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    let mut data = c.get_image_data(0, 0, 1, 1).unwrap();
    data.bytes_mut()[0] = 0;
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
#[test]
fn reset_even_same_size_resets_state_path_stack_and_pixels() {
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    c.save();
    c.rect(0., 0., 1., 1.);
    c.reset().unwrap();
    c.restore();
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
    assert!(c.path().elements().is_empty());
    assert_eq!(c.state().transform, Affine::IDENTITY);
}
#[test]
fn clone_resets_pixels_and_state_but_retains_dimensions() {
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    let mut clone = c.clone_blank(55).unwrap();
    assert_eq!(clone.key().id, 55);
    assert_eq!(pixel(&mut clone, 0, 0), [0; 4]);
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
#[test]
fn transfer_bitmap_consumes_pixels_not_drawing_state() {
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    c.rect(0., 0., 1., 1.);
    let bitmap = c.transfer_to_image_bitmap().unwrap();
    assert_eq!(bitmap.pixels.bytes(), [255, 0, 0, 255]);
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
#[derive(Default)]
struct Target {
    fail: bool,
    snapshot: Option<PromotionSnapshot>,
}
impl PromotionTarget for Target {
    fn present(&mut self, s: &PromotionSnapshot) -> Result<()> {
        if self.fail {
            return Err(Error::NeedsPromotion("test failure"));
        }
        self.snapshot = Some(s.clone());
        Ok(())
    }
}
#[test]
fn promotion_preserves_state_stack_path_identity_and_pixels() {
    let mut c = Canvas::new(42, 1, 1).unwrap();
    fill(&mut c, "red", [0., 0., 1., 1.]);
    c.save();
    c.translate(3., 0.);
    c.rect(0., 0., 1., 1.);
    let p = c.path().elements().to_vec();
    let mut t = Target::default();
    let key = c.promote(&mut t).unwrap();
    assert_eq!(key.id, 42);
    assert_eq!(c.path().elements(), p);
    assert_eq!(t.snapshot.unwrap().saved_states.len(), 1);
    c.restore();
    assert_eq!(c.state().transform, Affine::IDENTITY);
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
#[test]
fn failed_promotion_never_marks_success_or_loses_work() {
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    let key = c.key();
    assert!(
        c.promote(&mut Target {
            fail: true,
            ..Default::default()
        })
        .is_err()
    );
    assert_eq!(c.key(), key);
    assert_eq!(c.promoted_version(), None);
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
#[test]
fn stale_promoted_edits_cannot_overwrite_new_authored_version() {
    let mut c = canvas(1, 1);
    let key = c.promote(&mut Target::default()).unwrap();
    fill(&mut c, "red", [0., 0., 1., 1.]);
    assert_eq!(
        c.import_promoted(key, ImageData::new(1, 1).unwrap()),
        Err(Error::VersionConflict)
    );
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
#[test]
fn compacted_commands_keep_float_backing_and_old_sources_alive() {
    let mut c = canvas(1, 1);
    c.set_global_alpha(0.2);
    c.fill_rect(0., 0., 1., 1.).unwrap();
    let before = rgba(&mut c);
    c.compact().unwrap();
    assert!(c.commands().is_empty());
    assert_eq!(rgba(&mut c), before);
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0)[3], 92);
}
#[test]
fn radius_errors_and_nonfinite_paths_are_spec_ordered() {
    let mut c = canvas(2, 2);
    assert_eq!(
        c.arc(Point::ZERO, -1., 0., 1., false),
        Err(Error::IndexSize)
    );
    assert!(c.arc(Point::new(f64::NAN, 0.), -1., 0., 1., false).is_ok());
    assert!(c.path().elements().is_empty());
    assert_eq!(
        c.arc_to(Point::ZERO, Point::new(1., 1.), -1.),
        Err(Error::IndexSize)
    );
}
#[test]
fn arc_directions_and_large_sweeps_have_expected_winding() {
    for ccw in [false, true] {
        let mut c = canvas(4, 4);
        let end = if ccw { -20. } else { 20. };
        c.arc(Point::new(2., 2.), 2., 0., end, ccw).unwrap();
        assert!(c.is_point_in_path(Point::new(2., 2.), FillRule::NonZero));
        assert!(!c.is_point_in_path(Point::new(0., 0.), FillRule::NonZero));
        c.fill(FillRule::NonZero).unwrap();
        assert_eq!(pixel(&mut c, 1, 1)[3], 255);
    }
}
#[test]
fn arc_to_and_quadratic_implicit_subpaths_are_supported() {
    let mut c = canvas(4, 4);
    c.quadratic_curve_to(Point::new(0., 0.), Point::new(4., 0.));
    c.line_to(4., 4.);
    c.arc_to(Point::new(0., 4.), Point::ZERO, 1.).unwrap();
    c.close_path();
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 2, 2)[3], 255);
}
#[test]
fn round_rect_negative_extents_and_radius_validation() {
    let mut c = canvas(4, 4);
    assert_eq!(c.round_rect(0., 0., 4., 4., &[]), Err(Error::Range));
    assert_eq!(
        c.round_rect(0., 0., 4., 4., &[Vec2::new(-1., 1.)]),
        Err(Error::Range)
    );
    c.round_rect(4., 4., -4., -4., &[Vec2::new(100., 100.)])
        .unwrap();
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(pixel(&mut c, 1, 1)[3], 255);
    assert!(pixel(&mut c, 0, 0)[3] < 255);
}
#[test]
fn line_dash_odd_lists_are_doubled_and_invalid_assignments_ignored() {
    let mut c = canvas(8, 2);
    c.set_line_dash(&[2., 1., 3.]);
    assert_eq!(&*c.state().line.dash_pattern, &[2., 1., 3., 2., 1., 3.]);
    c.set_line_dash(&[-1.]);
    assert_eq!(c.state().line.dash_pattern.len(), 6);
    c.set_line_dash(&[0., 0.]);
    assert!(c.state().line.dash_pattern.is_empty());
}
#[test]
fn dashes_caps_and_transformed_stroke_follow_pen_space() {
    let mut c = canvas(12, 4);
    c.scale(2., 2.);
    c.set_line_width(1.);
    c.set_line_dash(&[1., 1.]);
    c.move_to(1., 1.);
    c.line_to(5., 1.);
    c.stroke().unwrap();
    assert_eq!(pixel(&mut c, 2, 1)[3], 255);
    assert_eq!(pixel(&mut c, 4, 1)[3], 0);
    c.set_line_dash(&[]);
    c.set_line_cap(Cap::Square);
    assert!(c.is_point_in_stroke(Point::new(1.5, 2.)));
    c.set_line_cap(Cap::Butt);
    assert!(!c.is_point_in_stroke(Point::new(1.5, 2.)));
}
#[test]
fn empty_and_oversized_buffers_fail_without_destroying_canvas() {
    assert_eq!(ImageData::new(0, 1), Err(Error::IndexSize));
    assert!(matches!(
        Canvas::new(1, u32::MAX, u32::MAX),
        Err(Error::Capacity)
    ));
    let mut c = canvas(1, 1);
    fill(&mut c, "red", [0., 0., 1., 1.]);
    assert_eq!(c.resize(u32::MAX, 2), Err(Error::Capacity));
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}

struct TestFonts {
    info: pm_text::font::FaceInfo,
}
impl TestFonts {
    fn new() -> Self {
        use pm_text::font::{FaceFacts, FaceInfo, FontBytes};
        let bytes = include_bytes!("fixtures/recorder-test.ttf").to_vec();
        let facts = FaceFacts::of(&bytes, 0);
        Self {
            info: FaceInfo {
                data: FontBytes::new(bytes),
                index: 0,
                blob: "synthetic".into(),
                coords: Vec::new(),
                synthetic_bold: false,
                synthetic_italic: false,
                custom: false,
                web: None,
                synthesized_hyphens: [None; 2],
                postscript_name: "RecorderTest".into(),
                typeface_key: "synthetic".into(),
                fallback_key: None,
                facts,
            },
        }
    }
}
impl pm_text::font::FontResolver for TestFonts {
    fn face(&self, _: u32) -> &pm_text::font::FaceInfo {
        &self.info
    }
    fn family(&mut self, _: &str, _: &pm_text::font::FontDesc) -> Option<u32> {
        Some(0)
    }
    fn standard(&mut self, _: &pm_text::font::FontDesc) -> Option<u32> {
        Some(0)
    }
    fn fallback_for_char(
        &mut self,
        _: &pm_text::font::FontDesc,
        _: u32,
        _: u32,
        _: bool,
    ) -> Option<u32> {
        Some(0)
    }
    fn last_resort(&mut self, _: &pm_text::font::FontDesc) -> Option<u32> {
        Some(0)
    }
    fn variable_face(&mut self, _: &str) -> Option<u32> {
        None
    }
}
#[test]
fn css_font_shorthand_quoted_families_and_units() {
    let f = Font::parse("italic 650 12pt/1.5 'Recorder Test', sans-serif", 16., 16.).unwrap();
    assert_eq!(f.desc.size, 16.);
    assert_eq!(f.desc.weight, 650.);
    assert!(f.desc.italic);
    assert_eq!(f.families.len(), 2);
    near(
        Font::parse("1.25em serif", 20., 16.).unwrap().desc.size as f64,
        25.,
    );
    near(
        Font::parse("2rem serif", 20., 16.).unwrap().desc.size as f64,
        32.,
    );
}
#[test]
fn invalid_font_assignment_keeps_previous_value() {
    let mut c = canvas(1, 1);
    c.set_font("20px serif").unwrap();
    c.set_font("not a font").unwrap();
    assert_eq!(c.state().font.desc.size, 20.);
}
#[test]
fn pm_text_measures_at_authored_size_without_chromium_floor() {
    let c = canvas(1, 1);
    let mut engine = TextEngine::new(TestFonts::new());
    let f = Font::parse("10.125px 'Recorder Test'", 16., 16.).unwrap();
    let l = engine.layout(&f, &[65], Direction::Ltr).unwrap();
    near(l.width, 6.0749969482421875);
    assert_eq!(c.state().font.desc.size, 10.);
}
#[test]
fn pm_text_width_and_metrics_come_from_fixture_font() {
    let mut c = canvas(30, 20);
    c.set_font("10px 'Recorder Test'").unwrap();
    let mut engine = TextEngine::new(TestFonts::new());
    let m = c.measure_text(&[65, 32, 66], &mut engine).unwrap();
    near(m.width, 15.);
    near(m.font_bounding_box_ascent, 8.);
    near(m.font_bounding_box_descent, 2.);
    near(m.actual_bounding_box_ascent, 7.);
    near(m.actual_bounding_box_right, 14.);
}
#[test]
fn fill_text_uses_canvas_baseline_not_editor_text_layout() {
    let mut c = canvas(8, 10);
    c.set_font("10px 'Recorder Test'").unwrap();
    let mut engine = TextEngine::new(TestFonts::new());
    c.fill_text(&[65], 0., 7., None, &mut engine).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0, 0, 0, 255]);
    assert_eq!(pixel(&mut c, 4, 6)[3], 255);
    assert_eq!(pixel(&mut c, 5, 6), [0; 4]);
    assert_eq!(pixel(&mut c, 0, 7), [0; 4]);
}
#[test]
fn text_align_and_direction_change_anchor_but_not_measurement_width() {
    let mut c = canvas(20, 10);
    c.set_font("10px serif").unwrap();
    let mut engine = TextEngine::new(TestFonts::new());
    c.set_direction(Direction::Rtl);
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.width, 6.);
    near(m.actual_bounding_box_left, 6.);
    near(m.actual_bounding_box_right, -1.);
    c.fill_text(&[65], 10., 7., None, &mut engine).unwrap();
    assert_eq!(pixel(&mut c, 4, 0)[3], 255);
    assert_eq!(pixel(&mut c, 10, 0)[3], 0);
}
#[test]
fn text_center_top_baseline_and_max_width_condense_glyphs() {
    let mut c = canvas(12, 10);
    c.set_font("10px serif").unwrap();
    c.set_text_align(TextAlign::Center);
    c.set_text_baseline(TextBaseline::Top);
    let mut engine = TextEngine::new(TestFonts::new());
    c.fill_text(&[65, 65], 6., 0., Some(6.), &mut engine)
        .unwrap();
    assert_eq!(pixel(&mut c, 3, 1)[3], 255);
    assert_eq!(pixel(&mut c, 2, 1)[3], 0);
    assert_eq!(pixel(&mut c, 8, 1)[3], 128);
}
#[test]
fn text_normalizes_ascii_whitespace_and_does_not_wrap() {
    let mut engine = TextEngine::new(TestFonts::new());
    let font = Font::default();
    let a = engine.layout(&font, &[65, 10, 65], Direction::Ltr).unwrap();
    let b = engine.layout(&font, &[65, 32, 65], Direction::Ltr).unwrap();
    near(a.width, 15.);
    assert_eq!(a.path, b.path);
}
#[test]
fn empty_text_has_zero_ink_and_advance() {
    let mut c = canvas(1, 1);
    let mut engine = TextEngine::new(TestFonts::new());
    let m = c.measure_text(&[], &mut engine).unwrap();
    near(m.width, 0.);
    c.fill_text(&[], 0., 0., None, &mut engine).unwrap();
    assert!(c.commands().is_empty());
}
#[test]
fn stroke_text_uses_same_shaped_outlines_and_stroke_state() {
    let mut c = canvas(8, 10);
    c.set_font("10px serif").unwrap();
    c.set_stroke_color("blue").unwrap();
    c.set_line_width(2.);
    let mut engine = TextEngine::new(TestFonts::new());
    c.stroke_text(&[65], 1., 8., None, &mut engine).unwrap();
    assert_eq!(pixel(&mut c, 0, 3), [0, 0, 255, 255]);
    assert_eq!(pixel(&mut c, 3, 3), [0; 4]);
}
// Independent Sutherland–Hodgman reference, used only by tests.
fn reference_area(mut polygon: Vec<Point>, x: f64, y: f64) -> f64 {
    for (axis, edge, keep_greater) in [
        (0, x, true),
        (0, x + 1., false),
        (1, y, true),
        (1, y + 1., false),
    ] {
        let old = std::mem::take(&mut polygon);
        if old.is_empty() {
            return 0.;
        }
        let coord = |p: Point| if axis == 0 { p.x } else { p.y };
        let inside = |p: Point| {
            if keep_greater {
                coord(p) >= edge
            } else {
                coord(p) <= edge
            }
        };
        for i in 0..old.len() {
            let a = old[i];
            let b = old[(i + 1) % old.len()];
            if inside(a) {
                polygon.push(a);
            }
            if inside(a) != inside(b) {
                let t = (edge - coord(a)) / (coord(b) - coord(a));
                polygon.push(a + (b - a) * t);
            }
        }
    }
    if polygon.len() < 3 {
        return 0.;
    }
    polygon
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let b = polygon[(i + 1) % polygon.len()];
            a.x * b.y - a.y * b.x
        })
        .sum::<f64>()
        .abs()
        / 2.
}
#[test]
fn polygon_pixels_match_independent_area_reference() {
    for n in 0..64 {
        let a = Point::new(-2. + (n % 7) as f64 * 0.13, -1.25);
        let b = Point::new(7.5, 0.2 + n as f64 * 0.017);
        let d = Point::new(2. + (n % 5) as f64 * 0.23, 6.75);
        let mut c = canvas(8, 8);
        c.move_to(a.x, a.y);
        c.line_to(b.x, b.y);
        c.line_to(d.x, d.y);
        c.fill(FillRule::NonZero).unwrap();
        let pixels = rgba(&mut c);
        for y in 0..8 {
            for x in 0..8 {
                let expected = (reference_area(vec![a, b, d], x as f64, y as f64) * 255.)
                    .round_ties_even() as i32;
                let actual = pixels[(y * 8 + x) * 4 + 3] as i32;
                assert!(
                    (expected - actual).abs() <= 1,
                    "triangle {n}, ({x},{y}): {actual} != {expected}"
                );
            }
        }
    }
}
#[test]
fn curve_commands_record_without_consuming_default_path() {
    let mut c = canvas(10, 10);
    c.move_to(0., 5.);
    c.bezier_curve_to(Point::new(0., 0.), Point::new(10., 0.), Point::new(10., 5.));
    c.line_to(0., 5.);
    let before = c.path().elements().to_vec();
    c.fill(FillRule::NonZero).unwrap();
    assert_eq!(c.path().elements(), before);
    assert!(c.is_point_in_path(Point::new(5., 3.), FillRule::NonZero));
}
#[test]
fn joins_miter_limits_and_caps_produce_finite_geometry() {
    for join in [Join::Miter, Join::Bevel, Join::Round] {
        for cap in [Cap::Butt, Cap::Square, Cap::Round] {
            let mut c = canvas(12, 12);
            c.set_line_join(join);
            c.set_line_cap(cap);
            c.set_miter_limit(2.);
            c.set_line_width(3.);
            c.move_to(2., 10.);
            c.line_to(6., 2.);
            c.line_to(10., 10.);
            c.stroke().unwrap();
            assert!(rgba(&mut c).chunks(4).any(|p| p[3] > 0));
        }
    }
}
#[test]
fn zero_sized_canvas_is_valid_but_not_a_drawable_source() {
    let mut c = canvas(0, 0);
    c.fill_rect(0., 0., 2., 2.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
    assert!(matches!(c.snapshot(), Err(Error::IndexSize)));
}
#[test]
fn inspection_exposes_command_identity_geometry_and_style() {
    let mut c = canvas(1, 1);
    c.set_global_alpha(0.5);
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(c.commands()[0].kind(), "draw");
    if let Command::Draw(d) = &c.commands()[0] {
        assert!(!d.path().elements().is_empty());
        assert_eq!(d.global_alpha(), 0.5);
        assert_eq!(d.clip_count(), 0);
    } else {
        panic!("draw missing");
    }
}
#[test]
fn pattern_nonrepeat_boundary_uses_analytic_coverage() {
    let p = Pattern::new(
        source(8, 1, vec![255, 0, 0, 255], 1, 1),
        Repetition::NoRepeat,
    );
    p.set_transform(Affine::translate((0.75, 0.))).unwrap();
    let mut c = canvas(2, 1);
    c.set_fill(Paint::Pattern(p));
    c.fill_rect(0., 0., 2., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 64]);
    assert_eq!(pixel(&mut c, 1, 0), [255, 0, 0, 191]);
}
#[test]
fn image_fractional_edge_coverage_clamps_source_sample() {
    let s = source(8, 1, vec![255, 0, 0, 255], 1, 1);
    let mut c = canvas(2, 1);
    c.draw_image(s, Rect::new(0., 0., 1., 1.), Rect::new(0.75, 0., 1.75, 1.))
        .unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 64]);
}
#[test]
fn path_boundary_is_inside_even_on_right_and_bottom_edges() {
    let mut c = canvas(2, 2);
    c.rect(0., 0., 1., 1.);
    for p in [
        Point::new(0., 0.),
        Point::new(1., 1.),
        Point::new(1., 0.5),
        Point::new(0.5, 1.),
    ] {
        assert!(c.is_point_in_path(p, FillRule::NonZero));
    }
}
#[test]
fn stroke_rect_degenerate_dimension_is_open_capped_line() {
    let mut c = canvas(4, 4);
    c.set_line_width(2.);
    c.set_line_cap(Cap::Square);
    c.stroke_rect(2., 1., 0., 2.).unwrap();
    assert_eq!(pixel(&mut c, 1, 0)[3], 255);
    c.reset().unwrap();
    c.stroke_rect(2., 2., 0., 0.).unwrap();
    assert!(c.commands().is_empty());
}
#[test]
fn promoted_edit_import_retains_state_and_rejects_second_stale_commit() {
    let mut c = canvas(1, 1);
    c.set_fill_color("blue").unwrap();
    let key = c.promote(&mut Target::default()).unwrap();
    c.import_promoted(
        key,
        ImageData::from_rgba(1, 1, vec![255, 0, 0, 255]).unwrap(),
    )
    .unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
    assert_eq!(
        c.import_promoted(key, ImageData::new(1, 1).unwrap()),
        Err(Error::VersionConflict)
    );
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0, 0, 255, 255]);
}
#[test]
fn hsl_blends_have_spec_primary_color_results() {
    for (op, want) in [
        ("hue", [54, 54, 255]),
        ("saturation", [255, 0, 0]),
        ("color", [54, 54, 255]),
        ("luminosity", [94, 0, 0]),
    ] {
        let mut c = canvas(1, 1);
        fill(&mut c, "red", [0., 0., 1., 1.]);
        c.set_composite(op);
        fill(&mut c, "blue", [0., 0., 1., 1.]);
        let p = pixel(&mut c, 0, 0);
        for i in 0..3 {
            assert!((p[i] as i32 - want[i]).abs() <= 1, "{op}: {p:?}");
        }
    }
}
#[test]
fn recorded_commands_replay_on_an_independent_worker_with_identical_pixels() {
    let mut c = canvas(8, 8);
    c.set_fill_color("#8452b080").unwrap();
    c.fill_rect(0.25, 0.75, 5.5, 3.25).unwrap();
    c.translate(2., 1.);
    c.arc(Point::new(2., 2.), 1.75, 0., 7., false).unwrap();
    c.fill(FillRule::EvenOdd).unwrap();
    let commands = c.commands().to_vec();
    let worker = std::thread::spawn(move || {
        let mut r = CpuRasterizer::new(8, 8).unwrap();
        r.replay(&commands).unwrap();
        r.image_data().unwrap()
    });
    assert_eq!(worker.join().unwrap().bytes(), rgba(&mut c));
}
#[test]
fn source_retirement_forbids_key_reuse_even_with_held_consumer() {
    let mut sources = Sources::default();
    let key = SourceKey { id: 7, version: 8 };
    let source = sources.insert(key, ImageData::new(1, 1).unwrap()).unwrap();
    sources.retire(key);
    assert_eq!(
        sources
            .insert(key, ImageData::from_rgba(1, 1, vec![255; 4]).unwrap())
            .unwrap_err(),
        Error::VersionConflict
    );
    assert_eq!(source.pixels.bytes(), [0; 4]);
}
#[test]
fn exact_image_copy_fast_path_matches_generic_composite_with_hidden_rgb() {
    let s = source(
        7,
        1,
        vec![255, 0, 0, 0, 51, 102, 153, 128, 42, 21, 63, 255],
        3,
        1,
    );
    let mut fast = canvas(3, 1);
    fast.set_composite("copy");
    fast.draw_image(
        s.clone(),
        Rect::new(0., 0., 3., 1.),
        Rect::new(0., 0., 3., 1.),
    )
    .unwrap();
    let mut general = canvas(3, 1);
    general
        .draw_image(s, Rect::new(0., 0., 3., 1.), Rect::new(0., 0., 3., 1.))
        .unwrap();
    assert_eq!(rgba(&mut fast), rgba(&mut general));
    assert_eq!(pixel(&mut fast, 0, 0), [0; 4]);
}
#[test]
fn sparse_float_tiles_retain_source_pixels_across_tile_edges_and_clip() {
    let bytes = (0..33 * 17)
        .flat_map(|i| [(i % 255) as u8, 20, 30, 255])
        .collect();
    let s = source(7, 1, bytes, 33, 17);
    let mut c = canvas(33, 17);
    c.set_composite("copy");
    c.draw_image(
        s.clone(),
        Rect::new(0., 0., 33., 17.),
        Rect::new(0., 0., 33., 17.),
    )
    .unwrap();
    c.set_composite("source-over");
    fill(&mut c, "blue", [15., 15., 3., 2.]);
    let image = c.snapshot().unwrap();
    for y in 0..17 {
        for x in 0..33 {
            let i = (y * 33 + x) * 4;
            if (15..18).contains(&x) && y >= 15 {
                assert_eq!(&image.pixels.bytes()[i..i + 4], &[0, 0, 255, 255]);
            } else {
                assert_eq!(&image.pixels.bytes()[i..i + 4], &s.pixels.bytes()[i..i + 4]);
            }
        }
    }
}
#[test]
fn readback_mutation_invalidates_canonical_metadata_without_affecting_equality() {
    let mut a = ImageData::new(1, 1).unwrap();
    let b = a.clone();
    a.bytes_mut()[0] = 0;
    assert_eq!(a, b);
    a.bytes_mut()[0] = 255;
    let s = source(4, 1, a.bytes().to_vec(), 1, 1);
    let mut c = canvas(1, 1);
    c.set_composite("copy");
    c.draw_image(s, Rect::new(0., 0., 1., 1.), Rect::new(0., 0., 1., 1.))
        .unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
}
fn text_engine_with_font(data: &[u8]) -> TextEngine<TestFonts> {
    let mut resolver = TestFonts::new();
    resolver.info.data = pm_text::font::FontBytes::new(data.to_vec());
    resolver.info.facts = pm_text::font::FaceFacts::of(data, 0);
    TextEngine::new(resolver)
}
#[test]
fn text_em_box_is_one_em_even_when_hhea_box_is_larger() {
    let mut engine = text_engine_with_font(include_bytes!("fixtures/recorder-metrics.ttf"));
    let mut c = canvas(1, 1);
    c.set_font("10px serif").unwrap();
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.font_bounding_box_ascent, 12.);
    near(m.font_bounding_box_descent, 3.);
    near(m.em_height_ascent, 8.);
    near(m.em_height_descent, 2.);
    c.set_text_baseline(TextBaseline::Middle);
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.em_height_ascent, 5.);
    near(m.em_height_descent, 5.);
}
#[test]
fn text_uses_opentype_hanging_and_ideographic_baselines() {
    let mut engine = text_engine_with_font(include_bytes!("fixtures/recorder-baselines.ttf"));
    let mut c = canvas(1, 1);
    c.set_font("10px serif").unwrap();
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.hanging_baseline, 6.);
    near(m.ideographic_baseline, -2.5);
    near(m.em_height_ascent, 7.5);
    near(m.em_height_descent, 2.5);
    c.set_text_baseline(TextBaseline::Hanging);
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.hanging_baseline, 0.);
    near(m.alphabetic_baseline, -6.);
    c.set_text_baseline(TextBaseline::Ideographic);
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.ideographic_baseline, 0.);
    near(m.alphabetic_baseline, 2.5);
}
#[test]
fn text_top_and_bottom_use_em_box_not_hhea_extents() {
    let mut engine = text_engine_with_font(include_bytes!("fixtures/recorder-baselines.ttf"));
    let mut c = canvas(1, 1);
    c.set_font("10px serif").unwrap();
    c.set_text_baseline(TextBaseline::Top);
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.em_height_ascent, 0.);
    near(m.font_bounding_box_ascent, 4.5);
    near(m.em_height_descent, 10.);
    c.set_text_baseline(TextBaseline::Bottom);
    let m = c.measure_text(&[65], &mut engine).unwrap();
    near(m.em_height_ascent, 10.);
    near(m.em_height_descent, 0.);
}
#[test]
fn font_parser_rejects_invalid_families_and_negative_line_height() {
    for s in [
        "20px 'Test' Extra",
        "20px 123",
        "20px inherit",
        "20px/ -1 serif",
        "20px/ -1px serif",
        "20px serif,",
    ] {
        assert_eq!(Font::parse(s, 16., 16.).unwrap_err(), Error::Syntax, "{s}");
    }
}
#[test]
fn font_keywords_and_units_are_ascii_case_insensitive() {
    let f = Font::parse("ITALIC BOLD 12PT SERIF", 16., 16.).unwrap();
    assert!(f.desc.italic);
    assert_eq!(f.desc.weight, 700.);
    assert_eq!(f.desc.size, 16.);
    assert!(matches!(
        f.families[0],
        pm_text::blink::measure::Family::Generic("serif")
    ));
}
#[test]
fn zero_font_size_is_valid_and_has_zero_advance() {
    let mut c = canvas(1, 1);
    c.set_font("0px serif").unwrap();
    let mut engine = TextEngine::new(TestFonts::new());
    assert_eq!(c.state().font.desc.size, 0.);
    near(c.measure_text(&[65], &mut engine).unwrap().width, 0.);
}
#[test]
fn relative_font_sizes_resolve_against_explicit_style_source() {
    let mut c = canvas(1, 1);
    c.set_font_with_context("150% serif", 20., 12.).unwrap();
    near(c.state().font.desc.size as f64, 30.);
    c.set_font_with_context("2rem serif", 20., 12.).unwrap();
    near(c.state().font.desc.size as f64, 24.);
}
#[test]
fn font_relative_weights_use_style_source_instead_of_previous_canvas_font() {
    let mut c = canvas(1, 1);
    c.set_font("100 10px serif").unwrap();
    c.set_font_with_source(
        "bolder 10px serif",
        FontContext {
            weight: 700.,
            ..FontContext::default()
        },
    )
    .unwrap();
    near(c.state().font.desc.weight as f64, 900.);
    c.set_font_with_source(
        "lighter 10px serif",
        FontContext {
            weight: 700.,
            ..FontContext::default()
        },
    )
    .unwrap();
    near(c.state().font.desc.weight as f64, 400.);
}
#[test]
fn zero_length_segments_are_pruned_before_stroking_as_the_spec_requires() {
    let mut c = canvas(4, 4);
    c.set_line_width(2.);
    c.set_line_cap(Cap::Round);
    c.move_to(2., 2.);
    c.line_to(2., 2.);
    c.stroke().unwrap();
    assert_eq!(pixel(&mut c, 1, 1)[3], 0);
    assert_eq!(pixel(&mut c, 0, 0)[3], 0);
}
#[test]
fn stroke_hit_test_counts_boundary_points_as_inside() {
    let mut c = canvas(4, 4);
    c.set_line_width(2.);
    c.move_to(0., 2.);
    c.line_to(4., 2.);
    assert!(c.is_point_in_stroke(Point::new(2., 3.)));
    assert!(!c.is_point_in_stroke(Point::new(2., 3.01)));
}
#[test]
fn eight_k_canvas_admission_uses_sparse_backing_without_downscaling() {
    let mut c = canvas(8192, 8192);
    assert_eq!(c.size(), (8192, 8192));
    fill(&mut c, "red", [8191., 8191., 1., 1.]);
    assert_eq!(pixel(&mut c, 8191, 8191), [255, 0, 0, 255]);
    assert_eq!(pixel(&mut c, 0, 0), [0; 4]);
}
#[test]
fn valid_unimplemented_font_forms_report_a_gate_without_mutating_font() {
    let mut c = canvas(1, 1);
    c.set_font("20px serif").unwrap();
    for s in ["caption", "SMALL-CAPS 12px serif", "condensed 12px serif"] {
        assert_eq!(c.set_font(s), Err(Error::NotSupported));
        assert_eq!(c.state().font.desc.size, 20.);
    }
}
#[test]
fn restricted_gradient_numbers_throw_type_error_before_algorithm_validation() {
    assert!(matches!(
        Gradient::new(GradientKind::Radial {
            start: Point::new(f64::NAN, 0.),
            r0: -1.,
            end: Point::ZERO,
            r1: 1.
        }),
        Err(Error::Type)
    ));
    let g = Gradient::new(GradientKind::Linear(Point::ZERO, Point::new(1., 0.))).unwrap();
    assert_eq!(g.add_color_stop(f64::NAN, "red"), Err(Error::Type));
    assert_eq!(g.add_color_stop(f64::INFINITY, "red"), Err(Error::Type));
}
#[test]
fn css_color_four_in_gamut_paint_and_gradient_stops_use_native_conversion() {
    let mut c = canvas(1, 1);
    fill(&mut c, "color(srgb 0.2 0.4 0.6 / 50%)", [0., 0., 1., 1.]);
    assert_eq!(pixel(&mut c, 0, 0), [51, 102, 153, 128]);
    let g = Gradient::new(GradientKind::Linear(Point::ZERO, Point::new(1., 0.))).unwrap();
    g.add_color_stop(0., "lab(0% 0 0)").unwrap();
    g.add_color_stop(1., "color(srgb 1 1 1)").unwrap();
    c.reset().unwrap();
    c.set_fill(Paint::Gradient(g));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [128, 128, 128, 255]);
}
#[test]
fn unsupported_color_conversion_is_explicit_and_preserves_the_previous_paint() {
    let mut c = canvas(1, 1);
    c.set_fill_color("red").unwrap();
    assert!(matches!(
        c.set_fill_color("color(display-p3 1 0 0)"),
        Err(Error::NeedsPromotion(_))
    ));
    c.fill_rect(0., 0., 1., 1.).unwrap();
    assert_eq!(pixel(&mut c, 0, 0), [255, 0, 0, 255]);
}
#[test]
fn css_system_colors_are_resolved_by_the_host_not_chromium_color_tables() {
    let mut c = canvas(1, 1);
    for color in ["Highlight", "light-dark(white, black)"] {
        assert!(matches!(
            c.set_fill_color(color),
            Err(Error::NeedsPromotion(_))
        ));
    }
}
