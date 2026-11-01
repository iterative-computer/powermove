use kurbo::Shape;
use pm_recorder_raster::{recorder::*, *};
use std::sync::Arc;
fn surface(w: u32, h: u32) -> Rasterizer {
    Rasterizer::new(7, w, h).unwrap()
}
fn red(c: &mut Canvas, x: f64, y: f64, w: f64, h: f64) {
    c.set_fill_color("red").unwrap();
    c.fill_rect(x, y, w, h).unwrap();
}
fn pixel(r: &Rasterizer, x: i32, y: i32) -> [u8; 4] {
    r.get_image_data(x, y, 1, 1)
        .unwrap()
        .bytes()
        .try_into()
        .unwrap()
}
fn mask(origin: [i32; 2], a: f32) -> AlphaMask {
    AlphaMask::new(1, 1, origin, vec![a]).unwrap()
}
fn shadow(offset: [f64; 2], blur: f64) -> Shadow {
    Shadow {
        color: Color::BLACK,
        offset,
        blur,
    }
}
#[test]
fn replay_owned_batch_on_worker_and_retain_source_version() {
    let mut sources = Sources::default();
    let source = sources
        .insert(
            SourceKey { id: 99, version: 1 },
            ImageData::from_rgba(1, 1, vec![255, 0, 0, 255]).unwrap(),
        )
        .unwrap();
    let mut c = Canvas::new(7, 2, 1).unwrap();
    c.draw_image(source, Rect::new(0., 0., 1., 1.), Rect::new(1., 0., 2., 1.))
        .unwrap();
    let batch = c.commands().to_vec();
    sources.retire(SourceKey { id: 99, version: 1 });
    drop(c);
    let r = std::thread::spawn(move || {
        let mut r = surface(2, 1);
        r.replay(r.key(), &batch).unwrap();
        r
    })
    .join()
    .unwrap();
    assert_eq!(pixel(&r, 1, 0), [255, 0, 0, 255]);
}
#[test]
fn suffix_versions_and_rejected_batches_preserve_pixels() {
    let mut c = Canvas::new(7, 2, 1).unwrap();
    red(&mut c, 0., 0., 1., 1.);
    let mut r = surface(2, 1);
    let initial = r.key();
    let next = r.replay(initial, c.commands()).unwrap();
    assert_eq!(next, c.key());
    assert_eq!(r.replay(initial, c.commands()), Err(Error::VersionConflict));
    assert_eq!(
        r.replay(SourceKey { id: 8, ..next }, &[]),
        Err(Error::VersionConflict)
    );
    assert_eq!(r.replay(next, &[]).unwrap(), next);
    red(&mut c, 1., 0., 1., 1.);
    r.replay(next, &c.commands()[1..]).unwrap();
    assert_eq!(
        r.image_data().unwrap().bytes(),
        &[255, 0, 0, 255, 255, 0, 0, 255]
    );
}
#[test]
fn replay_keeps_float_alpha_between_batches() {
    let mut c = Canvas::new(7, 1, 1).unwrap();
    c.set_global_alpha(0.001);
    red(&mut c, 0., 0., 1., 1.);
    let mut r = surface(1, 1);
    for _ in 0..1000 {
        r.replay(r.key(), c.commands()).unwrap();
        let _ = r.image_data().unwrap();
    }
    let expected = ((1. - 0.999f64.powi(1000)) * 255.).round() as u8;
    assert_eq!(pixel(&r, 0, 0), [255, 0, 0, expected]);
}
#[test]
fn signed_readback_and_extreme_bounds_are_transparent() {
    let mut c = Canvas::new(7, 2, 1).unwrap();
    red(&mut c, 0., 0., 1., 1.);
    let mut r = surface(2, 1);
    r.replay(r.key(), c.commands()).unwrap();
    assert_eq!(
        r.get_image_data(2, 1, -3, -1).unwrap().bytes(),
        &[0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0]
    );
    for x in [i32::MIN, i32::MAX] {
        assert_eq!(pixel(&r, x, x), [0; 4]);
    }
    assert_eq!(r.get_image_data(0, 0, 0, 1), Err(Error::IndexSize));
    assert_eq!(r.get_image_data(0, 0, i32::MIN, 1), Err(Error::Capacity));
}
#[test]
fn dirty_put_ignores_all_state_and_canonicalizes_hidden_rgb() {
    let mut c = Canvas::new(7, 3, 1).unwrap();
    c.scale(100., 100.);
    c.set_global_alpha(0.);
    c.set_composite("destination-in");
    c.rect(100., 100., 1., 1.);
    c.clip(FillRule::NonZero);
    c.put_image_data(
        ImageData::from_rgba(3, 1, vec![255, 0, 0, 255, 0, 255, 0, 128, 99, 88, 77, 0]).unwrap(),
        0,
        0,
        Some([3, 0, -2, 1]),
    )
    .unwrap();
    let mut r = surface(3, 1);
    r.replay(r.key(), c.commands()).unwrap();
    assert_eq!(
        r.image_data().unwrap().bytes(),
        &[0, 0, 0, 0, 0, 255, 0, 128, 0, 0, 0, 0]
    );
}
#[test]
fn resize_is_clear_and_failure_is_lossless() {
    let mut c = Canvas::new(7, 2, 1).unwrap();
    red(&mut c, 0., 0., 2., 1.);
    let mut r = surface(2, 1);
    r.replay(r.key(), c.commands()).unwrap();
    let key = r.key();
    assert_eq!(r.resize(key, u32::MAX, u32::MAX), Err(Error::Capacity));
    assert_eq!(r.key(), key);
    assert_eq!(pixel(&r, 0, 0), [255, 0, 0, 255]);
    let next = r.resize(key, 2, 1).unwrap();
    assert_eq!(next.version, key.version + 1);
    assert_eq!(pixel(&r, 0, 0), [0; 4]);
}
#[test]
fn zero_dimensions_and_sparse_8k_are_admitted() {
    let r = surface(0, 0);
    assert_eq!(r.image_data(), Err(Error::IndexSize));
    assert_eq!(pixel(&r, 1, 1), [0; 4]);
    let r = surface(8192, 8192);
    assert_eq!(r.size(), (8192, 8192));
    assert!(matches!(
        Rasterizer::new(1, 8193, 8192),
        Err(Error::Capacity)
    ));
}
#[test]
fn shadow_integer_and_fractional_offsets_preserve_float_alpha() {
    for (offset, expected) in [([1., 0.], vec![0, 255, 0]), ([0.5, 0.], vec![128, 128, 0])] {
        let mut r = surface(3, 1);
        r.draw_shadow(
            r.key(),
            &mask([0, 0], 1.),
            shadow(offset, 0.),
            "source-over",
            &[],
        )
        .unwrap();
        let a: Vec<u8> = r
            .image_data()
            .unwrap()
            .bytes()
            .chunks_exact(4)
            .map(|p| p[3])
            .collect();
        assert_eq!(a, expected);
    }
    let mut r = surface(1, 1);
    for _ in 0..1000 {
        r.draw_shadow(
            r.key(),
            &mask([0, 0], 0.001),
            shadow([0., 0.], 0.),
            "source-over",
            &[],
        )
        .unwrap();
    }
    assert_eq!(
        pixel(&r, 0, 0)[3],
        ((1. - 0.999f64.powi(1000)) * 255.).round() as u8
    );
}
#[test]
fn offcanvas_caster_can_cast_into_canvas() {
    let mut r = surface(2, 1);
    r.draw_shadow(
        r.key(),
        &mask([-2, 0], 1.),
        shadow([3., 0.], 0.),
        "source-over",
        &[],
    )
    .unwrap();
    assert_eq!(pixel(&r, 1, 0), [0, 0, 0, 255]);
}
#[test]
fn gaussian_is_symmetric_normalized_and_not_clipped_to_source() {
    let m = mask([0, 0], 1.).blurred(2.).unwrap();
    assert_eq!(m.size(), (11, 11));
    assert_eq!(m.origin(), [-5, -5]);
    let sum: f64 = m.values().iter().map(|v| f64::from(*v)).sum();
    assert!((sum - 1.).abs() < 1e-7);
    let center = m.values()[5 * 11 + 5];
    assert!((f64::from(center) - 1. / (2. * std::f64::consts::PI)).abs() < 1e-6);
    for y in 0..11 {
        for x in 0..11 {
            assert_eq!(m.values()[y * 11 + x], m.values()[(10 - y) * 11 + 10 - x]);
        }
    }
    assert_eq!(
        mask([0, 0], 1.)
            .blurred(f64::MIN_POSITIVE)
            .unwrap()
            .values(),
        &[0., 0., 0., 0., 1., 0., 0., 0., 0.]
    );
}
#[test]
fn shadow_clip_is_applied_after_blur_and_offset() {
    let clip = Clip {
        path: Rect::new(1.25, 0., 3., 1.).to_path(1e-4),
        rule: FillRule::NonZero,
    };
    let mut r = surface(3, 1);
    r.draw_shadow(
        r.key(),
        &mask([-1, 0], 1.),
        shadow([2., 0.], 0.),
        "source-over",
        &[clip.clone(), clip],
    )
    .unwrap();
    assert_eq!(pixel(&r, 0, 0), [0; 4]);
    assert_eq!(pixel(&r, 1, 0), [0, 0, 0, 191]);
}
#[test]
fn shadow_composites_transparent_source_once_inside_clip() {
    for op in [
        "copy",
        "source-in",
        "source-out",
        "destination-in",
        "destination-atop",
    ] {
        let mut c = Canvas::new(7, 3, 1).unwrap();
        red(&mut c, 0., 0., 3., 1.);
        let mut r = surface(3, 1);
        r.replay(r.key(), c.commands()).unwrap();
        let clip = Clip {
            path: Rect::new(0., 0., 2., 1.).to_path(1e-4),
            rule: FillRule::NonZero,
        };
        r.draw_shadow(
            r.key(),
            &mask([0, 0], 1.),
            shadow([0., 0.], 0.),
            op,
            &[clip],
        )
        .unwrap();
        assert_eq!(pixel(&r, 1, 0), [0; 4], "{op}");
        assert_eq!(pixel(&r, 2, 0), [255, 0, 0, 255], "{op}");
    }
}
#[test]
fn shadow_global_alpha_in_source_and_color_alpha_are_applied_once() {
    let mut r = surface(1, 1);
    let s = Shadow {
        color: Color {
            r: 1.,
            g: 0.,
            b: 0.,
            a: 0.5,
        },
        ..shadow([0., 0.], 0.)
    };
    r.draw_shadow(r.key(), &mask([0, 0], 0.5), s, "source-over", &[])
        .unwrap();
    assert_eq!(pixel(&r, 0, 0), [255, 0, 0, 64]);
}
#[test]
fn shadow_validation_failure_preserves_version_and_pixels() {
    let mut r = surface(1, 1);
    let key = r.key();
    for s in [
        shadow([f64::NAN, 0.], 0.),
        shadow([0., 0.], -1.),
        shadow([0., 0.], f64::INFINITY),
        shadow([0., 0.], 1e20),
    ] {
        assert!(
            r.draw_shadow(key, &mask([0, 0], 1.), s, "source-over", &[])
                .is_err()
        );
        assert_eq!(r.key(), key);
        assert_eq!(pixel(&r, 0, 0), [0; 4]);
    }
    assert_eq!(
        r.draw_shadow(key, &mask([0, 0], 1.), shadow([0., 0.], 0.), "invalid", &[]),
        Err(Error::Syntax)
    );
    for values in [vec![f32::NAN], vec![-1.], vec![2.]] {
        assert!(matches!(
            AlphaMask::new(1, 1, [0, 0], values),
            Err(Error::Range)
        ));
    }
    assert!(matches!(
        AlphaMask::new(0, 1, [0, 0], vec![]),
        Err(Error::IndexSize)
    ));
    assert!(matches!(
        AlphaMask::new(1, 1, [0, 0], vec![]),
        Err(Error::IndexSize)
    ));
    assert!(matches!(
        AlphaMask::new(u32::MAX, u32::MAX, [0, 0], vec![]),
        Err(Error::Capacity)
    ));
}
#[test]
fn command_sources_are_immutable_arc_leases() {
    let image = Arc::new(ImageData::from_rgba(1, 1, vec![0, 255, 0, 255]).unwrap());
    let command = Command::Put {
        image: image.clone(),
        dx: 0,
        dy: 0,
        dirty: [0, 0, 1, 1],
    };
    drop(image);
    let mut r = surface(1, 1);
    r.replay(r.key(), &[command]).unwrap();
    assert_eq!(pixel(&r, 0, 0), [0, 255, 0, 255]);
}
