use super::*;
use std::time::Instant;

// Read the kernel publication: the outline deliberately keeps its own preview.
fn published_x(s: &Stage) -> f64 {
    let published = s.rig.kernel.published(PROJECT).unwrap();
    pm_kernel::QueryMirror::new(&published)
        .queries()
        .evaluate(None, "l0", "position.x", 0.5)
        .unwrap()
        .as_f64()
        .unwrap()
}

#[gpui::test]
fn drag_latency_app_viewer_layer_move(cx: &mut TestAppContext) {
    let mut s = stage(cx);
    // Command suspends alignment snapping for exact numeric movement checks.
    let modifiers = Modifiers {
        platform: true,
        ..Modifiers::none()
    };
    s.vcx.update(|window, _| window.activate());
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    s.refresh(cx);
    let before = s.pose().position;
    let (body, ..) = s.points();
    let from = s.point_of([body[0] + 30., body[1] + 30.]);
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, modifiers);
    let surface = s.surface.clone();
    settle(cx, move |cx| surface.read_with(cx, |g, _| g.idle()));
    s.refresh(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    for (event, dx) in [12., 24., 36., 48., 36., 24., 12.].into_iter().enumerate() {
        eprintln!(
            "DRAG_EVENT {}",
            serde_json::json!({"case":"viewer_layer_move","event":event})
        );
        let started = Instant::now();
        s.vcx
            .simulate_mouse_move(from + point(px(dx), px(0.)), MouseButton::Left, modifiers);
        let surface = s.surface.clone();
        settle(cx, |cx| {
            surface.read_with(cx, |g, _| g.idle())
                && (published_x(&s) - (before[0] + f64::from(dx))).abs() < 0.01
        });
        s.refresh(cx);
        s.vcx
            .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
        let applied = published_x(&s);
        let expected = before[0] + f64::from(dx);
        let stale = (applied - expected).abs() > 0.01;
        eprintln!(
            "DRAG_APP {}",
            serde_json::json!({"case":"viewer_layer_move", "event":event,
            "event_to_observed_ms":started.elapsed().as_secs_f64()*1000., "expected":expected,
            "applied":applied, "stale_or_reordered":stale, "observation":"settled kernel publication upper bound"})
        );
        assert!(!stale, "event {event}: {applied} vs {expected}");
    }
    eprintln!(
        "DRAG_EVENT {}",
        serde_json::json!({"case":"viewer_layer_move_burst","event":7})
    );
    let started = Instant::now();
    s.vcx
        .simulate_events_in_frame(
            (1..=60)
                .chain((6..60).rev())
                .map(|step| gpui::MouseMoveEvent {
                    position: from + point(px(step as f32), px(0.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: modifiers,
                }),
        );
    let surface = s.surface.clone();
    settle(cx, |cx| {
        surface.read_with(cx, |g, _| g.idle()) && (published_x(&s) - (before[0] + 6.)).abs() < 0.01
    });
    s.refresh(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    let applied = published_x(&s);
    let expected = before[0] + 6.;
    let stale = (applied - expected).abs() > 0.01;
    eprintln!(
        "DRAG_APP {}",
        serde_json::json!({"case":"viewer_layer_move_burst","event":7,
        "event_to_observed_ms":started.elapsed().as_secs_f64()*1000.,"expected":expected,"applied":applied,
        "stale_or_reordered":stale,"observation":"settled kernel publication upper bound"})
    );
    assert!(!stale, "burst applied {applied}, expected {expected}");
    s.vcx
        .simulate_mouse_up(from + point(px(6.), px(0.)), MouseButton::Left, modifiers);
    s.refresh(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
}
