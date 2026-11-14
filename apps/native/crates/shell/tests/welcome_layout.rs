#![cfg(target_os = "macos")]
//! The Welcome window laid out with real fonts (Parley) and drawn by the
//! headless Metal renderer: long lines wrap inside their cards and a long list
//! scrolls inside the window, the overflow fixed on 2026-11-13 must not return.
//! `PM_WELCOME_CAPTURE=<dir>` also writes the picture.
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    welcome::{Button, Frame, Row, Severity, StepRow},
    welcome_view::WelcomeView,
};
use std::{rc::Rc, sync::Arc};

fn summary() -> Frame {
    let long = "displacement-origin-guide: It was copied but couldn’t load: requires a stage selector that the viewer no longer has, so it waits until it is updated for this version of Powermove.";
    let mut rows = vec![Row {
        symbol: "exclamationmark.triangle.fill",
        severity: Severity::Warning,
        text: long.into(),
    }];
    rows.extend((1..45).map(|i| Row {
        symbol: "checkmark.circle.fill",
        severity: Severity::Success,
        text: format!("extension-{i}: New in the original app. It loads."),
    }));
    let count = |title: &str, value: &str| StepRow {
        title: title.into(),
        value: value.into(),
        progress: None,
    };
    Frame {
        surface: "N006".into(),
        eyebrow: Some("Extensions From the Original App".into()),
        title: "Your Extensions Are In".into(),
        line: "43 brought in, 1 already in Powermove and 1 couldn’t load. Your original app stays unchanged.".into(),
        steps: vec![
            count("Brought in", "43"),
            count("Already in Powermove", "1"),
            count("Couldn’t load", "1"),
            count("Skipped", "1"),
        ],
        rows,
        buttons: vec![Button::new("open_native", "Open Powermove")],
        default: Some("open_native".into()),
        ..Frame::default()
    }
}

#[test]
fn a_long_list_wraps_and_scrolls_inside_the_window() {
    objc2::rc::autoreleasepool(|_| {
        for (name, scheme) in [("dark", Scheme::Dark), ("light", Scheme::Light)] {
            let tokens = Tokens::new(scheme, Accessibility::default());
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
            let window_size = size(px(m.welcome_width), px(m.welcome_height));
            let window = cx
                .open_window(window_size, |_, cx| {
                    cx.new(|cx| {
                        let mut view =
                            WelcomeView::new(tokens.clone(), summary(), Rc::new(|_, _| {}), cx);
                        view.clock = Some(1.);
                        view
                    })
                })
                .unwrap();
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).unwrap();
            if let Ok(dir) = std::env::var("PM_WELCOME_CAPTURE") {
                std::fs::create_dir_all(&dir).unwrap();
                image
                    .save(format!("{dir}/welcome-long-list-{name}.png"))
                    .unwrap();
            }
            let mut bounds = |selector: &str| {
                cx.debug_bounds(window.into(), selector)
                    .unwrap()
                    .unwrap_or_else(|| panic!("{selector} is not drawn"))
            };
            let list = bounds("welcome-rows");
            let first = bounds("welcome-row-0");
            let short = bounds("welcome-row-1");
            let button = bounds("welcome-open_native");
            let title = bounds("welcome-title");
            assert!(title.top() >= px(0.), "{title:?}");
            assert!(
                button.bottom() <= window_size.height,
                "{button:?} is off screen"
            );
            assert!(
                list.bottom() <= button.top(),
                "the list {list:?} covers {button:?}"
            );
            assert!(list.top() > title.bottom(), "the list rides over the title");
            // Wrapped, not clipped: inside the card, and taller than a one-line row.
            assert!(
                first.left() >= list.left() && first.right() <= list.right(),
                "{first:?} vs {list:?}"
            );
            assert!(
                first.size.height > short.size.height,
                "{first:?} didn't wrap"
            );
            // 45 rows don't fit: the list is cut to the room left, and scrolls.
            assert!(
                list.size.height < short.size.height * 45.,
                "{list:?} grew to fit"
            );
        }
    });
}
