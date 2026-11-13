//! P-CHILD through GPUI's own window code (its own process: GPUI needs its
//! NSApplication subclass, and AppKit makes one application per process).
#[cfg(target_os = "macos")]
fn main() {
    gpui_open_path();
}
#[cfg(not(target_os = "macos"))]
fn main() {}

/// GPUI's real open path, as the app opens the Agent popover: a PopUp asked to
/// show and take focus while the app is inactive stays off screen, attached to
/// the window that asked, through its activate path too, and never shows once
/// that window closed. A Dialog begins as a sheet at once in a normal launch;
/// in a development lane launch a sheet (a Dialog, GPUI's own prompt) asked
/// for while inactive begins on nothing and moves no window: it waits for
/// activation and is cancelled once its window closed. Activation is
/// prohibited, as in the shell's smoke mode; the parent is never shown.
/// Exits the process when done.
#[cfg(target_os = "macos")]
fn gpui_open_path() {
    use gpui::PromptLevel;
    use gpui::{
        AppContext as _, Application, Bounds, Context, IntoElement, MacActivationPolicy, Render,
        Window, WindowBounds, WindowKind, WindowOptions, div, point, px, size,
    };
    use gpui_macos::child_window::{follows_policy, is_pending, sheet_pending, show_pending};
    use objc2_app_kit::{NSView, NSWindow};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{rc::Rc, time::Duration};
    struct Empty;
    impl Render for Empty {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    fn native(window: &Window) -> objc2::rc::Retained<NSWindow> {
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            panic!("AppKit window")
        };
        let view = unsafe { &*(handle.ns_view.as_ptr() as *const NSView) };
        view.window().expect("a window")
    }
    let bounds = Some(WindowBounds::Windowed(Bounds::new(
        point(px(-20000.), px(-20000.)),
        size(px(200.), px(200.)),
    )));
    Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)))
        .with_activation_policy(MacActivationPolicy::Prohibited)
        .run(move |cx| {
            let parent = cx
                .open_window(
                    WindowOptions {
                        show: false,
                        focus: false,
                        window_bounds: bounds,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| Empty),
                )
                .unwrap();
            let mut options = WindowOptions {
                kind: WindowKind::PopUp,
                show: true,
                focus: true,
                window_bounds: bounds,
                ..Default::default()
            };
            options.parent = Some(parent.into());
            let popup = cx.open_window(options, |_, cx| cx.new(|_| Empty)).unwrap();
            let parent_ns = parent.update(cx, |_, w, _| native(w)).unwrap();
            let popup_ns = popup.update(cx, |_, w, _| native(w)).unwrap();
            assert!(follows_policy(&popup_ns), "the pop-up keeps the policy: level {} hides {} behaviour {:?}", popup_ns.level(), popup_ns.hidesOnDeactivate(), popup_ns.collectionBehavior());
            assert!(!popup_ns.isVisible(), "GPUI's open shows nothing while inactive");
            assert!(is_pending(&popup_ns), "it waits for the app to become active");
            assert!(
                popup_ns
                    .parentWindow()
                    .is_some_and(|p| std::ptr::eq(&*p, &*parent_ns)),
                "a child of the window that asked"
            );
            // A normal launch: a Dialog begins as a sheet at once, as AppKit
            // does, even while the app is inactive.
            let mut options = WindowOptions {
                kind: WindowKind::Dialog,
                show: true,
                focus: true,
                window_bounds: bounds,
                ..Default::default()
            };
            let own = cx
                .open_window(
                    WindowOptions {
                        show: false,
                        focus: false,
                        window_bounds: bounds,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| Empty),
                )
                .unwrap();
            let own_ns = own.update(cx, |_, w, _| native(w)).unwrap();
            options.parent = Some(own.into());
            let attached = cx.open_window(options, |_, cx| cx.new(|_| Empty)).unwrap();
            let attached_ns = attached.update(cx, |_, w, _| native(w)).unwrap();
            assert!(
                attached_ns
                    .sheetParent()
                    .is_some_and(|p| std::ptr::eq(&*p, &*own_ns)),
                "a normal launch begins the sheet at once"
            );
            assert!(!sheet_pending(&own_ns));
            own_ns.endSheet(&attached_ns);
            attached.update(cx, |_, w, _| w.remove_window()).unwrap();
            own.update(cx, |_, w, _| w.remove_window()).unwrap();
            // A development lane launch: a sheet asked for while inactive
            // waits, attached to nothing.
            gpui_macos::child_window::hold_sheets_while_inactive(false);
            let mut options = WindowOptions {
                kind: WindowKind::Dialog,
                show: true,
                focus: true,
                window_bounds: bounds,
                ..Default::default()
            };
            options.parent = Some(parent.into());
            let dialog = cx.open_window(options, |_, cx| cx.new(|_| Empty)).unwrap();
            let dialog_ns = dialog.update(cx, |_, w, _| native(w)).unwrap();
            assert!(!dialog_ns.isVisible(), "a sheet shows nothing while inactive");
            assert!(dialog_ns.sheetParent().is_none(), "it is begun on nothing");
            assert!(parent_ns.attachedSheet().is_none());
            assert!(!parent_ns.isVisible(), "its window was not ordered in");
            assert!(sheet_pending(&parent_ns), "it waits for the app to become active");
            let answer = popup
                .update(cx, |_, w, cx| {
                    w.prompt(PromptLevel::Info, "Question", None, &["OK"], cx)
                })
                .unwrap();
            popup.update(cx, |_, w, _| w.activate()).unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                assert!(!popup_ns.isVisible(), "activate shows nothing while inactive");
                assert!(is_pending(&popup_ns));
                assert!(
                    popup_ns.attachedSheet().is_none() && sheet_pending(&popup_ns),
                    "GPUI's prompt waits too"
                );
                assert!(!dialog_ns.isVisible() && sheet_pending(&parent_ns));
                cx.update(|cx| {
                    parent.update(cx, |_, w, _| w.remove_window()).unwrap();
                });
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                show_pending();
                assert!(
                    !popup_ns.isVisible(),
                    "its window closed while it waited: it stays hidden"
                );
                assert!(
                    !dialog_ns.isVisible() && !sheet_pending(&parent_ns),
                    "a sheet whose window closed is dropped"
                );
                assert!(
                    !sheet_pending(&popup_ns),
                    "the prompt on a hidden window is dropped"
                );
                assert!(
                    answer.await.is_err(),
                    "a dropped prompt is cancelled, not answered"
                );
                println!("child_windows: GPUI's open and activate keep a pop-up off screen while inactive, and a dialog sheet and a prompt wait for activation");
                std::process::exit(0);
            })
            .detach();
        });
}
