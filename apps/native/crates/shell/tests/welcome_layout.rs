#![cfg(target_os = "macos")]
//! The Welcome window laid out with real fonts (Parley) and drawn by the
//! headless Metal renderer, in its own size: a step's screens keep the title
//! put and the buttons at the bottom, long reasons wrap inside their card,
//! and a list longer than the room scrolls with a fade that says so; nothing
//! runs past the window or under the buttons.
//! `PM_WELCOME_CAPTURE=<dir>` also writes the pictures.
use gpui::{AppContext, HeadlessAppContext, px, size};
use pm_design::{Accessibility, Scheme, Tokens};
use pm_shell::{
    welcome::{Button, Frame, Names, Row, Severity, Stat},
    welcome_view::WelcomeView,
};
use std::{rc::Rc, sync::Arc};

fn note(name: &str, reason: &str, severity: Severity) -> Row {
    Row {
        symbol: if severity == Severity::Warning {
            "exclamationmark.triangle.fill"
        } else {
            "info.circle"
        },
        severity,
        text: name.into(),
        detail: Some(reason.into()),
    }
}
/// Chike's summary (one skip, 45 names) or one with many long exceptions.
fn summary(overflow: bool) -> Frame {
    let rows = if overflow {
        let mut rows = vec![note(
            "Displacement Origin Guide",
            "It was copied but couldn’t load: requires a stage selector that the viewer no longer has, so it waits until it is updated for this version of Powermove.",
            Severity::Warning,
        )];
        rows.extend((1..14).map(|i| {
            note(
                &format!("Folder {i}"),
                "Its folder is empty or incomplete, so there’s nothing to bring in.",
                Severity::Note,
            )
        }));
        rows
    } else {
        vec![note(
            "figma-layer-import",
            "Its folder is empty or incomplete, so there’s nothing to bring in.",
            Severity::Note,
        )]
    };
    let stat = |value: &str, label: &str| Stat {
        value: value.into(),
        label: label.into(),
    };
    Frame {
        surface: "N006".into(),
        top: true,
        eyebrow: Some("Extensions From the Original App".into()),
        title: "Your Extensions Are In".into(),
        line: "One couldn’t load. The reason is below. Your original app stays unchanged.".into(),
        stats: vec![stat("45", "Brought in"), stat("1", "Skipped")],
        rows_title: Some("Not Brought In".into()),
        rows,
        names: Some(Names {
            title: "Brought In".into(),
            names: (1..=45).map(|i| format!("Extension Number {i}")).collect(),
            preview: 6,
        }),
        buttons: vec![Button::new("open_native", "Open Powermove")],
        default: Some("open_native".into()),
        ..Frame::default()
    }
}

#[test]
fn a_summary_breathes_wraps_and_scrolls_inside_the_window() {
    objc2::rc::autoreleasepool(|_| {
        for (name, scheme, overflow) in [
            ("dark", Scheme::Dark, false),
            ("light", Scheme::Light, false),
            ("dark-overflow", Scheme::Dark, true),
        ] {
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
                        let mut view = WelcomeView::new(
                            tokens.clone(),
                            summary(overflow),
                            Rc::new(|_, _| {}),
                            cx,
                        );
                        view.clock = Some(1.);
                        view
                    })
                })
                .unwrap();
            cx.run_until_parked();
            // The fade needs one layout to know there's more: the window asks
            // for the next frame, which the headless harness draws on demand.
            cx.capture_screenshot(window.into()).unwrap();
            cx.update_window(window.into(), |_, window, _| window.refresh())
                .unwrap();
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).unwrap();
            if let Ok(dir) = std::env::var("PM_WELCOME_CAPTURE") {
                std::fs::create_dir_all(&dir).unwrap();
                image
                    .save(format!("{dir}/welcome-summary-{name}.png"))
                    .unwrap();
            }
            let mut bounds = |selector: &str| cx.debug_bounds(window.into(), selector).unwrap();
            let get = |b: Option<gpui::Bounds<gpui::Pixels>>, what: &str| {
                b.unwrap_or_else(|| panic!("{what} is not drawn ({name})"))
            };
            let title = get(bounds("welcome-title"), "title");
            let body = get(bounds("welcome-body"), "body");
            let stats = get(bounds("welcome-stats"), "stats");
            let first = get(bounds("welcome-row-0"), "row 0");
            let button = get(bounds("welcome-open_native"), "button");
            let more = bounds("welcome-more");
            // Air: clear of the title bar, sections apart, buttons on screen.
            assert!(
                title.top() >= px(m.toolbar),
                "{title:?} under the title bar"
            );
            assert!(
                stats.top() - title.bottom() >= px(tokens.space.xxl),
                "{name}"
            );
            assert!(
                body.top() - stats.bottom() >= px(tokens.space.xxl),
                "{name}"
            );
            assert!(
                button.bottom() <= window_size.height - px(m.sheet_inset),
                "{name}"
            );
            assert!(
                body.bottom() <= button.top(),
                "the list {body:?} covers {button:?}"
            );
            assert!(
                first.left() >= body.left() && first.right() <= body.right(),
                "{first:?} vs {body:?}"
            );
            if overflow {
                // Wrapped, not clipped: taller than a one-line reason; and
                // the list that doesn't fit says so.
                let short = get(bounds("welcome-row-1"), "row 1");
                assert!(
                    first.size.height > short.size.height,
                    "{first:?} didn't wrap"
                );
                assert!(more.is_some(), "an overflowing list shows its fade");
            } else {
                // Chike's summary fits whole: nothing cut off, nothing fades.
                assert!(more.is_none(), "{name}: nothing more to see, no fade");
                let names = get(bounds("welcome-names"), "names");
                assert!(
                    names.bottom() <= body.bottom(),
                    "{names:?} cut off at {body:?}"
                );
                assert!(bounds("welcome-names-toggle").is_some(), "Show All 45");
            }
        }
    });
}

#[test]
fn keychain_failure_and_retry_surfaces_render_headlessly() {
    objc2::rc::autoreleasepool(|_| {
        for (name, title, line, retry) in [
            (
                "keychain-missing",
                "Keychain Access Wasn’t Allowed",
                "The original app’s sign-in key wasn’t found in your Keychain. Open the original app and sign in, then try again. Your sign-ins and extension values stay in the original app. You can continue and sign in here.",
                true,
            ),
            (
                "keychain-consent",
                "Keep Your Sign-Ins",
                "Powermove can keep your extension secrets and sign-ins. Continue to allow one macOS Keychain request for Powermove Safe Storage. Your original app stays unchanged.",
                false,
            ),
        ] {
            let tokens = Tokens::new(Scheme::Dark, Accessibility::default());
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
            let frame = Frame {
                surface: "N002".into(),
                title: title.into(),
                line: line.into(),
                buttons: if retry {
                    vec![
                        Button::new("quit", "Quit Powermove").leading(),
                        Button::new("skip_secret_consent", "Continue Without Them"),
                        Button::new("resume_with_secret_consent", "Try Again"),
                    ]
                } else {
                    vec![
                        Button::new("quit", "Quit Powermove").leading(),
                        Button::new("skip_secret_consent", "Not Now"),
                        Button::new("continue", "Continue"),
                    ]
                },
                default: Some(
                    if retry {
                        "skip_secret_consent"
                    } else {
                        "continue"
                    }
                    .into(),
                ),
                ..Frame::default()
            };
            let window = cx
                .open_window(size(px(m.welcome_width), px(m.welcome_height)), |_, cx| {
                    cx.new(|cx| WelcomeView::new(tokens, frame, Rc::new(|_, _| {}), cx))
                })
                .unwrap();
            cx.run_until_parked();
            let image = cx.capture_screenshot(window.into()).unwrap();
            if let Ok(dir) = std::env::var("PM_WELCOME_CAPTURE") {
                std::fs::create_dir_all(&dir).unwrap();
                image.save(format!("{dir}/{name}.png")).unwrap();
            }
            let title = cx
                .debug_bounds(window.into(), "welcome-title")
                .unwrap()
                .unwrap();
            assert!(title.top() >= px(0.));
            let button_id = if retry {
                "welcome-resume_with_secret_consent"
            } else {
                "welcome-continue"
            };
            let button = cx.debug_bounds(window.into(), button_id).unwrap().unwrap();
            assert!(button.bottom() <= px(m.welcome_height));
        }
    });
}
