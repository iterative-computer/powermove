use gpui::{prelude::*, *};
use std::sync::Arc;
pub(crate) fn capture<V: Render + 'static>(
    name: &str,
    make: impl FnOnce(&mut Window, &mut App) -> Entity<V>,
) {
    let Ok(dir) = std::env::var("PM_UTILITY_CAPTURE") else {
        return;
    };
    let scheme = std::env::var("PM_NATIVE_APPEARANCE").unwrap();
    std::fs::create_dir_all(&dir).unwrap();
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
    let t = pm_design::Tokens::new(
        if scheme == "dark" {
            pm_design::Scheme::Dark
        } else {
            pm_design::Scheme::Light
        },
        pm_design::Accessibility::default(),
    );
    let m = t.metrics;
    let (w, h) = if name.starts_with("tasks") {
        (m.palette_width, m.palette_height)
    } else if name.starts_with("store") {
        (m.min_width, m.min_height)
    } else if name == "welcome" {
        (m.welcome_width, m.welcome_height)
    } else {
        (m.settings_width, m.settings_height)
    };
    let window = cx.open_window(size(px(w), px(h)), make).unwrap();
    for _ in 0..20 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.capture_screenshot(window.into())
        .unwrap()
        .save(format!("{dir}/{name}-{scheme}.png"))
        .unwrap();
}

#[::core::prelude::v1::test]
#[ignore = "writes synthetic review captures"]
fn capture_utility_settings_search() {
    let Ok(path) = std::env::var("PM_SETTINGS_CAPTURE_STATE") else {
        return;
    };
    let commits: Vec<pm_surface::Commit> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut surface = pm_surface::Surface::default();
    for commit in commits {
        surface.apply(commit).unwrap();
    }
    let first = pm_builtins::Publication {
        panel: "settings".into(),
        generation: 1,
        surface: Arc::new(surface),
    };
    let rig = crate::v4_view_tests::rig("settings", 1);
    capture("settings-no-results", |window, cx| {
        let tokens = pm_shell::tokens();
        let fill = tokens.colors.panel;
        let view = cx.new(|cx| {
            crate::v4_view::SurfaceView::new(
                rig.runtime.clone(),
                first,
                tokens,
                crate::v4_view::Natives::default(),
                window,
                cx,
            )
        });
        cx.new(|_| SettingsFrame { view, fill })
    });
}
struct SettingsFrame {
    view: Entity<crate::v4_view::SurfaceView>,
    fill: u32,
}
impl Render for SettingsFrame {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .bg(rgb(self.fill))
            .child(self.view.clone())
    }
}
