use pm_panel_viewerchrome::*;
fn viewport() -> Viewport {
    Viewport {
        stage: Size {
            width: 800.0,
            height: 600.0,
        },
        composition: Size {
            width: 1920.0,
            height: 1080.0,
        },
        fit_inset: 8.0,
    }
}
fn near(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-9, "{a} != {b}");
}
#[test]
fn presets_and_custom_selection_are_semantic() {
    let nav = Navigation::default();
    assert_eq!(nav.choice(viewport()).unwrap(), ZoomChoice::Fit);
    for &zoom in ZOOM_PRESETS {
        assert_eq!(
            nav.set_zoom(zoom).unwrap().choice(viewport()).unwrap(),
            ZoomChoice::Preset(zoom)
        );
    }
    assert_eq!(
        nav.set_zoom(0.73).unwrap().choice(viewport()).unwrap(),
        ZoomChoice::Custom(0.73)
    );
}
#[test]
fn fit_tracks_resize_and_ignores_old_pan() {
    let nav = Navigation {
        pan: Point { x: 999.0, y: 222.0 },
        ..Navigation::default()
    };
    near(nav.shown(viewport()).unwrap(), 784.0 / 1920.0);
    near(nav.origin(viewport()).unwrap().x, 8.0);
    let mut v = viewport();
    v.stage.width = 400.0;
    near(nav.shown(v).unwrap(), 384.0 / 1920.0);
    assert_eq!(nav.set_zoom(1.0).unwrap().pan, Point::default());
}
#[test]
fn zoom_clamps_and_rejects_nonfinite_without_changing_state() {
    let nav = Navigation::default();
    assert_eq!(nav.set_zoom(-1.0).unwrap().zoom, 0.05);
    assert_eq!(nav.set_zoom(100.0).unwrap().zoom, 8.0);
    for z in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(nav.set_zoom(z).is_err());
    }
    assert_eq!(nav, Navigation::default());
}
#[test]
fn pointer_anchored_zoom_preserves_composition_point_and_is_reversible() {
    let v = viewport();
    let nav = Navigation::default();
    let pointer = Point {
        x: 124.25,
        y: 520.5,
    };
    let original = nav.composition_point(v, pointer).unwrap();
    let zoomed = nav.zoom_at(v, pointer, 1.25).unwrap();
    let after = zoomed.composition_point(v, pointer).unwrap();
    near(original.x, after.x);
    near(original.y, after.y);
    let reversed = zoomed.zoom_at(v, pointer, 0.8).unwrap();
    near(reversed.shown(v).unwrap(), nav.shown(v).unwrap());
    near(reversed.pan.x, 0.0);
    near(reversed.pan.y, 0.0);
}
#[test]
fn wheel_pan_shift_remap_and_zoom_step_bounds() {
    let v = viewport();
    let nav = Navigation::default();
    let pan = nav
        .scroll(
            v,
            Point::default(),
            Scroll::Pan {
                delta: Point { x: 0.0, y: 20.5 },
                shift: true,
            },
        )
        .unwrap();
    assert!(!pan.fit);
    assert_eq!(pan.pan, Point { x: -20.5, y: 0.0 });
    near(pan.zoom, nav.shown(v).unwrap());
    let pan2 = pan
        .scroll(
            v,
            Point::default(),
            Scroll::Pan {
                delta: Point { x: 4.5, y: -10.0 },
                shift: true,
            },
        )
        .unwrap();
    assert_eq!(pan2.pan, Point { x: -25.0, y: 10.0 });
    let zoom = nav
        .scroll(v, Point::default(), Scroll::ZoomPixels(-10000.0))
        .unwrap();
    near(zoom.zoom, nav.shown(v).unwrap() * 2.0);
    let zoom = nav
        .scroll(v, Point::default(), Scroll::ZoomPixels(10000.0))
        .unwrap();
    near(zoom.zoom, nav.shown(v).unwrap() * 0.5);
}
#[test]
fn pan_drag_is_from_start_and_shift_is_live() {
    let v = viewport();
    let drag = PanDrag {
        start: Navigation::default(),
    };
    assert_eq!(
        drag.update(v, Point { x: 3.0, y: -4.0 }, false)
            .unwrap()
            .pan,
        Point { x: 3.0, y: -4.0 }
    );
    assert_eq!(
        drag.update(v, Point { x: 5.0, y: -8.0 }, true).unwrap().pan,
        Point { x: 10.0, y: -16.0 }
    );
    assert!(drag.start.fit);
}
#[test]
fn recovery_is_shown_only_for_fully_offscreen_manual_composition() {
    let v = viewport();
    let nav = Navigation::default().set_zoom(1.0).unwrap();
    assert!(!nav.out_of_view(v, false).unwrap());
    let off = nav.pan_by(v, Point { x: 2000.0, y: 0.0 }).unwrap();
    assert!(off.out_of_view(v, false).unwrap());
    assert!(!off.out_of_view(v, true).unwrap());
    assert!(!off.fit().out_of_view(v, false).unwrap());
    assert_eq!(off.fit().pan, Point::default());
}
#[test]
fn zoom_click_option_click_marquee_and_cancel_preview() {
    let v = viewport();
    let nav = Navigation::default();
    let start = Point { x: 200.0, y: 150.0 };
    let end = Point { x: 600.0, y: 450.0 };
    near(
        nav.zoom_release(v, start, end, false, false, 3.0)
            .unwrap()
            .zoom,
        nav.shown(v).unwrap() * 1.25,
    );
    near(
        nav.zoom_release(v, start, end, true, true, 3.0)
            .unwrap()
            .zoom,
        nav.shown(v).unwrap() * 0.8,
    );
    let marquee = nav.zoom_release(v, start, end, true, false, 3.0).unwrap();
    assert!(marquee.zoom > nav.shown(v).unwrap());
    near(marquee.pan.x, 0.0);
    near(marquee.pan.y, 0.0);
    // The rectangle is ephemeral, no view mutation occurs until release.
    assert!(nav.fit);
}
#[test]
fn invalid_geometry_and_overflow_are_rejected() {
    let nav = Navigation::default();
    let mut v = viewport();
    v.stage.width = 0.0;
    assert!(nav.shown(v).is_err());
    v = viewport();
    v.fit_inset = f64::NAN;
    assert!(nav.shown(v).is_err());
    assert!(
        nav.zoom_at(
            viewport(),
            Point {
                x: f64::NAN,
                y: 0.0
            },
            2.0
        )
        .is_err()
    );
    assert!(nav.zoom_at(viewport(), Point::default(), 0.0).is_err());
    assert!(
        nav.zoom_release(
            viewport(),
            Point::default(),
            Point::default(),
            true,
            false,
            -1.0
        )
        .is_err()
    );
}
#[test]
fn explicit_quality_does_not_follow_zoom_and_auto_tracks_density() {
    let size = Size {
        width: 1920.0,
        height: 1080.0,
    };
    for (quality, width) in [
        (Quality::Full, 1920),
        (Quality::Half, 960),
        (Quality::Quarter, 480),
    ] {
        assert_eq!(
            preview_size(size, 0.1, 2.0, quality, quality.scale())
                .unwrap()
                .width,
            width
        );
    }
    assert_eq!(
        preview_size(size, 0.25, 2.0, Quality::Auto, 1.0)
            .unwrap()
            .width,
        960
    );
    assert_eq!(
        preview_size(size, 0.25, 2.0, Quality::Auto, 0.5)
            .unwrap()
            .width,
        480
    );
    assert_eq!(
        preview_size(size, 8.0, 4.0, Quality::Auto, 1.0)
            .unwrap()
            .width,
        1920
    );
    assert!(preview_size(size, 1.0, 1.0, Quality::Auto, f64::NAN).is_err());
}
