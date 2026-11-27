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
            let (ground, panel_alpha) = (tokens.ground(), tokens.translucency.panel);
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
            // Source-over of a token colour at an alpha onto a probed pixel.
            let over = |(c, a): (u32, f32), under: [u8; 4]| -> [u8; 3] {
                let c = rgb(c);
                std::array::from_fn(|i| {
                    (c[i] as f32 * a + under[i] as f32 * (1. - a)).round() as u8
                })
            };
            let near = |got: [u8; 4], want: [u8; 3], what: &str| {
                assert!(
                    (0..3).all(|i| got[i].abs_diff(want[i]) <= 1) && got[3] == 255,
                    "{name}: {what} {got:?}, want {want:?}"
                );
            };
            // Probe a gap, a panel interior, a panel edge and the viewer, away
            // from glyphs/AA. Headless has an opaque black backdrop where the
            // app has the desktop blur, so every layer's composite is exact:
            // ground over black, panel material over ground. A panel has no
            // border: its edge is the elevation's hairline in its first half
            // point, with the plain material straight after it and the plain
            // ground straight before.
            let first = metrics.dock_gap as u32 * scale;
            let gap = image.get_pixel(2 * scale, 200 * scale).0;
            let panel = image.get_pixel(40 * scale, 150 * scale).0;
            let edge = image.get_pixel(first, 200 * scale).0;
            near(gap, over(ground, [0, 0, 0, 255]), "ground");
            near(panel, over((expected.panel, panel_alpha), gap), "panel");
            assert_eq!(
                image.get_pixel(first - 1, 200 * scale).0,
                gap,
                "{name}: gap"
            );
            assert_eq!(
                image.get_pixel(first + 2 * scale, 200 * scale).0,
                panel,
                "{name}: no border inside the hairline"
            );
            assert_ne!(edge, panel, "{name}: a hairline edges the panel");
            if translucent {
                // In dark the panel steps lighter than the ground it sits on.
                if scheme == Scheme::Dark {
                    assert!(panel[0] > gap[0], "{name}: panel above ground");
                }
            } else {
                // Reduce Transparency: ground and panel are the solid tokens.
                assert_eq!(&gap[..3], &rgb(expected.window), "{name}: solid ground");
                assert_eq!(&panel[..3], &rgb(expected.panel), "{name}: solid panel");
            }
            if a11y.increase_contrast {
                assert_eq!(&edge[..3], &rgb(expected.separator), "{name}: solid edge");
            }
            assert_eq!(
                &image.get_pixel(500 * scale, 150 * scale).0[..3],
                &rgb(expected.viewer)
            );
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
