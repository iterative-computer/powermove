//! Headless pictures of the built-in panels (language §16 review loop): the
//! real V8 panels and kernel on a synthetic project, painted by GPUI's Metal
//! renderer without a screen. AppKit control slots are absent, as in the host
//! tests. Run on demand:
//! `PM_PANEL_CAPTURE=/tmp/pm-shots/panels cargo test -p pm-native-app capture_builtin_panels -- --ignored`
use crate::v4_view::{Natives, SurfaceView};
use crate::v4_view_tests::{publication, rig};
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
                let r = rig(panel, 8);
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
