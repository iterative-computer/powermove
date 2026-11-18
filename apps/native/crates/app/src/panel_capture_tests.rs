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
                    rig_project(panel, &std::fs::read_to_string(path).unwrap())
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
                let h = std::env::var("PM_PANEL_CAPTURE_HEIGHT")
                    .ok()
                    .and_then(|v| v.parse::<f32>().ok())
                    .unwrap_or(h);
                let first = publication(&r);
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
                let image = cx.capture_screenshot(window.into()).unwrap();
                image.save(format!("{dir}/{panel}-{name}.png")).unwrap();
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
    let Ok(states) = std::env::var("PM_AGENT_STATES") else {
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
        for (scheme_name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            objc2::rc::autoreleasepool(|_| {
                let tokens = Tokens::new(scheme, Accessibility::default());
                let (w, h) = (tokens.metrics.agent_width, tokens.metrics.agent_height);
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
                let fill = tokens.colors.floating;
                let window = cx
                    .open_window(size(px(w), px(h)), |window, cx| {
                        let view = cx.new(|cx| {
                            SurfaceView::new(runtime, first, tokens, Natives::default(), window, cx)
                        });
                        cx.new(|_| Frame { view, fill })
                    })
                    .unwrap();
                for _ in 0..20 {
                    cx.run_until_parked();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                let image = cx.capture_screenshot(window.into()).unwrap();
                image
                    .save(format!("{dir}/agent-{name}-{scheme_name}.png"))
                    .unwrap();
            });
        }
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
