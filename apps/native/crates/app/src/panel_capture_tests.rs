//! Headless pictures of the built-in panels (language §16 review loop): the
//! real V8 panels and kernel on a synthetic project, painted by GPUI's Metal
//! renderer without a screen. AppKit control slots are absent, as in the host
//! tests. Run on demand:
//! `PM_PANEL_CAPTURE=/tmp/pm-shots/panels cargo test -p pm-native-app capture_builtin_panels -- --ignored`
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{PROJECT, publication, rig, rig_project};
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use std::sync::Arc;

fn wait_export_publication(ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        if let Some(result) = ticket.try_take() {
            result.unwrap();
            return;
        }
        assert!(std::time::Instant::now() < deadline, "export publication stalled");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
#[ignore = "writes PNGs; set PM_PANEL_CAPTURE"]
fn capture_builtin_panels() {
    let dir = std::env::var("PM_PANEL_CAPTURE").unwrap_or_else(|_| "/tmp/pm-shots/panels".into());
    std::fs::create_dir_all(&dir).unwrap();
    let only = std::env::var("PM_PANEL_CAPTURE_ONLY").ok();
    let export_view = std::env::var("PM_PANEL_CAPTURE_EXPORT_VIEW").ok();
    if let Some(view) = &export_view {
        assert!(["settings", "settings-bottom", "project", "rendering", "queue", "empty"].contains(&view.as_str()), "unknown export capture view");
    }
    for (panel, w, h) in [
        ("timeline", 1100., 360.),
        ("inspector", 320., 600.),
        ("layer-effects", 320., 600.),
        ("assets", 300., 500.),
        ("fxbrowser", 320., 420.),
        ("viewer", 800., 480.),
        ("agent", 360., 620.),
        ("settings", 760., 520.),
        ("export", 600., 520.),
        ("color", 360., 760.),
    ] {
        if only
            .as_deref()
            .is_some_and(|o| !o.split(',').any(|p| p == panel))
        {
            continue;
        }
        for (name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            objc2::rc::autoreleasepool(|_| {
                let tokens = Tokens::new(scheme, Accessibility::default());
                let r = if let Ok(path) = std::env::var("PM_PANEL_CAPTURE_PROJECT") {
                    if std::path::Path::new(&path)
                        .extension()
                        .is_some_and(|e| e == "pmv")
                    {
                        let opened = pm_kernel::open_project(
                            std::path::Path::new(&path),
                            Default::default(),
                        )
                        .unwrap();
                        crate::v4_view_tests::rig_source(panel, pm_kernel::OpenSource::File(opened))
                    } else {
                        rig_project(panel, &std::fs::read_to_string(path).unwrap())
                    }
                } else {
                    rig(panel, 8)
                };
                if let Ok(layer) = std::env::var("PM_PANEL_CAPTURE_LAYER") {
                    r.kernel
                        .call(
                            PROJECT,
                            pm_kernel::Request::new(
                                pm_kernel::Caller::native(),
                                pm_kernel::SelectionCall::Layers {
                                    ids: vec![layer],
                                    add: false,
                                },
                            ),
                        )
                        .wait()
                        .unwrap()
                        .unwrap();
                    for (_, ticket) in r.runtime.publish() {
                        let ticket = ticket.unwrap();
                        while ticket.try_take().is_none() {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                    }
                }
                if let Ok(layer) = std::env::var("PM_PANEL_CAPTURE_EXPAND") {
                    let ticket = r.runtime.timeline_action(serde_json::json!({"type":"disclosure", "id":pm_panel_timeline::v4::row_id(&pm_panel_timeline::RowId::Layer(layer)), "open":true})).unwrap();
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                    loop {
                        if let Some(result) = ticket.try_take() {
                            result.unwrap();
                            break;
                        }
                        assert!(std::time::Instant::now() < deadline);
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    let client = pm_panel_timeline::TimelineClient::new(
                        r.kernel.clone(),
                        PROJECT,
                        pm_kernel::Caller::native(),
                    )
                    .unwrap();
                    let rows =
                        pm_panel_timeline::v4::project_window(&client.model, 0, 512).unwrap();
                    std::fs::write(
                        format!("{dir}/timeline-{name}-rows.json"),
                        serde_json::to_vec_pretty(&rows).unwrap(),
                    )
                    .unwrap();
                    std::fs::write(
                        format!("{dir}/timeline-{name}-ui.txt"),
                        format!(
                            "{:?}",
                            r.kernel.published(PROJECT).unwrap().ui_state.timeline
                        ),
                    )
                    .unwrap();
                }
                if panel == "export" && (std::env::var_os("PM_PANEL_CAPTURE_QUEUE").is_some() || export_view.as_deref().is_some_and(|v|v != "empty")) {
                    let ticket = r.runtime.observe_render_queue(serde_json::json!([
                        {"id":1,"name":"Opening","format":"mp4","path":"/Exports/Opening.mp4","status":"Rendering","progress":0.42,"error":""},
                        {"id":2,"name":"Logo","format":"prores","profile":"422","path":"/Exports/Logo.mov","status":"Pending","progress":0.,"error":""},
                        {"id":3,"name":"Yesterday","format":"mp4","path":"/Exports/Yesterday.mp4","status":"Done","progress":1.,"error":"","finishedAt":1794686520},
                        {"id":4,"name":"Credits","format":"mp4","path":"/Exports/Credits.mp4","status":"Failed","progress":0.,"error":"Missing media","finishedAt":1794686460},
                        {"id":5,"name":"Lower third","format":"mp4","path":"/Exports/Lower-third.mp4","status":"Cancelled","progress":0.,"error":"","finishedAt":1794686280}
                    ])).unwrap();
                    while ticket.try_take().is_none() { std::thread::sleep(std::time::Duration::from_millis(1)); }
                }
                let preview = tempfile::tempdir().unwrap();
                let preview_key = "export-preview/capture-fixture";
                if panel == "export" {
                    if matches!(export_view.as_deref(), Some("settings" | "settings-bottom" | "project")) {
                        // An idle sheet has a pending item and a finished export.
                        wait_export_publication(r.runtime.observe_render_queue(serde_json::json!([
                            {"id":2,"name":"Logo","format":"prores","path":"/Exports/Logo.mov","status":"Pending","progress":0.,"error":""},
                            {"id":3,"name":"Opening","format":"mp4","path":"/Exports/Opening.mp4","status":"Done","progress":1.,"error":"","finishedAt":1794686520}
                        ])).unwrap());
                    } else if export_view.as_deref() == Some("rendering") {
                        wait_export_publication(r.runtime.observe_render_queue(serde_json::json!([
                            {"id":1,"name":"Opening","format":"mp4","path":"/Exports/Opening.mp4","status":"Rendering","progress":0.42,"error":""},
                            {"id":2,"name":"Logo","format":"prores","path":"/Exports/Logo.mov","status":"Pending","progress":0.,"error":""},
                            {"id":3,"name":"Credits","format":"mp4","path":"/Exports/Credits.mp4","status":"Pending","progress":0.,"error":""},
                            {"id":4,"name":"Teaser","format":"mp4","path":"/Exports/Teaser.mp4","status":"Pending","progress":0.,"error":""}
                        ])).unwrap());
                    } else if export_view.as_deref() == Some("empty") {
                        wait_export_publication(r.runtime.observe_render_queue(serde_json::json!([])).unwrap());
                    }
                    wait_export_publication(r.runtime.observe_export_destination(serde_json::json!({"path":"/Exports/Launch Film.mp4","format":"mp4"})).unwrap());
                    if std::env::var_os("PM_PANEL_CAPTURE_QUEUE").is_some()
                        || export_view.as_deref().is_some_and(|v|v == "rendering" || v == "queue") {
                        let image = preview.path().join("frame.png");
                        image::RgbImage::from_fn(640, 360, |x, y| {
                            if (x as i64 - 320).pow(2) + (y as i64 - 180).pow(2) < 7000 {
                                image::Rgb([250, 170, 105])
                            } else {
                                image::Rgb([26, 48 + (y / 10) as u8, 85 + (x / 12) as u8])
                            }
                        }).save(&image).unwrap();
                        crate::media_thumbs::register(preview_key, image);
                        wait_export_publication(r.runtime.observe_export(serde_json::json!({
                            "phase":"rendering","frame":126,"total":300,
                            "elapsed":12.,"eta":17.,"preview":preview_key,
                        })).unwrap());
                    }
                    if std::env::var_os("PM_PANEL_CAPTURE_QUEUE").is_some()
                        || export_view.as_deref().is_some_and(|v|v == "queue" || v == "empty") {
                        wait_export_publication(r.runtime.show_section("export", "queue").unwrap());
                    } else {
                        wait_export_publication(r.runtime.show("export").unwrap());
                    }
                }
                let h = std::env::var("PM_PANEL_CAPTURE_HEIGHT")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(h);
                let _preset_profile = tempfile::tempdir().unwrap();
                if panel == "fxbrowser" && std::env::var_os("PM_PRESET_CAPTURE").is_some() {
                    r.runtime.set_presets_folder(_preset_profile.path());
                    let folder = _preset_profile.path().join("presets");
                    std::fs::create_dir_all(&folder).unwrap();
                    std::fs::write(folder.join("move.json"),r#"{"schemaVersion":1,"id":"move","name":"Gentle Move","effects":[],"properties":[{"path":"opacity","channel":{"v":100,"kf":[]}}]}"#).unwrap();
                    if std::env::var("PM_PRESET_CAPTURE").as_deref() == Ok("sheet") {
                        let ticket = r
                            .runtime
                            .preset_sheet(serde_json::json!({"layer":"l0"}))
                            .unwrap();
                        while ticket.try_take().is_none() {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                    } else {
                        for (_, t) in r.runtime.publish() {
                            let t = t.unwrap();
                            while t.try_take().is_none() {
                                std::thread::sleep(std::time::Duration::from_millis(1));
                            }
                        }
                    }
                }
                let mut first = publication(&r);
                let hovered_job = match export_view.as_deref() {
                    Some("rendering") => Some("queue.job.3"),
                    Some("queue") => Some("queue.job.4"),
                    _ if std::env::var_os("PM_PANEL_CAPTURE_QUEUE").is_some() => Some("queue.job.4"),
                    _ => None,
                };
                if panel == "export" && let Some(hovered_job) = hovered_job {
                    let target = first.surface.nodes().values().find(|n| n.props.get("id").and_then(serde_json::Value::as_str) == Some(hovered_job)).unwrap().id;
                    let ticket = r.runtime.event("export", first.generation, first.surface.revision(), target, "pointerenter", Default::default()).unwrap();
                    while ticket.try_take().is_none() { std::thread::sleep(std::time::Duration::from_millis(1)); }
                    first = r.runtime.take_publications().pop().unwrap_or(first);
                }
                if panel == "export" && export_view.as_deref() == Some("project") {
                    let target = first.surface.nodes().values().find(|n|
                        n.props.get("role").and_then(serde_json::Value::as_str) == Some("radio")
                            && n.props.get("label").and_then(serde_json::Value::as_str) == Some("Project")
                    ).unwrap().id;
                    let ticket = r.runtime.event("export", first.generation, first.surface.revision(), target, "click", Default::default()).unwrap();
                    while ticket.try_take().is_none() {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    first = r.runtime.take_publications().pop().unwrap();
                    assert!(first.surface.nodes().values().any(|n| n.props.get("id").and_then(serde_json::Value::as_str) == Some("export.project.editable")));
                }
                if panel == "export"
                    && matches!(export_view.as_deref(), Some("settings" | "settings-bottom" | "project"))
                {
                    // Drive the same host wake that republishes completed encoder checks.
                    let summary = if export_view.as_deref() == Some("project") {
                        "Synthetic Film.pmv"
                    } else {
                        "300 frames · 10 s · 640×360"
                    };
                    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
                    while !first.surface.nodes().values().any(|node| {
                        node.props.get("id").and_then(serde_json::Value::as_str) == Some("export.summary")
                            && node.props.get("label").and_then(serde_json::Value::as_str)
                                == Some(summary)
                    }) {
                        if let Some(ticket) = r.runtime.publish_finished_checks().transpose().unwrap() {
                            wait_export_publication(ticket);
                        }
                        if let Some(next) = r.runtime.take_publications().pop() {
                            first = next;
                        }
                        assert!(std::time::Instant::now() < deadline, "export plan and encoder checks did not settle");
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                }
                if panel == "export" && export_view.as_deref() == Some("rendering") {
                    assert!(first.surface.nodes().values().any(|node| {
                        node.props.get("resource").and_then(serde_json::Value::as_str) == Some(preview_key)
                    }), "the Export sheet must publish the last encoded frame");
                }
                let runtime = r.runtime.clone();
                let mut cx = HeadlessAppContext::with_platform(
                    Arc::new(
                        gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                            .with_automatic_optical_sizing(),
                    ),
                    Arc::new(()),
                    || {
                        Some(Box::new(
                            gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                        ))
                    },
                );
                let panel_fill = tokens.colors.panel;
                let capture_timeline = panel == "timeline"
                    && std::env::var_os("PM_TIMELINE_HEADER_CAPTURE").is_some();
                let timeline_tokens = tokens.clone();
                let window = cx
                    .open_window(size(px(w), px(h)), |window, cx| {
                        let natives = Natives::default();
                        let outline = natives.outline.clone();
                        if capture_timeline {
                            use crate::timeline_tests::{Recorder, Recording};
                            let delegate = std::rc::Rc::new(Recording(std::rc::Rc::new(Recorder::default())));
                            let timeline = cx.new(|cx| crate::timeline_surface::TimelineSurface::new(
                                r.kernel.clone(),
                                PROJECT.into(),
                                timeline_tokens.clone(),
                                delegate,
                                outline.clone(),
                                Default::default(),
                                cx,
                            ));
                            natives.views.borrow_mut().insert("timeline".into(), timeline.into());
                        }
                        let view = cx.new(|cx| {
                            SurfaceView::new(runtime, first, tokens, natives, window, cx)
                        });
                        if capture_timeline {
                            outline.bind(view.downgrade());
                        }
                        cx.new(|_| Frame {
                            view,
                            fill: panel_fill,
                            timecode: None,
                        })
                    })
                    .unwrap();
                for _ in 0..40 {
                    cx.run_until_parked();
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                if capture_timeline {
                    // The live AppKit layer has no window in this harness.
                    // Paint its first-frame text at the same origin.
                    let slot = crate::playback::timecode_slot(&timeline_tokens);
                    window.update(&mut cx, |frame, _, cx| {
                        let bounds = frame.view.read(cx)
                            .timeline_timecode_bounds(px(w), px(slot.w))
                            .expect("timeline timecode should fit the capture");
                        frame.timecode = Some((
                            bounds.left(),
                            bounds.center().y - px(timeline_tokens.metrics.ruler / 2.),
                            timeline_tokens,
                        ));
                        cx.notify();
                    }).unwrap();
                    cx.run_until_parked();
                }
                if std::env::var_os("PM_PANEL_CAPTURE_BOTTOM").is_some()
                    || panel == "export" && export_view.as_deref() == Some("settings-bottom")
                {
                    window
                        .update(&mut cx, |frame, _, cx| {
                            frame
                                .view
                                .update(cx, |view, cx| view.review_scroll_to(f32::MAX, cx));
                        })
                        .unwrap();
                    cx.run_until_parked();
                }
                if panel == "export" {
                    // Draw again after the asset decode or a programmatic scroll.
                    for _ in 0..3 {
                        cx.update_window(window.into(), |_, window, cx| {
                            window.refresh();
                            window.draw(cx).clear(cx);
                        }).unwrap();
                        cx.run_until_parked();
                    }
                }
                let image = cx.capture_screenshot(window.into()).unwrap();
                if panel == "export" && export_view.as_deref() == Some("rendering") {
                    assert!(image.pixels().any(|pixel|
                        u16::from(pixel[0]) > 215 && pixel[1] > 145
                            && pixel[1] < 205 && pixel[2] > 70 && pixel[2] < 145
                    ), "the synthetic preview should paint its amber disc");
                }
                let picture = if panel == "export" {
                    export_view.as_deref().unwrap_or(panel)
                } else { panel };
                image.save(format!("{dir}/{picture}-{name}.png")).unwrap();
                if panel == "export" {
                    crate::media_thumbs::forget(preview_key);
                }
                if panel == "inspector" {
                    window
                        .update(&mut cx, |frame, _, cx| {
                            frame
                                .view
                                .update(cx, |view, cx| view.review_scroll_to(f32::MAX, cx));
                        })
                        .unwrap();
                    cx.run_until_parked();
                    cx.capture_screenshot(window.into())
                        .unwrap()
                        .save(format!("{dir}/{panel}-{name}-bottom.png"))
                        .unwrap();
                }
                drop(r);
            });
        }
    }
}

/// Headless before and after pictures from clicking a Tracks header's chevron.
#[test]
#[ignore = "writes PNGs; set PM_PANEL_CAPTURE"]
fn capture_track_disclosure() {
    use crate::timeline_tests::{Recorder, Recording};
    use gpui::{Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, point};
    use pm_panel_timeline::{RowId, TimelineClient, tracks, v4};
    use serde_json::json;
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    let dir = std::env::var("PM_PANEL_CAPTURE")
        .unwrap_or_else(|_| "/tmp/pm-shots/track-disclosure".into());
    std::fs::create_dir_all(&dir).unwrap();
    let clip = |id: &str, name: &str, from: f64| {
        let animated = |channel: &str, value: i32| json!({
            "v": value,
            "kf": [{"i": format!("{id}-{channel}"), "t": 3, "v": value, "ei": [1, 1], "eo": [0, 0]}],
        });
        json!({
            "id": id, "name": name, "type": "shape", "from": from, "dur": 6, "on": true,
            "p": {
                "anchor.x": animated("ax", 50),
                "anchor.y": animated("ay", 50),
                "opacity": animated("opacity", 60),
                "position.x": animated("x", 320),
                "position.y": animated("y", 180),
                "rotation": animated("rotation", 0),
                "scale.x": animated("sx", 100),
                "scale.y": animated("sy", 100),
                "skew": animated("skew", 0),
            },
            "d": {"w": 100, "h": 100}, "fx": [],
        })
    };
    // Overlapping clips fill V1, V2 and V3. Only the middle clip's rows are revealed.
    let project = json!({
        "id": "integration", "name": "Tracks", "compId": "c", "w": 640, "h": 360, "fps": 30,
        "dur": 10, "bg": "#000000", "assets": {}, "comps": {},
        "layers": [clip("a", "Lower Third", 0.), clip("b", "Middle", 1.), clip("c", "Upper", 2.)],
    });
    let r = rig_project("timeline", &project.to_string());
    let act = |action| {
        let ticket = r.runtime.timeline_action(action).unwrap();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(result) = ticket.try_take() {
                result.unwrap();
                return;
            }
            assert!(Instant::now() < deadline, "timeline setup stalled");
            std::thread::sleep(Duration::from_millis(1));
        }
    };
    act(json!({"type": "mode", "mode": "Tracks"}));
    act(json!({
        "type": "disclosure", "id": v4::row_id(&RowId::Layer("b".into())), "open": true,
    }));
    let client =
        TimelineClient::new(r.kernel.clone(), PROJECT, pm_kernel::Caller::native()).unwrap();
    let layout = tracks::layout(&client.model.index, &client.model.reveal);
    assert_eq!(layout.video, 3);
    let lane = layout.item("b").unwrap().lane;
    let header = json!(["track", "Video", lane]).to_string();
    let disclosure = format!("timeline:disclosure:{header}");
    let property_id = v4::row_id(&RowId::Property {
        layer_id: "b".into(),
        channel: "opacity".into(),
    });
    let property = format!("timeline:row:{property_id}");
    let first = publication(&r);
    let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
    let panel_fill = tokens.colors.panel;
    let runtime = r.runtime.clone();
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(
            gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                .with_automatic_optical_sizing(),
        ),
        Arc::new(()),
        || {
            Some(Box::new(
                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    );
    cx.allow_parking();
    let mut timeline = None;
    let window = cx
        .open_window(size(px(1100.), px(340.)), |window, cx| {
            let natives = Natives::default();
            let outline = natives.outline.clone();
            let native = cx.new(|cx| {
                crate::timeline_surface::TimelineSurface::new(
                    r.kernel.clone(),
                    PROJECT.into(),
                    tokens.clone(),
                    Rc::new(Recording(Rc::new(Recorder::default()))),
                    outline.clone(),
                    Default::default(),
                    cx,
                )
            });
            timeline = Some(native.clone());
            natives
                .views
                .borrow_mut()
                .insert("timeline".into(), native.into());
            let view = cx.new(|cx| SurfaceView::new(runtime, first, tokens, natives, window, cx));
            outline.bind(view.downgrade());
            cx.new(|_| Frame {
                view,
                fill: panel_fill,
                timecode: None,
            })
        })
        .unwrap();
    let timeline = timeline.unwrap();
    let settle = |cx: &mut HeadlessAppContext, open: bool| {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            for next in r.runtime.take_publications() {
                window
                    .update(cx, |frame, _, cx| {
                        frame.view.update(cx, |view, cx| view.publish(next, cx));
                        timeline.update(cx, |timeline, cx| timeline.published(cx));
                    })
                    .unwrap();
            }
            cx.run_until_parked();
            let paint_matches = cx.update(|cx| {
                timeline.read(cx).scene().is_some_and(|scene| {
                    scene.row(&header).is_some() && scene.row(&property_id).is_some() == open
                })
            });
            if paint_matches {
                let shot = cx.capture_screenshot(window.into()).unwrap();
                if cx
                    .debug_bounds(window.into(), &disclosure)
                    .unwrap()
                    .is_some()
                    && cx.debug_bounds(window.into(), &property).unwrap().is_some() == open
                {
                    return shot;
                }
            }
            assert!(Instant::now() < deadline, "track did not reach open={open}");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    settle(&mut cx, true)
        .save(format!("{dir}/track-expanded.png"))
        .unwrap();
    for (open, name) in [(false, "collapsed"), (true, "reopened")] {
        let position = cx
            .debug_bounds(window.into(), &disclosure)
            .unwrap()
            .unwrap()
            .center();
        cx.simulate_event(
            window.into(),
            MouseDownEvent {
                position,
                modifiers: Modifiers::none(),
                button: MouseButton::Left,
                click_count: 1,
                first_mouse: false,
            },
        );
        cx.simulate_event(
            window.into(),
            MouseUpEvent {
                position,
                modifiers: Modifiers::none(),
                button: MouseButton::Left,
                click_count: 1,
            },
        );
        cx.simulate_event(
            window.into(),
            MouseMoveEvent {
                position: point(px(1000.), px(330.)),
                modifiers: Modifiers::none(),
                pressed_button: None,
            },
        );
        settle(&mut cx, open)
            .save(format!("{dir}/track-{name}.png"))
            .unwrap();
    }
}

/// The panel on solid `colors.panel`, as Reduce Transparency shows it.
struct Frame {
    view: gpui::Entity<SurfaceView>,
    fill: u32,
    timecode: Option<(gpui::Pixels, gpui::Pixels, Tokens)>,
}
impl gpui::Render for Frame {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled};
        gpui::div()
            .size_full()
            .bg(gpui::rgb(self.fill))
            .font_family(pm_design::TEXT_FONT)
            .child(self.view.clone())
            .children(self.timecode.as_ref().map(|(x, y, tokens)| {
                gpui::div().absolute().left(*x).top(*y)
                    .h(px(tokens.metrics.ruler)).flex().items_center()
                    .text_color(gpui::rgb(tokens.colors.text))
                    .text_size(px(tokens.typography.timecode))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child("00:00:00:00")
            }))
    }
}

/// The Threads body and its separately hosted header, as in the live panel.
struct ThreadsCaptureFrame {
    view: gpui::Entity<SurfaceView>,
    header: gpui::Entity<crate::v4_view::HeaderView>,
    fill: u32,
    height: f32,
    header_height: f32,
    before: bool,
}
impl gpui::Render for ThreadsCaptureFrame {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::prelude::*;
        let height = self.height - self.header_height;
        let body = if self.before {
            let content = self.view.read(cx).in_body(height);
            gpui::div()
                .id("old-agent-scroll-body")
                .size_full()
                .overflow_y_scroll()
                .child(self.view.clone().cached(content))
                .into_any_element()
        } else {
            crate::builtins::fixed_body_view("agent", &self.view)
        };
        gpui::div()
            .size_full()
            .flex()
            .flex_col()
            .bg(gpui::rgb(self.fill))
            .font_family(pm_design::TEXT_FONT)
            .child(
                gpui::div()
                    .h(px(self.header_height))
                    .child(self.header.clone()),
            )
            .child(gpui::div().h(px(height)).overflow_hidden().child(body))
    }
}

/// Headless pictures of Agent panel states for design review: each
/// `<state>.json` in `PM_AGENT_STATES` holds the panel's own commits (the
/// JS view rendered from a fixture publication), painted here at the
/// popover's size by the real renderer.
/// `PM_AGENT_STATES=… PM_PANEL_CAPTURE=… cargo test -p pm-native-app capture_agent_states -- --ignored`
#[test]
#[ignore = "writes PNGs; set PM_AGENT_STATES and PM_PANEL_CAPTURE"]
fn capture_agent_states() {
    let timeline = std::env::var_os("PM_COMP_TAB_STATES").is_some();
    let Ok(states) = std::env::var("PM_AGENT_STATES") else {
        return;
    };
    let dir = std::env::var("PM_PANEL_CAPTURE").unwrap_or_else(|_| "/tmp/pm-shots/panels".into());
    std::fs::create_dir_all(&dir).unwrap();
    if let Ok(path) = std::env::var("PM_AGENT_CAPTURE_THUMBNAIL") {
        crate::media_thumbs::register("agent-capture", path.into());
    }
    let panel = std::env::var("PM_CAPTURE_STATE_PANEL").or_else(|_| std::env::var("PM_CAPTURE_PANEL")).unwrap_or_else(|_| "agent".into());
    let r = rig(&panel, 1);
    let mut files: Vec<_> = std::fs::read_dir(&states)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for file in files {
        let commits: Vec<pm_surface::Commit> =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let mut surface = pm_surface::Surface::default();
        for commit in commits {
            surface.apply(commit).unwrap();
        }
        let surface = Arc::new(surface);
        let name = file.file_stem().unwrap().to_string_lossy().to_string();
        for (scheme_name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            objc2::rc::autoreleasepool(|_| {
                let tokens = Tokens::new(scheme, Accessibility::default());
                let (w, h) = if panel == "settings" {
                    (
                        (pm_shell::settings_window::MIN_SIZE.0
                            - pm_shell::settings_window::SIDEBAR_WIDTH) as f32,
                        pm_shell::settings_window::MIN_SIZE.1 as f32,
                    )
                } else if timeline {
                    (900., 200.)
                } else if panel == "layer-effects" {
                    (320., 200.)
                } else if panel == "easing" {
                    (320., 1500.)
                } else {
                    (tokens.metrics.agent_width, tokens.metrics.agent_height)
                };
                let first = pm_builtins::Publication {
                    panel: panel.clone(),
                    generation: 1,
                    surface: surface.clone(),
                };
                let runtime = r.runtime.clone();
                let mut cx = HeadlessAppContext::with_platform(
                    Arc::new(
                        gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                            .with_automatic_optical_sizing(),
                    ),
                    Arc::new(()),
                    || {
                        Some(Box::new(
                            gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                        ))
                    },
                );
                let fill = if panel == "settings" {
                    tokens.colors.panel_secondary
                } else {
                    tokens.colors.floating
                };
                let window = cx
                    .open_window(size(px(w), px(h)), |window, cx| {
                        let view = cx.new(|cx| {
                            let mut view = SurfaceView::new(
                                runtime,
                                first,
                                tokens,
                                Natives::default(),
                                window,
                                cx,
                            );
                            if timeline {
                                view.capture_composition_tab_hover(name.contains("-hover"));
                            }
                            view
                        });
                        cx.new(|_| Frame { view, fill, timecode: None })
                    })
                    .unwrap();
                for _ in 0..30 {
                    window.update(&mut cx, |_, window, _| window.refresh()).unwrap();
                    cx.run_until_parked();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                if std::env::var_os("PM_AGENT_CAPTURE_THUMBNAIL").is_some()
                    && surface.nodes().values().any(|n| n.props.get("resource").and_then(serde_json::Value::as_str) == Some("agent-capture"))
                {
                    let pictures = window.update(&mut cx, |_, window, _| window.rendered_primitive_counts().3).unwrap();
                    assert!(pictures > 0, "{name}/{scheme_name}: attachment image was not painted");
                }
                let request = file.parent().unwrap().join("requests").join(file.file_name().unwrap());
                if request.is_file() {
                    let input: serde_json::Value = serde_json::from_slice(&std::fs::read(request).unwrap()).unwrap();
                    if !input["picker"].is_null() {
                        let id = input["anchor"].as_u64().unwrap();
                        let frame = window.update(&mut cx, |frame, _, cx| frame.view.read(cx).anchor_bounds(id)).unwrap()
                            .expect("composer picker anchored to a measured control");
                        let rect = pm_shell::layout::Rect {
                            x: f32::from(frame.origin.x), y: f32::from(frame.origin.y),
                            w: f32::from(frame.size.width), h: f32::from(frame.size.height),
                        };
                        std::fs::write(format!("{dir}/{panel}-{name}-{scheme_name}-anchor.json"), serde_json::to_vec(&rect).unwrap()).unwrap();
                    }
                }
                let image = cx.capture_screenshot(window.into()).unwrap();
                image
                    .save(if timeline {
                        format!("{dir}/{name}-{scheme_name}.png")
                    } else {
                        format!("{dir}/{panel}-{name}-{scheme_name}.png")
                    })
                    .unwrap();
            });
        }
    }
}

// G4: snapshots of the real ColorPanel, before and after, painted headlessly.
#[test]
#[ignore = "writes PNGs; set PM_COLOUR_STATES and PM_PANEL_CAPTURE"]
fn capture_colour_stack_states() {
    let Ok(states) = std::env::var("PM_COLOUR_STATES") else {
        return;
    };
    let dir = std::env::var("PM_PANEL_CAPTURE").unwrap_or_else(|_| "/tmp/pm-shots/panels".into());
    std::fs::create_dir_all(&dir).unwrap();
    let r = rig("agent", 1);
    let mut files: Vec<_> = std::fs::read_dir(&states)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    for file in files {
        let commits: Vec<pm_surface::Commit> =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let mut surface = pm_surface::Surface::default();
        for commit in commits {
            surface.apply(commit).unwrap();
        }
        let surface = Arc::new(surface);
        let name = file.file_stem().unwrap().to_string_lossy().to_string();
        for (height_name, h) in [("full", 1600.), ("bottom", 620.)] {
            for (scheme_name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
                objc2::rc::autoreleasepool(|_| {
                    let tokens = Tokens::new(scheme, Accessibility::default());
                    let w = 360.;
                    let first = pm_builtins::Publication {
                        panel: "agent".into(),
                        generation: 1,
                        surface: surface.clone(),
                    };
                    let runtime = r.runtime.clone();
                    let mut cx = HeadlessAppContext::with_platform(
                        Arc::new(
                            gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                                .with_automatic_optical_sizing(),
                        ),
                        Arc::new(()),
                        || {
                            Some(Box::new(
                                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                            ))
                        },
                    );
                    let fill = tokens.colors.panel;
                    let window = cx
                        .open_window(size(px(w), px(h)), |window, cx| {
                            let view = cx.new(|cx| {
                                let mut view = SurfaceView::new(
                                    runtime,
                                    first,
                                    tokens,
                                    Natives::default(),
                                    window,
                                    cx,
                                );
                                view.grows = true;
                                view
                            });
                            cx.new(|_| ColourCaptureFrame {
                                view,
                                fill,
                                height: h,
                                scroll: gpui::ScrollHandle::new(),
                            })
                        })
                        .unwrap();
                    for _ in 0..20 {
                        cx.run_until_parked();
                        std::thread::sleep(std::time::Duration::from_millis(20));
                    }
                    // Match the production side-panel body: it measures natural
                    // content before scrolling. Settle layout/culling after moving.
                    for _ in 0..4 {
                        let _ = cx.capture_screenshot(window.into()).unwrap();
                        window
                            .update(&mut cx, |frame, window, cx| {
                                if height_name == "bottom" {
                                    frame.scroll.set_offset(gpui::point(
                                        px(0.),
                                        -frame.scroll.max_offset().y,
                                    ));
                                    frame
                                        .view
                                        .update(cx, |v, cx| v.review_scroll_to(f32::MAX, cx));
                                }
                                window.refresh();
                                cx.notify();
                            })
                            .unwrap();
                        cx.run_until_parked();
                    }
                    let image = cx.capture_screenshot(window.into()).unwrap();
                    if name == "after" {
                        let bounds = cx
                            .debug_bounds(window.into(), "color:add-row")
                            .unwrap()
                            .expect("add row is laid out");
                        assert!(
                            bounds.origin.y >= px(0.) && bounds.bottom() <= px(h + 0.5),
                            "add row outside {height_name} viewport: {bounds:?}"
                        );
                    }
                    image
                        .save(format!(
                            "{dir}/colour-{name}-{height_name}-{scheme_name}.png"
                        ))
                        .unwrap();
                });
            }
        }
    }
}

/// The same grow-to-content wrapper used by a side panel in the dock. A
/// bare fixed-height SurfaceView compresses/culls its children in captures.
struct ColourCaptureFrame {
    view: gpui::Entity<SurfaceView>,
    fill: u32,
    height: f32,
    scroll: gpui::ScrollHandle,
}
impl gpui::Render for ColourCaptureFrame {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::prelude::*;
        let content = self.view.read(cx).in_body(self.height);
        gpui::div()
            .size_full()
            .bg(gpui::rgb(self.fill))
            .font_family(pm_design::TEXT_FONT)
            .child(
                gpui::div()
                    .id("colour-capture-body")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(self.view.clone().cached(content)),
            )
    }
}

#[test]
#[ignore = "writes synthetic before/after PNGs; set PM_PANEL_CAPTURE"]
fn capture_keyframe_reveal_from_properties() {
    use crate::timeline_surface::TimelineSurface;
    use crate::timeline_tests::{Recorder, Recording};
    use std::rc::Rc;
    let dir = std::env::var("PM_PANEL_CAPTURE").expect("capture destination");
    std::fs::create_dir_all(&dir).unwrap();
    let mut r = rig("inspector", 4);
    r.kernel
        .call(
            PROJECT,
            pm_kernel::Request::new(
                pm_kernel::Caller::native(),
                pm_kernel::SelectionCall::Layers {
                    ids: vec!["l0".into()],
                    add: false,
                },
            ),
        )
        .wait()
        .unwrap()
        .unwrap();
    let mount = Arc::get_mut(&mut r.runtime)
        .unwrap()
        .mount("timeline")
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while mount.try_take().is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    for (_, ticket) in r.runtime.publish() {
        let ticket = ticket.unwrap();
        while ticket.try_take().is_none() {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
    let first = r
        .runtime
        .take_publications()
        .into_iter()
        .find(|p| p.panel == "timeline");
    // Take the timeline's current accepted publication even when a later
    // Properties publication was emitted in the same batch.
    let first = first.expect("timeline publication");
    let scheme_name = std::env::var("PM_NATIVE_APPEARANCE").unwrap_or_else(|_| "dark".into());
    let scheme = if scheme_name == "light" { Scheme::Light } else { Scheme::Dark };
    let tokens = Tokens::new(scheme, Accessibility::default());
    let mut cx = HeadlessAppContext::with_platform(
        Arc::new(gpui_parley::ParleyTextSystem::new(
            gpui_parley::SystemFonts::Load,
        )),
        Arc::new(()),
        || {
            Some(Box::new(
                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
            ))
        },
    );
    let natives = Natives::default();
    let surface = cx.new(|cx| {
        TimelineSurface::new(
            r.kernel.clone(),
            PROJECT.into(),
            tokens.clone(),
            Rc::new(Recording(Rc::new(Recorder::default()))),
            natives.outline.clone(),
            Default::default(),
            cx,
        )
    });
    natives
        .views
        .borrow_mut()
        .insert("timeline".into(), surface.clone().into());
    let outline = natives.outline.clone();
    let window = cx
        .open_window(size(px(1100.), px(360.)), |window, cx| {
            cx.new(|cx| SurfaceView::new(r.runtime.clone(), first, tokens, natives, window, cx))
        })
        .unwrap();
    let view = window.entity(&cx).unwrap();
    outline.bind(view.downgrade());
    for phase in ["before", "after"] {
        if phase == "after" {
            let ticket = r
                .runtime
                .panel_command(
                    "inspector",
                    "panel.inspector",
                    &serde_json::json!({"type":"key","row":"opacity"}),
                )
                .unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                for request in r.runtime.take_platform_requests() {
                    assert_eq!(request.operation, "command");
                    assert_eq!(request.input["name"], "timeline.focusGraph");
                    surface.update(&mut cx, |t, cx| t.focus_graph("l0".into(), "opacity".into(), cx));
                    r.runtime.platform_reply(request.id, Ok(serde_json::Value::Null));
                }
                if let Some(result) = ticket.try_take() { result.unwrap(); break; }
                cx.run_until_parked();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            surface.update(&mut cx, |t, cx| t.published(cx));
        }
        for (_, ticket) in r.runtime.publish() {
            let ticket = ticket.unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            while ticket.try_take().is_none() {
                cx.run_until_parked();
                assert!(std::time::Instant::now() < deadline);
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
        for p in r.runtime.take_publications() {
            if p.panel == "timeline" {
                view.update(&mut cx, |v, cx| v.publish(p, cx));
            }
        }
        for _ in 0..30 {
            cx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let has_row = surface.read_with(&cx, |s, _| s.model().rows().iter().any(|r| matches!(&r.id,
            pm_panel_timeline::RowId::Property { layer_id, channel } if layer_id == "l0" && channel == "opacity")));
        assert_eq!(has_row, phase == "after");
        cx.capture_screenshot(window.into())
            .unwrap()
            .save(format!("{dir}/keyframe-{phase}-{scheme_name}.png"))
            .unwrap();
    }
}

/// Replays the .19 and updated Agent surfaces in their native fixed and
/// growing body hosts. The updated surface's compact commit is applied only
/// when the renderer reports the actual viewport crossing the threshold.
#[test]
#[ignore = "writes PNGs; set PM_THREADS_CAPTURE_STATES and PM_PANEL_CAPTURE"]
fn capture_threads_layout() {
    #[derive(serde::Deserialize)]
    struct Fixture {
        initial: Vec<pm_surface::Commit>,
        compact: Vec<pm_surface::Commit>,
    }
    let states = std::env::var("PM_THREADS_CAPTURE_STATES").unwrap();
    let dir = std::env::var("PM_PANEL_CAPTURE").unwrap();
    let before = std::env::var("PM_THREADS_CAPTURE_PHASE").unwrap() == "before";
    std::fs::create_dir_all(&dir).unwrap();
    // The panel publications are synthetic. No Agent account or profile is needed.
    let rig = rig("viewer", 1);
    let mut failures = Vec::new();
    for hosting in ["docked", "floating", "popover"] {
        let fixture: Fixture = serde_json::from_reader(
            std::fs::File::open(format!("{states}/{hosting}.json")).unwrap(),
        )
        .unwrap();
        let mut initial = pm_surface::Surface::default();
        for commit in fixture.initial {
            initial.apply(commit).unwrap();
        }
        let mut compact = initial.clone();
        for commit in fixture.compact {
            compact.apply(commit).unwrap();
        }
        let initial = Arc::new(initial);
        let compact = Arc::new(compact);
        let title_selector = initial
            .nodes()
            .values()
            .find(|node| node.text == "Make your next move")
            .map(|node| format!("n{}", node.parent.unwrap_or(node.id)))
            .unwrap();
        let suggestion_selector = initial
            .nodes()
            .values()
            .find(|node| {
                node.props.get("label").and_then(serde_json::Value::as_str)
                    == Some("Find useful footage")
            })
            .map(|node| format!("n{}", node.id))
            .unwrap();
        for (theme, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            let tokens = Tokens::new(scheme, Accessibility::default());
            let minimum = tokens.agent_panel_minimum_height();
            let heights = match (hosting, before) {
                ("docked", false) => {
                    vec![minimum, minimum + 20., 200., 220., 270., 360., 640., 900.]
                }
                // .19 allowed slots shorter than the composer's new minimum.
                ("docked", true) => vec![120., 220., 360., 640.],
                ("floating", _) => vec![360.],
                _ if before => vec![],
                _ => vec![minimum, 220., 900.],
            };
            for h in heights {
                objc2::rc::autoreleasepool(|_| {
                    let mut cx = HeadlessAppContext::with_platform(
                        Arc::new(
                            gpui_parley::ParleyTextSystem::new(gpui_parley::SystemFonts::Load)
                                .with_automatic_optical_sizing(),
                        ),
                        Arc::new(()),
                        || {
                            Some(Box::new(
                                gpui_apple::metal_renderer::MetalHeadlessRenderer::new(),
                            ))
                        },
                    );
                    let w = if hosting == "docked" {
                        320.
                    } else {
                        tokens.metrics.agent_width
                    };
                    let header_height = tokens.metrics.header;
                    let fill = if hosting == "docked" {
                        tokens.colors.panel
                    } else {
                        tokens.colors.floating
                    };
                    let publication = |surface| pm_builtins::Publication {
                        panel: "agent".into(),
                        generation: 1,
                        surface,
                    };
                    let window = cx
                        .open_window(size(px(w), px(h)), |window, cx| {
                            let view = cx.new(|cx| {
                                let mut view = SurfaceView::new(
                                    rig.runtime.clone(),
                                    publication(initial.clone()),
                                    tokens.clone(),
                                    Natives::default(),
                                    window,
                                    cx,
                                );
                                view.grows = before;
                                view
                            });
                            let header =
                                cx.new(|cx| crate::v4_view::HeaderView::new(view.clone(), cx));
                            cx.new(|_| ThreadsCaptureFrame {
                                view,
                                header,
                                fill,
                                height: h,
                                header_height,
                                before,
                            })
                        })
                        .unwrap();
                    let view = window
                        .update(&mut cx, |frame, _, _| frame.view.clone())
                        .unwrap();
                    let window = gpui::AnyWindowHandle::from(window);
                    let draw = |cx: &mut HeadlessAppContext| {
                        cx.update_window(window, |_, window, _| window.refresh())
                            .unwrap();
                        let screenshot = cx.capture_screenshot(window).unwrap();
                        cx.run_until_parked();
                        screenshot
                    };
                    let mut screenshot = draw(&mut cx);
                    // In .19 the growing body's first pass feeds its next height.
                    if before {
                        let _ = draw(&mut cx);
                        screenshot = draw(&mut cx);
                    }
                    let compact_signal = cx.update(|cx| {
                        let view = view.read(cx);
                        view.log.iter().rev().find_map(|(_, name, detail)| {
                            (name == "scroll").then_some(())?;
                            detail.value.as_ref()?["compact"].as_bool()
                        })
                    });
                    if compact_signal == Some(true) {
                        cx.update(|cx| {
                            view.update(cx, |view, cx| {
                                view.publish(publication(compact.clone()), cx)
                            })
                        });
                        cx.run_until_parked();
                        screenshot = draw(&mut cx);
                    }

                    let tag = format!("threads-{hosting}-{}-{theme}", h.round());
                    let mut bounds = |id: &str| cx.debug_bounds(window, id).unwrap();
                    let header = bounds("agent-header").expect("separate header");
                    let scroll = bounds("agent-conversation");
                    let composer = bounds("agent-composer");
                    let pinned = composer.is_some_and(|composer| {
                        composer.top() >= header.bottom() - px(0.5)
                            && (f32::from(composer.bottom()) - (h - 8.)).abs() <= 0.5
                            && ["agent-field", "agent-model", "agent-effort", "agent-access"]
                                .into_iter()
                                .all(|id| {
                                    bounds(id).is_some_and(|part| {
                                        part.top() >= composer.top()
                                            && part.bottom() <= composer.bottom()
                                            && part.left() >= composer.left()
                                            && part.right() <= composer.right()
                                    })
                                })
                            && scroll
                                .is_none_or(|scroll| scroll.bottom() <= composer.top() + px(0.5))
                    });
                    if !pinned {
                        failures.push(format!("{tag}: composer or picker clipped: {composer:?}"));
                    }
                    if let Some(scroll) = scroll.filter(|scroll| scroll.size.height > px(36.)) {
                        let title =
                            bounds("agent-welcome-title").or_else(|| bounds(&title_selector));
                        let top = title.map(|title| f32::from(title.top() - header.bottom()));
                        if top.is_none_or(|top| (top - 16.).abs() > 0.5) {
                            failures.push(format!("{tag}: welcome inset {top:?}, expected 16 pt"));
                        }
                        if !before {
                            let description = bounds("agent-welcome-description");
                            let should_compact = scroll.size.height < px(120.);
                            if compact_signal != Some(should_compact)
                                || should_compact == description.is_some()
                            {
                                failures.push(format!("{tag}: incorrect compact layout for {scroll:?}, event={compact_signal:?}"));
                            }
                            let suggestions =
                                bounds("agent-suggestions").expect("suggestion list laid out");
                            let mut last = title.expect("welcome title laid out");
                            for next in description.into_iter().chain([suggestions]) {
                                let gap = f32::from(next.top() - last.bottom());
                                if !(-0.5..=16.5).contains(&gap) {
                                    failures
                                        .push(format!("{tag}: {gap} pt between welcome elements"));
                                }
                                last = next;
                            }
                            if hosting == "docked" && h == 220. {
                                if bounds(&suggestion_selector)
                                    .is_none_or(|row| row.bottom() > scroll.bottom() + px(0.5))
                                {
                                    failures.push(format!("{tag}: first suggestion is clipped"));
                                }
                                let (offset, _, max) = cx.update(|cx| {
                                    let view = view.read(cx);
                                    view.test_scroll_state(
                                        view.prepared.semantics["agent-conversation"],
                                    )
                                    .unwrap()
                                });
                                if offset.y != px(0.) || max.y <= px(0.) {
                                    failures.push(format!("{tag}: suggestions have no scroll range from the top: {offset:?}, {max:?}"));
                                }
                                cx.update(|cx| {
                                    view.update(cx, |view, cx| {
                                        view.review_scroll_to(f32::from(max.y), cx)
                                    })
                                });
                                let _ = draw(&mut cx);
                                let scrolled = cx
                                    .debug_bounds(window, "agent-suggestions")
                                    .unwrap()
                                    .unwrap();
                                if scrolled.bottom() > scroll.bottom() + px(0.5) {
                                    failures.push(format!(
                                        "{tag}: bottom of suggestions cannot be reached"
                                    ));
                                }
                                if cx.debug_bounds(window, "agent-composer").unwrap() != composer {
                                    failures.push(format!("{tag}: composer moved while scrolling"));
                                }
                            }
                        }
                    }
                    eprintln!(
                        "{tag}: header={header:?}, scroll={scroll:?}, composer={composer:?}, compact={compact_signal:?}"
                    );
                    if (hosting == "docked" && [220., 360., 640.].contains(&h))
                        || hosting == "floating"
                    {
                        screenshot.save(format!("{dir}/{tag}.png")).unwrap();
                    }
                });
            }
        }
    }
    if before {
        for failure in &failures {
            eprintln!("Expected .19 failure: {failure}");
        }
        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("composer or picker clipped"))
        );
        assert!(
            failures
                .iter()
                .any(|failure| failure.contains("welcome inset"))
        );
    } else {
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }
}
