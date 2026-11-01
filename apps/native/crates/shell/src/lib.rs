//! Phase 1 application shell. Kernel and engine are deliberately integration seams.
pub mod keyboard;
pub mod layout;
pub mod menu;
mod native;
pub mod transient;
pub mod view;
use gpui::{prelude::*, *};
use pm_design::{Scheme, Tokens};
use std::rc::Rc;
pub fn run() -> anyhow::Result<()> {
    let device = metal::Device::system_default()
        .ok_or_else(|| anyhow::anyhow!("Metal device unavailable"))?;
    run_with_device(device)
}
struct SharedMetalDevice(metal::Device);
impl Global for SharedMetalDevice {}
pub fn run_with_device(device: metal::Device) -> anyhow::Result<()> {
    launch(device, false)
}
/// Initializes native chrome with activation prohibited and every window hidden.
pub fn smoke() -> anyhow::Result<()> {
    launch(
        metal::Device::system_default().ok_or_else(|| anyhow::anyhow!("Metal unavailable"))?,
        true,
    )
}
fn launch(device: metal::Device, smoke: bool) -> anyhow::Result<()> {
    let platform = Rc::new(gpui_macos::MacPlatform::new_with_metal_device(
        false,
        device.clone(),
    ));
    let app = Application::with_platform(platform);
    let app = if smoke {
        app.with_activation_policy(MacActivationPolicy::Prohibited)
    } else {
        app
    };
    app.with_accessibility_forced().run(move |cx| {
        cx.set_global(SharedMetalDevice(device));
        native::install_menus();
        open_editor_with_visibility(cx, !smoke);
        if smoke {
            cx.quit();
        } else {
            cx.activate(true);
        }
    });
    Ok(())
}
fn tokens() -> Tokens {
    let scheme = if std::env::var("PM_NATIVE_APPEARANCE").as_deref() == Ok("light") {
        Scheme::Light
    } else if std::env::var("PM_NATIVE_APPEARANCE").as_deref() == Ok("dark") {
        Scheme::Dark
    } else {
        use objc2_app_kit::*;
        let app = NSApplication::sharedApplication(objc2::MainThreadMarker::new().unwrap());
        let name = app.effectiveAppearance().name().to_string();
        if name.contains("Dark") {
            Scheme::Dark
        } else {
            Scheme::Light
        }
    };
    Tokens::new(scheme, native::settings())
}
pub(crate) fn open_editor(cx: &mut App) {
    open_editor_with_visibility(cx, true);
}
fn open_editor_with_visibility(cx: &mut App, visible: bool) {
    let t = tokens();
    let m = t.metrics;
    let result = cx.open_window(
        WindowOptions::new()
            .show(visible)
            .focus(visible)
            .macos_window_background(MacosWindowBackground::Transparent)
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(m.toolbar), px(m.toolbar)),
                size(px(m.window_width), px(m.window_height)),
            )))),
        move |window, cx| {
            window.set_window_title("Untitled — Powermove Native");
            let device = cx.global::<SharedMetalDevice>().0.clone();
            let (host, rx) = native::Host::install(window, &device, t.clone());
            if !visible {
                host.borrow().smoke_validate();
            }
            let editor = cx.new(|cx| view::Editor::new(t, Some(host), cx));
            editor.update(cx, |_, cx| {
                cx.observe_window_appearance(window, |v, w, cx| v.follow_appearance(w, cx))
                    .detach();
            });
            let weak = editor.downgrade();
            let handle = window.window_handle();
            cx.spawn(async move |cx| {
                while let Ok(cmd) = rx.recv().await {
                    let r = handle.update(cx, |_, window, cx| {
                        weak.update(cx, |v, cx| v.command(cmd, window, cx))
                    });
                    if r.is_err() {
                        break;
                    }
                }
            })
            .detach();
            editor
        },
    );
    if let Err(e) = result {
        eprintln!("Could not open editor: {e}");
    }
}
struct Settings {
    tokens: Tokens,
    selected: usize,
    focus: FocusHandle,
}
const SECTIONS: [&str; 5] = [
    "Project",
    "General",
    "Accounts",
    "Transcription",
    "Extensions",
];
impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.tokens;
        let mut sidebar = div()
            .flex()
            .flex_col()
            .w(px(t.metrics.settings_sidebar))
            .p(px(t.metrics.inset))
            .gap(px(t.metrics.gap));
        for (i, s) in SECTIONS.iter().enumerate() {
            sidebar = sidebar.child(
                div()
                    .id(i)
                    .rounded(px(t.metrics.control_radius))
                    .p(px(t.metrics.gap))
                    .bg(rgb(if i == self.selected {
                        t.colors.selected_fill
                    } else {
                        t.colors.panel_secondary
                    }))
                    .child(*s)
                    .on_click(cx.listener(move |v, _, _, cx| {
                        v.selected = i;
                        cx.notify();
                    })),
            );
        }
        div()
            .size_full()
            .flex()
            .bg(rgb(t.colors.panel))
            .text_color(rgb(t.colors.text))
            .font_family(pm_design::TEXT_FONT)
            .text_size(px(t.typography.body))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|v, e: &KeyDownEvent, w, cx| {
                match e.keystroke.key.as_str() {
                    "up" => v.selected = v.selected.saturating_sub(1),
                    "down" => v.selected = (v.selected + 1).min(SECTIONS.len() - 1),
                    "home" => v.selected = 0,
                    "end" => v.selected = SECTIONS.len() - 1,
                    "escape" => w.remove_window(),
                    _ => {}
                }
                cx.notify();
            }))
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .p(px(t.metrics.inset))
                    .text_size(px(t.typography.display))
                    .child(SECTIONS[self.selected]),
            )
    }
}
pub(crate) fn open_settings(cx: &mut App) {
    let existing = cx
        .windows()
        .into_iter()
        .find(|w| w.downcast::<Settings>().is_some());
    if let Some(w) = existing {
        let _ = w.update(cx, |_, w, _| w.activate());
        return;
    }
    let t = tokens();
    let m = t.metrics;
    let _ = cx.open_window(
        WindowOptions::new()
            .titlebar(Some(TitlebarOptions {
                title: Some("Settings".into()),
                ..Default::default()
            }))
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(m.toolbar), px(m.toolbar)),
                size(px(m.settings_width), px(m.settings_height)),
            )))),
        |w, cx| {
            let entity = cx.new(|cx| Settings {
                tokens: t,
                selected: 0,
                focus: cx.focus_handle(),
            });
            let f = entity.read(cx).focus.clone();
            w.focus(&f, cx);
            entity
        },
    );
}
