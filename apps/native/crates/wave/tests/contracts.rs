use pm_wave::*;
#[test]
fn defaults_and_validation() {
    for kind in [WaveType::Vertical, WaveType::Horizontal, WaveType::Center] {
        let s = WaveStyle::new(kind);
        s.validate().unwrap();
        assert!(s.duration() > 3.);
    }
    let s = WaveStyle::default();
    assert_eq!(s.anticipation, 0.5);
    assert_eq!(s.travel, 2.28);
    assert_eq!(s.amplitude, 34.);
    assert_eq!(s.particle_count, 64.);
    let mut bad = s.clone();
    bad.lens_width = 0.;
    assert!(bad.validate().is_err());
    bad = s.clone();
    bad.amplitude = f64::NAN;
    assert!(bad.validate().is_err());
    bad = s.clone();
    bad.particle_count = 257.;
    assert!(bad.validate().is_err());
    bad = s;
    bad.orientation = 1.1;
    assert!(bad.validate().is_err());
    for invalid in ["#xy0000", "#abc", "##123456", "é12345"] {
        assert!(Palette::hex(invalid).is_err());
    }
    assert_eq!(
        Palette::hex("#528dff").unwrap(),
        Palette::Single([82, 141, 255])
    );
    let mut clock = WaveClock::default();
    assert!(clock.scrub(f64::NAN).is_err());
    clock.scrub(-1.).unwrap();
    assert_eq!(clock.progress(), 0.);
    clock.scrub(2.).unwrap();
    assert_eq!(clock.progress(), 1.);
}
#[test]
fn reversing_preserves_current_state_and_uses_original_collapse_curve() {
    let style = WaveStyle::default();
    let mut clock = WaveClock::default();
    clock.play();
    clock.tick(style.duration() * 0.4, &style, false);
    assert!((clock.progress() - 0.4).abs() < 1e-12);
    clock.reverse();
    assert_eq!(clock.take_finish(), None);
    assert!((clock.progress() - 0.4).abs() < 1e-12);
    clock.tick(style.duration() / style.return_speed * 0.5, &style, false);
    let expected = 0.4 * (1. - bezier_ease([0.22, 1., 0.36, 1.], 0.5));
    assert!((clock.progress() - expected).abs() < 1e-12);
    clock.tick(10., &style, false);
    assert_eq!(clock.progress(), 0.);
    assert_eq!(clock.take_finish(), Some(false));
    assert_eq!(clock.take_finish(), None);
    assert!(!clock.is_running());
    clock.tick(10., &style, false);
    assert_eq!(clock.take_finish(), None);
    clock.play();
    clock.tick(10., &style, false);
    assert_eq!(clock.take_finish(), Some(true));
    clock.reset();
    assert_eq!(clock.progress(), 0.);
}
#[test]
fn reduced_motion_uses_300ms_linear_crossfade_and_ignores_return_speed() {
    let style = WaveStyle::default();
    let mut clock = WaveClock::default();
    clock.play();
    clock.tick(0.15, &style, true);
    assert_eq!(clock.progress(), 0.5);
    clock.reverse();
    clock.tick(0.075, &style, true);
    assert!((clock.progress() - 0.25).abs() < 1e-12);
    clock.tick(0.075, &style, true);
    assert_eq!(clock.take_finish(), Some(false));
}
#[test]
fn source_lean_and_center_hold() {
    let source = Rect {
        x: 100.,
        y: 300.,
        w: 150.,
        h: 44.,
    };
    let s = WaveStyle::default();
    let a = Front::at(0.2, source, 620., 400., &s, None);
    let b = Front::at(
        0.2,
        source,
        620.,
        400.,
        &s,
        Some(Point { x: 250., y: 320. }),
    );
    assert!(b.tilt > a.tilt);
    assert!(b.ox > a.ox);
    let s = WaveStyle::new(WaveType::Center);
    let a = Front::at(0.2, source, 620., 400., &s, None);
    let b = Front::at(
        0.2,
        source,
        620.,
        400.,
        &s,
        Some(Point { x: 250., y: 320. }),
    );
    assert_eq!(a.tilt, b.tilt);
    assert_eq!(a.ox, b.ox);
    assert_eq!(a.hold, 22.);
}
#[cfg(target_os = "macos")]
#[test]
fn exact_endpoints_crossfade_same_view_and_resource_leases() {
    let device = metal::Device::system_default().expect("Metal required");
    let mut r = WaveRenderer::new(&device).unwrap();
    let from = image::RgbaImage::from_pixel(64, 48, image::Rgba([40, 80, 120, 255]));
    let to = image::RgbaImage::from_pixel(64, 48, image::Rgba([200, 160, 80, 255]));
    let s = WaveStyle {
        corner_radius: 0.,
        ..WaveStyle::default()
    };
    let source = Rect {
        x: 24.,
        y: 36.,
        w: 16.,
        h: 8.,
    };
    r.prepare(&from, Some(&to), 64., &s).unwrap();
    let baseline = r.texture_count();
    assert_eq!(
        r.render(0., source, 64., 48., None, false)
            .unwrap()
            .read_image()
            .unwrap(),
        from
    );
    assert_eq!(
        r.render(1., source, 64., 48., None, false)
            .unwrap()
            .read_image()
            .unwrap(),
        to
    );
    let fade = r
        .render(0.5, source, 64., 48., None, true)
        .unwrap()
        .read_image()
        .unwrap();
    assert!(fade.pixels().all(|p| p.0 == [120, 120, 100, 255]));
    assert_eq!(
        r.texture_count(),
        baseline,
        "no new textures in the frame loop"
    );
    let held = r.render(0.4, source, 64., 48., None, false).unwrap();
    r.release();
    assert_eq!(r.texture_count(), 1, "consumer lease keeps only its output");
    drop(held);
    assert_eq!(r.texture_count(), 0);
    r.prepare(&from, None, 64., &s).unwrap();
    assert_eq!(
        r.render(1., source, 64., 48., None, false)
            .unwrap()
            .read_image()
            .unwrap(),
        from
    );
    r.release();
    assert_eq!(r.texture_count(), 0);
}
