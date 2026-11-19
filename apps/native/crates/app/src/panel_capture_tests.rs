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

#[test]
#[ignore = "writes PNGs; set PM_PANEL_CAPTURE"]
fn capture_builtin_panels() {
    let dir = std::env::var("PM_PANEL_CAPTURE").unwrap_or_else(|_| "/tmp/pm-shots/panels".into());
    std::fs::create_dir_all(&dir).unwrap();
    let only = std::env::var("PM_PANEL_CAPTURE_ONLY").ok();
    let export_view = std::env::var("PM_PANEL_CAPTURE_EXPORT_VIEW").ok();
    if let Some(view) = &export_view {
        assert!(["settings", "project", "rendering", "queue", "empty"].contains(&view.as_str()), "unknown export capture view");
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
                    let wait = |ticket: pm_runtime_v8::Ticket<pm_runtime_v8::Evaluation>| {
                        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
                        loop {
                            if let Some(result) = ticket.try_take() { result.unwrap(); break; }
                            assert!(std::time::Instant::now() < deadline, "export capture did not settle");
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                    };
                    if matches!(export_view.as_deref(), Some("settings" | "project")) {
                        // An idle sheet has a pending item and a finished export.
                        wait(r.runtime.observe_render_queue(serde_json::json!([
                            {"id":2,"name":"Logo","format":"prores","path":"/Exports/Logo.mov","status":"Pending","progress":0.,"error":""},
                            {"id":3,"name":"Opening","format":"mp4","path":"/Exports/Opening.mp4","status":"Done","progress":1.,"error":"","finishedAt":1794686520}
                        ])).unwrap());
                    } else if export_view.as_deref() == Some("rendering") {
                        wait(r.runtime.observe_render_queue(serde_json::json!([
                            {"id":1,"name":"Opening","format":"mp4","path":"/Exports/Opening.mp4","status":"Rendering","progress":0.42,"error":""},
                            {"id":2,"name":"Logo","format":"prores","path":"/Exports/Logo.mov","status":"Pending","progress":0.,"error":""},
                            {"id":3,"name":"Credits","format":"mp4","path":"/Exports/Credits.mp4","status":"Pending","progress":0.,"error":""},
                            {"id":4,"name":"Teaser","format":"mp4","path":"/Exports/Teaser.mp4","status":"Pending","progress":0.,"error":""}
                        ])).unwrap());
                    } else if export_view.as_deref() == Some("empty") {
                        wait(r.runtime.observe_render_queue(serde_json::json!([])).unwrap());
                    }
                    wait(r.runtime.observe_export_destination(serde_json::json!({"path":"/Exports/Launch Film.mp4","format":"mp4"})).unwrap());
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
                        wait(r.runtime.observe_export(serde_json::json!({
                            "phase":"rendering","frame":126,"total":300,
                            "elapsed":12.,"eta":17.,"preview":preview_key,
                        })).unwrap());
                    }
                    if std::env::var_os("PM_PANEL_CAPTURE_QUEUE").is_some()
                        || export_view.as_deref().is_some_and(|v|v == "queue" || v == "empty") {
                        wait(r.runtime.show_section("export", "queue").unwrap());
                    } else {
                        wait(r.runtime.show("export").unwrap());
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
                    let target = first.surface.nodes().values().find(|n| n.props["id"] == hovered_job).unwrap().id;
                    let ticket = r.runtime.event("export", first.generation, first.surface.revision(), target, "pointerenter", Default::default()).unwrap();
                    while ticket.try_take().is_none() { std::thread::sleep(std::time::Duration::from_millis(1)); }
                    first = r.runtime.take_publications().pop().unwrap_or(first);
                }
                if panel == "export" && export_view.as_deref() == Some("project") {
                    let target = first.surface.nodes().values().find(|n|
                        n.props["role"] == "radio" && n.props["label"] == "Project"
                    ).unwrap().id;
                    let ticket = r.runtime.event("export", first.generation, first.surface.revision(), target, "click", Default::default()).unwrap();
                    while ticket.try_take().is_none() {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    first = r.runtime.take_publications().pop().unwrap();
                    assert!(first.surface.nodes().values().any(|n| n.props["id"] == "export.project.editable"));
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
                let window = cx
                    .open_window(size(px(w), px(h)), |window, cx| {
                        let view = cx.new(|cx| {
                            SurfaceView::new(runtime, first, tokens, Natives::default(), window, cx)
                        });
                        cx.new(|_| Frame {
                            view,
                            fill: panel_fill,
                        })
                    })
                    .unwrap();
                for _ in 0..40 {
                    cx.run_until_parked();
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                if std::env::var_os("PM_PANEL_CAPTURE_BOTTOM").is_some() {
                    window
                        .update(&mut cx, |frame, _, cx| {
                            frame
                                .view
                                .update(cx, |view, cx| view.review_scroll_to(f32::MAX, cx));
                        })
                        .unwrap();
                    cx.run_until_parked();
                }
                let image = cx.capture_screenshot(window.into()).unwrap();
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

/// The panel on solid `colors.panel`, as Reduce Transparency shows it.
struct Frame {
    view: gpui::Entity<SurfaceView>,
    fill: u32,
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
                        cx.new(|_| Frame { view, fill })
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
