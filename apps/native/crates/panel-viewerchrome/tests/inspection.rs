use pm_panel_viewerchrome::*;
#[test]
fn marquee_requires_full_enclosure_filters_inactive_and_unpickable_and_shift_toggles() {
    let p = |x, y| Point { x, y };
    let candidates = vec![
        MarqueeCandidate {
            id: "a".into(),
            bounds: Rect::between(p(1.0, 1.0), p(3.0, 3.0)),
            active: true,
            pickable: true,
        },
        MarqueeCandidate {
            id: "b".into(),
            bounds: Rect::between(p(-1.0, 1.0), p(3.0, 3.0)),
            active: true,
            pickable: true,
        },
        MarqueeCandidate {
            id: "c".into(),
            bounds: Rect::between(p(2.0, 2.0), p(4.0, 4.0)),
            active: false,
            pickable: true,
        },
        MarqueeCandidate {
            id: "d".into(),
            bounds: Rect::between(p(2.0, 2.0), p(4.0, 4.0)),
            active: true,
            pickable: false,
        },
        MarqueeCandidate {
            id: "e".into(),
            bounds: Rect::between(p(4.0, 4.0), p(5.0, 5.0)),
            active: true,
            pickable: true,
        },
    ];
    let mut drag = Marquee::new(p(0.0, 0.0), vec!["a".into(), "other".into()], true).unwrap();
    drag.update(p(10.0, 10.0), true).unwrap();
    assert_eq!(drag.finish(&candidates).unwrap(), vec!["other", "e"]);
    drag.shift = false;
    assert_eq!(drag.finish(&candidates).unwrap(), vec!["a", "e"]);
}
#[test]
fn empty_click_shift_click_cancel_and_bad_candidate_preserve_before_selection() {
    let mut drag = Marquee::new(Point::default(), vec!["a".into()], true).unwrap();
    assert_eq!(drag.finish(&[]).unwrap(), vec!["a"]);
    drag.shift = false;
    assert!(drag.finish(&[]).unwrap().is_empty());
    drag.update(Point { x: 10.0, y: 10.0 }, true).unwrap();
    drag.cancel();
    assert!(drag.rectangle.is_none());
    assert_eq!(drag.finish(&[]).unwrap(), vec!["a"]);
    assert!(
        Marquee::new(
            Point {
                x: f64::NAN,
                y: 0.0
            },
            vec![],
            false
        )
        .is_err()
    );
}
#[test]
fn navigation_matches_actual_tag_function_outputs() {
    use pm_model::{Value, json::parse};
    let fixture = parse(include_str!("fixtures/navigation-v1.2.0.json")).unwrap();
    let n = |v: &Value, k: &str| v.get(k).unwrap().as_f64().unwrap();
    for case in fixture.get("cases").unwrap().as_array().unwrap() {
        let stage = case.get("stage").unwrap();
        let composition = case.get("composition").unwrap();
        let zoom = n(case, "zoom");
        let viewport = Viewport {
            stage: Size {
                width: n(stage, "width"),
                height: n(stage, "height"),
            },
            composition: Size {
                width: n(composition, "width"),
                height: n(composition, "height"),
            },
            fit_inset: 8.0,
        };
        let nav = Navigation {
            fit: false,
            zoom,
            pan: Point { x: 13.25, y: -17.5 },
        };
        let origin = nav.origin(viewport).unwrap();
        let expected = case.get("origin").unwrap();
        assert!((origin.x - n(expected, "x")).abs() < 1e-10);
        assert!((origin.y - n(expected, "y")).abs() < 1e-10);
        let rendered = preview_size(viewport.composition, zoom, 2.0, Quality::Auto, 0.5).unwrap();
        let expected = case.get("size").unwrap();
        assert_eq!(f64::from(rendered.width), n(expected, "width"));
        assert_eq!(f64::from(rendered.height), n(expected, "height"));
        let pointer = case.get("pointer").unwrap();
        let point = case.get("point").unwrap();
        let pan = case.get("pan").unwrap();
        // Construct navigation from the tagged pan, then verify its coordinate invariant.
        let nav = Navigation {
            fit: false,
            zoom,
            pan: Point {
                x: n(pan, "x"),
                y: n(pan, "y"),
            },
        };
        let actual = nav
            .composition_point(
                viewport,
                Point {
                    x: n(pointer, "x"),
                    y: n(pointer, "y"),
                },
            )
            .unwrap();
        assert!((actual.x - n(point, "x")).abs() < 1e-9);
        assert!((actual.y - n(point, "y")).abs() < 1e-9);
    }
}
