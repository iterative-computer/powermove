//! Closing a project tab from inside a GPUI update (its own process: GPUI
//! needs its NSApplication subclass, one application per process).
#[cfg(target_os = "macos")]
fn main() {
    close_inside_an_update();
}
#[cfg(not(target_os = "macos"))]
fn main() {}

/// A close asked for while GPUI is updating (a command handler closing a
/// tab) must still reach the window's should-close handler, which owns the
/// save question, and must leave the app and the other project running.
/// `performClose` there skips the handler and aborts in the close callback;
/// `request_close` asks on the next turn of the run loop. Windows stay off
/// screen and activation is prohibited. Exits the process when done.
#[cfg(target_os = "macos")]
fn close_inside_an_update() {
    use gpui::{
        AppContext as _, Application, Bounds, Context, IntoElement, MacActivationPolicy, Render,
        Window, WindowBounds, WindowOptions, div, point, px, size,
    };
    use objc2_app_kit::{NSView, NSWindow};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{cell::Cell, rc::Rc, time::Duration};
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
    let options = || WindowOptions {
        show: false,
        focus: false,
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(-20000.), px(-20000.)),
            size(px(200.), px(200.)),
        ))),
        ..Default::default()
    };
    Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)))
        .with_activation_policy(MacActivationPolicy::Prohibited)
        .run(move |cx| {
            let first = cx
                .open_window(options(), |_, cx| cx.new(|_| Empty))
                .unwrap();
            let second = cx
                .open_window(options(), |_, cx| cx.new(|_| Empty))
                .unwrap();
            // As the app does: the handler decides (a save question), and
            // the window closes only when the app removes it.
            let asked = Rc::new(Cell::new(0));
            let count = asked.clone();
            second
                .update(cx, |_, window, cx| {
                    window.on_window_should_close(cx, move |_, _| {
                        count.set(count.get() + 1);
                        false
                    });
                })
                .unwrap();
            let second_ns = second.update(cx, |_, w, _| native(w)).unwrap();
            // The first window's command handler closes the second tab.
            first
                .update(cx, |_, _, _| pm_shell::request_close(&second_ns))
                .unwrap();
            assert_eq!(asked.get(), 0, "nothing closes inside the update");
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                assert_eq!(asked.get(), 1, "the should-close handler was asked once");
                let open = cx.update(|cx| cx.windows().len());
                assert_eq!(open, 2, "a refused close leaves both projects open");
                // The handler agrees: only that window goes.
                cx.update(|cx| {
                    second.update(cx, |_, w, _| w.remove_window()).unwrap();
                });
                cx.background_executor()
                    .timer(Duration::from_millis(300))
                    .await;
                let open = cx.update(|cx| cx.windows().len());
                assert_eq!(open, 1, "the other project's window stays");
                println!("tab_close: a close asked inside an update reaches the handler and keeps the app");
                std::process::exit(0);
            })
            .detach();
        });
}
