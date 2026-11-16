//! PM-401: the person hides the app with the left button held in one of its
//! windows, then lets go. What the release sets off (here, as in the app, the
//! window brought forward) must not show the app again: ordering any of its
//! windows in unhides the whole app over whatever the person moved on to. It
//! waits until the person shows the app, and happens then.
//!
//! Its own process (GPUI needs its NSApplication), activation prohibited; the
//! only windows it orders in stand far off screen. Exits when done.
#[cfg(all(target_os = "macos", debug_assertions))]
fn main() {
    use gpui::{
        AppContext as _, Application, Bounds, Context, InteractiveElement, IntoElement,
        MacActivationPolicy, MouseButton, ParentElement as _, Render, Styled, Window,
        WindowBounds, WindowOptions, div, point, px, size,
    };
    use gpui_macos::child_window::{app_hidden, waiting_for_shown};
    use objc2::{msg_send, rc::Retained, runtime::AnyObject};
    use objc2_app_kit::{NSApplication, NSEvent, NSView, NSWindow};
    use objc2_foundation::{MainThreadMarker, NSPoint};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{cell::Cell, rc::Rc, time::Duration};

    /// A view that brings its window forward when a press is released in
    /// it, as the editor does after some gestures end.
    struct Editor(Rc<Cell<usize>>);
    impl Render for Editor {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let released = self.0.clone();
            div().size_full().child(
                div()
                    .id("canvas")
                    .size_full()
                    .on_mouse_up(MouseButton::Left, move |_, window, _| {
                        released.set(released.get() + 1);
                        window.activate();
                    }),
            )
        }
    }
    fn native(window: &Window) -> Retained<NSWindow> {
        let RawWindowHandle::AppKit(handle) =
            HasWindowHandle::window_handle(window).unwrap().as_raw()
        else {
            panic!("AppKit window")
        };
        let view = unsafe { &*(handle.ns_view.as_ptr() as *const NSView) };
        view.window().expect("a window")
    }
    // Far off screen, it still draws (and so takes the press).
    // SAFETY: before any other thread starts.
    unsafe { std::env::set_var("PM_NATIVE_DRAW_OCCLUDED", "1") };
    let bounds = Some(WindowBounds::Windowed(Bounds::new(
        point(px(-20000.), px(-20000.)),
        size(px(200.), px(200.)),
    )));
    let released = Rc::new(Cell::new(0));
    Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)))
        .with_activation_policy(MacActivationPolicy::Prohibited)
        .run(move |cx| {
            let seen = released.clone();
            let editor = cx
                .open_window(
                    WindowOptions {
                        show: true,
                        focus: false,
                        window_bounds: bounds,
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| Editor(seen)),
                )
                .unwrap();
            let window = editor.update(cx, |_, w, _| native(w)).unwrap();
            cx.spawn(async move |cx| {
                let wait = |ms| cx.background_executor().timer(Duration::from_millis(ms));
                wait(400).await;
                let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
                assert!(window.isVisible(), "the editor window stands (off screen)");
                let mouse = |kind: usize| -> Retained<NSEvent> {
                    unsafe {
                        let e: *mut AnyObject = msg_send![objc2::class!(NSEvent),
                            mouseEventWithType: kind,
                            location: NSPoint::new(100., 100.),
                            modifierFlags: 0usize,
                            timestamp: 0f64,
                            windowNumber: window.windowNumber(),
                            context: std::ptr::null::<AnyObject>(),
                            eventNumber: 0isize,
                            clickCount: 1isize,
                            pressure: 1f32];
                        Retained::retain(e.cast()).unwrap()
                    }
                };
                // Pressed, hidden with the button held, released.
                window.sendEvent(&mouse(1));
                wait(100).await;
                app.hide(None);
                wait(400).await;
                assert!(app_hidden() && !window.isVisible(), "the person hid the app");
                window.sendEvent(&mouse(2));
                wait(600).await;
                assert_eq!(released.get(), 1, "the release reached the editor");
                assert!(
                    app_hidden() && !window.isVisible(),
                    "the release showed nothing: the app stays hidden"
                );
                assert_eq!(waiting_for_shown(), 1, "bringing the window forward waits");
                // The person shows the app: the window comes forward then.
                app.unhide(None);
                wait(400).await;
                assert!(!app_hidden() && window.isVisible(), "shown again");
                assert_eq!(waiting_for_shown(), 0, "what waited ran");
                println!("hidden_release: a release in a hidden app's window shows nothing until the person shows the app");
                std::process::exit(0);
            })
            .detach();
        });
}
#[cfg(not(all(target_os = "macos", debug_assertions)))]
fn main() {
    println!("hidden_release: skipped (macOS development build)");
}
