//! macOS application host. Portable shell contracts live in lib.rs.
use crate::{integration, native, session, theme, view};
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
    launch(device, false, None, None, None)
}
/// Initializes native chrome with activation prohibited and every window hidden.
pub fn smoke() -> anyhow::Result<()> {
    launch(
        metal::Device::system_default().ok_or_else(|| anyhow::anyhow!("Metal unavailable"))?,
        true,
        None,
        None,
        None,
    )
}
/// Launch using the engine device and app-owned panel/command implementation.
pub fn run_integrated(device: metal::Device, factory: integration::Factory) -> anyhow::Result<()> {
    launch(device, false, Some(factory), None, None)
}
/// Opens the editor windows. Whoever holds the gate calls it exactly once.
pub type Finish = Rc<dyn Fn(&mut App)>;
pub type Gate = Box<dyn FnOnce(Finish, &mut App)>;
/// Files and `powermove://` links LaunchServices hands the app (Finder, the
/// Dock icon, `open`), as absolute URL strings, in arrival order.
pub type OpenUrls = Rc<dyn Fn(Vec<String>, &mut App)>;
/// As `run_integrated`, with no editor window until `gate` calls its `Finish`.
pub fn run_gated(
    device: metal::Device,
    factory: integration::Factory,
    gate: Gate,
    open_urls: OpenUrls,
) -> anyhow::Result<()> {
    launch(device, false, Some(factory), Some(gate), Some(open_urls))
}
fn launch(
    device: metal::Device,
    smoke: bool,
    factory: Option<integration::Factory>,
    gate: Option<Gate>,
    open_urls: Option<OpenUrls>,
) -> anyhow::Result<()> {
    native::install_identity();
    let theme_error = theme::load_from_env();
    if let Some(e) = &theme_error {
        eprintln!("{e}");
    }
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
    // A cold start delivers its documents before `run`'s callback: listen
    // first, hand them over once the app exists.
    let (urls_tx, urls_rx) = async_channel::unbounded::<Vec<String>>();
    app.on_open_urls(move |urls| {
        let _ = urls_tx.try_send(urls);
    });
    app.with_accessibility_forced().run(move |cx| {
        cx.set_global(SharedMetalDevice(device));
        if let Some(open_urls) = open_urls {
            cx.spawn(async move |cx| {
                while let Ok(urls) = urls_rx.recv().await {
                    cx.update(|cx| open_urls(urls, cx));
                }
            })
            .detach();
        }
        if let Some(factory) = factory {
            cx.set_global(integration::AppIntegration(factory));
        }
        native::install_menus();
        native::install_accent();
        // Dev/test appearance override: menus and sheets follow it too.
        if std::env::var_os("PM_NATIVE_APPEARANCE").is_some() {
            native::app_appearance(&tokens());
        }
        if smoke {
            open_editor_window(cx, false, None, None);
        } else if let Some(gate) = gate {
            gate(Rc::new(restore_session), cx);
        } else {
            restore_session(cx);
        }
        if smoke {
            cx.quit();
        } else if objc2_foundation::NSBundle::mainBundle()
            .bundleIdentifier()
            .is_none()
        {
            // A bare binary (`cargo run`) isn't activated by LaunchServices.
            // A bundled app never asks: LaunchServices brings a normal launch
            // forward itself, and a background one (`open -g`, a login item,
            // an agent) must stay behind the person's app; an activation
            // request at launch could take the launcher's grant and come
            // to the front anyway.
            cx.activate(false);
        }
    });
    Ok(())
}
pub fn tokens() -> Tokens {
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
    theme::tokens(scheme, native::settings())
}
/// Reopens the saved windows (all of them only when the system keeps windows
/// on quit; see `Session::restore_plan`). With no session, one new window.
fn restore_session(cx: &mut App) {
    let plan = view::session_path()
        .filter(|p| p.exists())
        .and_then(|p| match session::Session::load(&p) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("Window session could not be restored: {e}");
                None
            }
        })
        .map(|s| s.restore_plan(keeps_windows()))
        .unwrap_or_default();
    if plan.is_empty() {
        open_editor_window(cx, true, None, None);
        return;
    }
    for state in plan {
        let leader = state.tab_group;
        open_editor_window(cx, true, Some(state), leader);
    }
}
/// System Settings ▸ Desktop & Dock ▸ “Close windows when quitting an application”
/// off ⇒ `NSQuitAlwaysKeepsWindows`. `PM_NATIVE_RESTORE=all|one` overrides it.
fn keeps_windows() -> bool {
    match std::env::var("PM_NATIVE_RESTORE").as_deref() {
        Ok("all") => return true,
        Ok("one") => return false,
        _ => {}
    }
    use objc2_foundation::{NSString, NSUserDefaults};
    NSUserDefaults::standardUserDefaults()
        .boolForKey(&NSString::from_str("NSQuitAlwaysKeepsWindows"))
}
/// A new editor window, cascaded from the key one (File ▸ New Window).
pub fn open_editor(cx: &mut App) {
    open_editor_window(cx, true, None, None);
}
/// Opens a new editor window as a native tab of the window with session id `leader`.
pub(crate) fn open_editor_tab(cx: &mut App, leader: u64) {
    open_editor_window(cx, true, None, Some(leader).filter(|l| *l != 0));
}
fn next_window_id() -> u64 {
    thread_local! { static NEXT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) }; }
    let saved = view::session_path()
        .filter(|p| p.exists())
        .and_then(|p| session::Session::load(&p).ok())
        .map_or(1, |s| s.next_id());
    NEXT.with(|n| {
        let id = n.get().max(saved);
        n.set(id + 1);
        id
    })
}
fn open_editor_window(
    cx: &mut App,
    visible: bool,
    restore: Option<session::WindowState>,
    tab_leader: Option<u64>,
) {
    // The key editor window, for cascading a brand-new window. Read from AppKit:
    // this can run inside that editor's own update.
    let cascade = native::editor_frame();
    let t = tokens();
    let m = t.metrics;
    let result = cx.open_window(
        WindowOptions::new()
            .show(visible)
            .focus(visible)
            .macos_window_background(MacosWindowBackground::Transparent)
            // Content runs under the glass toolbar (full-size content view), so
            // the window tint and blur continue behind it.
            .titlebar(Some(TitlebarOptions {
                title: Some("Untitled".into()),
                appears_transparent: true,
                traffic_light_position: None,
            }))
            .window_bounds(Some(WindowBounds::Windowed(Bounds::new(
                point(px(m.toolbar), px(m.toolbar)),
                size(px(m.window_width), px(m.window_height)),
            )))),
        move |window, cx| {
            window.set_window_title("Untitled");
            let device = cx.global::<SharedMetalDevice>().0.clone();
            let (host, rx) = native::Host::install(window, &device, t.clone());
            if !visible {
                host.borrow().smoke_validate();
            }
            let window_id = if visible {
                restore.as_ref().map_or_else(next_window_id, |s| s.id)
            } else {
                0
            };
            if let Some(s) = restore.as_ref().filter(|_| visible) {
                host.borrow_mut().restored_project = Some((s.project.clone(), keeps_windows()));
            }
            if visible {
                let h = host.borrow();
                h.set_window_id(window_id);
                match &restore {
                    Some(s) => h.apply_frame(s.frame, s.fullscreen),
                    None if tab_leader.is_none() => {
                        if let Some(from) = cascade {
                            h.cascade_from(from);
                        }
                    }
                    None => {}
                }
                if let Some(leader) = tab_leader {
                    h.join_tabs(leader);
                }
            }
            let integration = cx
                .try_global::<integration::AppIntegration>()
                .map(|g| g.0.clone())
                .map(|factory| factory(host.clone(), window, cx));
            let editor = cx.new(|cx| {
                let mut editor = view::Editor::new(t, Some(host), cx);
                if let Some(i) = &integration {
                    i.observe(cx);
                }
                editor.integration = integration;
                if let Some(state) = &restore {
                    editor.restore(state);
                } else {
                    editor.window_id = window_id;
                }
                editor
            });
            editor.update(cx, |v, cx| {
                cx.observe_window_appearance(window, |v, w, cx| v.follow_appearance(w, cx))
                    .detach();
                cx.observe_window_bounds(window, |v, _, cx| v.schedule_session(cx))
                    .detach();
                cx.observe_window_activation(window, |v, _, cx| v.schedule_session(cx))
                    .detach();
                v.persist_session();
            });
            // A window the user closes leaves the session; quitting keeps them all.
            let closing = editor.downgrade();
            window.on_window_should_close(cx, move |_, cx| {
                let _ = closing.update(cx, |v, _| v.forget_session());
                true
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
            if visible {
                review_open(editor.downgrade(), window.window_handle(), cx);
            }
            editor
        },
    );
    if let Err(e) = result {
        eprintln!("Could not open editor: {e}");
    }
}
/// Development builds only: `--open a,b` (`PM_NATIVE_REVIEW_OPEN`) runs those
/// commands in the first editor window once it has settled, so design review
/// captures reach Settings, sheets and the palette without the menu bar or focus.
fn review_open(editor: WeakEntity<view::Editor>, handle: AnyWindowHandle, cx: &mut App) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let Some(names) = std::env::var("PM_NATIVE_REVIEW_OPEN")
        .ok()
        .filter(|_| cfg!(debug_assertions))
    else {
        return;
    };
    ONCE.call_once(|| {
        cx.spawn(async move |cx| {
            // Wait for the project and built-ins to load, then space the commands out.
            let mut wait = 8;
            for name in names.split(',').map(str::trim).filter(|n| !n.is_empty()) {
                cx.background_executor()
                    .timer(std::time::Duration::from_secs(wait))
                    .await;
                wait = 3;
                let cmd = native::Command {
                    name: name.into(),
                    args: Vec::new(),
                    token: None,
                };
                let _ = handle.update(cx, |_, window, cx| {
                    editor.update(cx, |v, cx| v.command(cmd, window, cx))
                });
            }
        })
        .detach();
    });
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
