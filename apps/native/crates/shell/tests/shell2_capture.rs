//! Shell 2 renderer captures: drag-to-dock preview, the docked result and the
//! mockup tab strip, on the real GPUI tree + P-HEADLESS Metal renderer.
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::layout::{DropTarget, Edge, Layout};
use std::sync::Arc;

#[test]
fn drag_to_dock_preview_and_result_render() {
    objc2::rc::autoreleasepool(|_| {
        let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
        let c = tokens.colors.clone();
        let m = tokens.metrics;
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
        let window = cx
            .open_window(size(px(m.window_width), px(m.window_height)), |_, cx| {
                cx.new(|cx| {
                    let mut v = pm_shell::view::Editor::new(tokens, None, cx);
                    v.layout = Layout::preset("design");
                    v
                })
            })
            .unwrap();
        cx.run_until_parked();
        let dir = std::path::Path::new("/tmp/pm-shell2");
        std::fs::create_dir_all(dir).unwrap();
        let before = cx.capture_screenshot(window.into()).unwrap();
        before.save(dir.join("headless-1-default.png")).unwrap();
        let scale = before.width() / m.window_width as u32;
        let rgb = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];

        // Drag Effects to the window's bottom edge, over the timeline.
        let (x, y) = (700., m.window_height - m.status - m.gap - 2.);
        window
            .update(&mut cx, |v, _, cx| {
                v.simulate_drag("fxbrowser", x, y);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        let dragging = cx.capture_screenshot(window.into()).unwrap();
        dragging
            .save(dir.join("headless-2-drag-preview.png"))
            .unwrap();
        // The preview tints the bottom dock with the accent; the timeline body was solid panel.
        let probe = dragging
            .get_pixel(400 * scale, (m.window_height as u32 - 120) * scale)
            .0;
        assert_ne!(&probe[..3], &rgb(c.panel), "preview must tint the target");
        assert!(probe[0] > probe[2], "accent (orange) tint, got {probe:?}");

        let applied = window
            .update(&mut cx, |v, _, cx| {
                cx.notify();
                v.simulate_drop()
            })
            .unwrap();
        assert_eq!(applied, Some(DropTarget::Edge(Edge::Bottom)));
        cx.run_until_parked();
        let after = cx.capture_screenshot(window.into()).unwrap();
        after
            .save(dir.join("headless-3-docked-bottom.png"))
            .unwrap();
        let layout = window.update(&mut cx, |v, _, _| v.layout.clone()).unwrap();
        let bottom = layout.docks.iter().find(|d| d.id == "bottom").unwrap();
        assert_eq!(bottom.groups.len(), 2);
        assert_eq!(bottom.groups[1].tabs, ["fxbrowser"]);
        // The right dock is now a single full-height Properties panel.
        assert_eq!(
            layout
                .docks
                .iter()
                .find(|d| d.id == "right")
                .unwrap()
                .groups
                .len(),
            1
        );
        // Saved layout round trip through the shell command path (no AppKit host).
        window
            .update(&mut cx, |v, _, cx| {
                assert!(v.run_shell_command("layout:save:Bottom effects", cx));
                assert!(v.run_shell_command("layout:reset", cx));
                assert_eq!(v.layout, Layout::preset("design"));
                assert!(v.run_shell_command("layout:apply:Bottom effects", cx));
                assert!(v.run_shell_command("panel:close:timeline", cx));
                assert!(!v.layout.open_panels().contains(&"timeline"));
                assert!(v.run_shell_command("panel:toggle:timeline", cx));
                assert!(v.layout.open_panels().contains(&"timeline"));
                assert!(!v.run_shell_command("unknown:verb", cx));
            })
            .unwrap();
        cx.run_until_parked();
        assert_ne!(before.as_raw(), after.as_raw());
    });
}
