#![cfg(target_os = "macos")]
//! Real GPUI element tree + P-HEADLESS Metal renderer. No AppKit windows/screens.
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use std::sync::Arc;
#[test]
fn shell_window_appearance_matrix() {
    objc2::rc::autoreleasepool(|_| {
        let mut outputs = vec![];
        for (name, scheme, a11y) in [
            ("light", Scheme::Light, Accessibility::default()),
            ("dark", Scheme::Dark, Accessibility::default()),
            (
                "reduce-transparency",
                Scheme::Dark,
                Accessibility {
                    reduce_transparency: true,
                    ..Default::default()
                },
            ),
            (
                "increase-contrast",
                Scheme::Dark,
                Accessibility {
                    increase_contrast: true,
                    ..Default::default()
                },
            ),
        ] {
            let tokens = Tokens::new(scheme, a11y);
            let expected = tokens.colors.clone();
            let translucent = tokens.translucent();
            let metrics = tokens.metrics;
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
                .open_window(
                    size(px(metrics.window_width), px(metrics.window_height)),
                    |_, cx| {
                        cx.new(|cx| {
                            let mut v = pm_shell::view::Editor::new(tokens, None, cx);
                            v.layout = pm_shell::layout::Layout::preset("design");
                            v
                        })
                    },
                )
                .unwrap();
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).unwrap();
            let scale = image.width() / metrics.window_width as u32;
            assert!(scale > 0);
            assert_eq!(
                image.dimensions(),
                (
                    metrics.window_width as u32 * scale,
                    metrics.window_height as u32 * scale
                )
            );
            let rgb = |v: u32| [(v >> 16) as u8, (v >> 8) as u8, v as u8];
            // Probe panel and viewer interiors and a panel edge, away from glyphs/AA.
            let panel = image.get_pixel(40 * scale, 150 * scale).0;
            let edge = image.get_pixel(6 * scale, 200 * scale).0;
            if translucent {
                // The panel material lets the window ground (and the desktop
                // blur under it) through, so it never reads as the solid token.
                assert_ne!(&panel[..3], &rgb(expected.panel), "{name}: translucent");
            } else {
                // Reduce Transparency: the panel is exactly the solid token.
                assert_eq!((&panel[..3], panel[3]), (&rgb(expected.panel)[..], 255));
            }
            assert_eq!(
                &image.get_pixel(500 * scale, 150 * scale).0[..3],
                &rgb(expected.viewer)
            );
            if a11y.increase_contrast {
                assert_eq!(&edge[..3], &rgb(expected.separator), "{name}: solid edge");
            }
            std::fs::create_dir_all("/tmp/pm-shots").unwrap();
            image
                .save(format!("/tmp/pm-shots/ptext-shell-headless-{name}.png"))
                .unwrap();
            outputs.push(image);
        }
        // Appearance, Reduce Transparency (solid window and panels) and
        // Increase Contrast (solid separator edges) each change the pixels.
        assert_ne!(outputs[0].as_raw(), outputs[1].as_raw());
        assert_ne!(outputs[1].as_raw(), outputs[2].as_raw());
        assert_ne!(outputs[1].as_raw(), outputs[3].as_raw());
    });
}
