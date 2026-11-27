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
