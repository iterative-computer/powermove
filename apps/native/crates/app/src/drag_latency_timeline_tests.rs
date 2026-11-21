use super::*;
use std::time::Instant;

// Observe after worker settlement: this is an upper bound on model application,
// not a physical presentation timestamp. Every sample is logged, including misses.
fn report(case: &str, event: usize, started: Instant, expected: f64, applied: f64, tolerance: f64) {
    let stale = (expected - applied).abs() > tolerance;
    eprintln!(
        "DRAG_APP {}",
        serde_json::json!({
            "case": case, "event": event, "event_to_observed_ms": started.elapsed().as_secs_f64()*1000.,
            "expected": expected, "applied": applied, "stale_or_reordered": stale,
            "observation": if case.starts_with("strip_move") { "settled native frame geometry upper bound" } else { "settled model upper bound" }
        })
    );
    assert!(
        !stale,
        "{case} event {event}: expected {expected}, applied {applied}"
    );
}

// Shipping17 previews a strip in its native frame until mouse-up commits it.
fn painted_start(s: &Stage, layer: &str) -> f64 {
    let frame = s.surface.read_with(&s.vcx, |t, _| t.frame().unwrap());
    frame
        .ops
        .iter()
        .find(|op| op.id == format!("clip:{layer}"))
        .unwrap()
        .detail["from"]
        .as_f64()
        .unwrap()
}

#[gpui::test]
fn drag_latency_app_strip_move(cx: &mut TestAppContext) {
    let mut s = stage(cx, 3);
    s.vcx.update(|window, _| window.activate());
    let before = s.start_of("l1");
    let from = s.at(1, before + 1.);
    let pps = s.surface.read_with(&s.vcx, |t, _| t.pps());
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    for (event, dx) in [12., 24., 36., 48., 36., 24., 12.].into_iter().enumerate() {
        eprintln!(
            "DRAG_EVENT {}",
            serde_json::json!({"case":"strip_move","event":event})
        );
        let started = Instant::now();
        s.vcx.simulate_mouse_move(
            from + point(px(dx), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        s.idle(cx);
        s.vcx
            .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
        report(
            "strip_move",
            event,
            started,
            before + f64::from(dx) / pps,
            painted_start(&s, "l1"),
            1. / 30. + 0.001,
        );
    }
    eprintln!(
        "DRAG_EVENT {}",
        serde_json::json!({"case":"strip_move_burst","event":7})
    );
    let started = Instant::now();
    s.vcx
        .simulate_events_in_frame(
            (1..=60)
                .chain((6..60).rev())
                .map(|step| gpui::MouseMoveEvent {
                    position: from + point(px(1. * step as f32), px(0.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::none(),
                }),
        );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    report(
        "strip_move_burst",
        7,
        started,
        before + 6. / pps,
        painted_start(&s, "l1"),
        1. / 30. + 0.001,
    );
    s.vcx.simulate_mouse_up(
        from + point(px(6.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    assert!(
        (s.start_of("l1") - (before + 6. / pps)).abs() <= 1. / 30. + 0.001,
        "the released strip must commit its final preview"
    );
}

#[gpui::test]
fn drag_latency_app_precomp_trim(cx: &mut TestAppContext) {
    let mut project: serde_json::Value =
        serde_json::from_str(&crate::v4_view_tests::project_json(2)).unwrap();
    project["layers"][0]["type"] = serde_json::json!("precomp");
    project["layers"][0]["d"] = serde_json::json!({"comp":"inner","w":100,"h":100});
    project["comps"]["inner"] = serde_json::json!({"id":"inner","name":"Inner","w":100,"h":100,"fps":30,"dur":2,"bg":"#000000","layers":[]});
    let mut s = stage_project(cx, project.to_string());
    s.vcx.update(|window, _| window.activate());
    let before = s.out_of("l0");
    let from = s.at(0, before) - point(px(2.), px(0.));
    let pps = s.surface.read_with(&s.vcx, |t, _| t.pps());
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    for (event, dx) in [-12., -24., -36., -48., -36., -24., -12.]
        .into_iter()
        .enumerate()
    {
        eprintln!(
            "DRAG_EVENT {}",
            serde_json::json!({"case":"precomp_trim","event":event})
        );
        let started = Instant::now();
        s.vcx.simulate_mouse_move(
            from + point(px(dx), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        s.idle(cx);
        s.vcx
            .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
        report(
            "precomp_trim",
            event,
            started,
            before + f64::from(dx) / pps,
            s.out_of("l0"),
            1. / 30. + 0.001,
        );
    }
    eprintln!(
        "DRAG_EVENT {}",
        serde_json::json!({"case":"precomp_trim_burst","event":7})
    );
    let started = Instant::now();
    s.vcx
        .simulate_events_in_frame(
            (1..=60)
                .chain((6..60).rev())
                .map(|step| gpui::MouseMoveEvent {
                    position: from + point(px(-1. * step as f32), px(0.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::none(),
                }),
        );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    report(
        "precomp_trim_burst",
        7,
        started,
        before - 6. / pps,
        s.out_of("l0"),
        1. / 30. + 0.001,
    );
    s.vcx.simulate_mouse_up(
        from + point(px(-6.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
}

#[gpui::test]
fn drag_latency_app_property_scrub(cx: &mut TestAppContext) {
    let mut s = stage(cx, 1);
    s.vcx.update(|window, _| window.activate());
    let value = |s: &Stage| {
        s.surface.read_with(&s.vcx, |t, _| {
            t.model()
                .index
                .channel("l0", "opacity")
                .unwrap()
                .value
                .as_f64()
                .unwrap()
        })
    };
    let before = value(&s);
    let from = s.at(0, 0.5);
    s.surface.update(&mut s.vcx, |t, _| {
        t.press_property("l0".into(), "opacity".into(), from)
    });
    for (event, dx) in [-12., -24., -36., -48., -36., -24., -12.]
        .into_iter()
        .enumerate()
    {
        eprintln!(
            "DRAG_EVENT {}",
            serde_json::json!({"case":"property_scrub","event":event})
        );
        let started = Instant::now();
        s.vcx.simulate_mouse_move(
            from + point(px(dx), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        s.idle(cx);
        s.vcx
            .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
        report(
            "property_scrub",
            event,
            started,
            before + f64::from(dx) * 0.5,
            value(&s),
            0.001,
        );
    }
    eprintln!(
        "DRAG_EVENT {}",
        serde_json::json!({"case":"property_scrub_burst","event":7})
    );
    let started = Instant::now();
    s.vcx
        .simulate_events_in_frame(
            (1..=60)
                .chain((6..60).rev())
                .map(|step| gpui::MouseMoveEvent {
                    position: from + point(px(-1. * step as f32), px(0.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::none(),
                }),
        );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    report(
        "property_scrub_burst",
        7,
        started,
        before - 3.,
        value(&s),
        0.001,
    );
    s.vcx.simulate_mouse_up(
        from + point(px(-6.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
}

#[gpui::test]
fn drag_latency_app_splitter(cx: &mut TestAppContext) {
    let mut s = stage(cx, 2);
    s.vcx.update(|window, _| window.activate());
    let before = s.surface.read_with(&s.vcx, |t, _| t.layer_column());
    let from = point(s.origin().x + px(before as f32), s.at(1, 0.).y);
    s.vcx
        .simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    for (event, dx) in [12., 24., 36., 48., 36., 24., 12.].into_iter().enumerate() {
        eprintln!(
            "DRAG_EVENT {}",
            serde_json::json!({"case":"splitter","event":event})
        );
        let started = Instant::now();
        s.vcx.simulate_mouse_move(
            from + point(px(dx), px(0.)),
            MouseButton::Left,
            Modifiers::none(),
        );
        s.idle(cx);
        s.vcx
            .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
        let applied = s.surface.read_with(&s.vcx, |t, _| t.layer_column());
        report(
            "splitter",
            event,
            started,
            before + f64::from(dx),
            applied,
            0.001,
        );
    }
    eprintln!(
        "DRAG_EVENT {}",
        serde_json::json!({"case":"splitter_burst","event":7})
    );
    let started = Instant::now();
    s.vcx
        .simulate_events_in_frame(
            (1..=60)
                .chain((6..60).rev())
                .map(|step| gpui::MouseMoveEvent {
                    position: from + point(px(1. * step as f32), px(0.)),
                    pressed_button: Some(MouseButton::Left),
                    modifiers: Modifiers::none(),
                }),
        );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
    report(
        "splitter_burst",
        7,
        started,
        before + 6.,
        s.surface.read_with(&s.vcx, |t, _| t.layer_column()),
        0.001,
    );
    s.vcx.simulate_mouse_up(
        from + point(px(6.), px(0.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    s.idle(cx);
    s.vcx
        .simulate_events_in_frame(std::iter::empty::<gpui::MouseMoveEvent>());
}
