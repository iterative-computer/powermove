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
/// that window closed. Activation is prohibited, as in the shell's smoke mode;
/// the parent is never shown. Exits the process when done.
#[cfg(target_os = "macos")]
fn gpui_open_path() {
    use gpui::{
        AppContext as _, Application, Bounds, Context, IntoElement, MacActivationPolicy, Render,
        Window, WindowBounds, WindowKind, WindowOptions, div, point, px, size,
    };
    use gpui_macos::child_window::{follows_policy, is_pending, show_pending};
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
            popup.update(cx, |_, w, _| w.activate()).unwrap();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                assert!(!popup_ns.isVisible(), "activate shows nothing while inactive");
                assert!(is_pending(&popup_ns));
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
                println!("child_windows: GPUI's open and activate keep a pop-up off screen while inactive");
                std::process::exit(0);
            })
            .detach();
        });
}
