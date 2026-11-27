//! P-LAUNCH: the launch callback runs before AppKit's run loop starts, in an
//! autorelease pool of its own (NativeGPUI 0003, 0007). A window it opens and
//! then closes must be freed, not kept by the pool `run` drains only at quit:
//! a kept window stayed in the window server and in `orderedWindows`.
//!
//! Its own process (GPUI needs its NSApplication), activation prohibited; the
//! window stands far off screen. Exits when done.
#[cfg(target_os = "macos")]
fn main() {
    use gpui::{
        AppContext as _, Application, Bounds, Context, IntoElement, MacActivationPolicy, Render,
        Window, WindowBounds, WindowOptions, div, point, px, size,
    };
    use objc2::rc::Weak;
    use objc2_app_kit::{NSApplication, NSView, NSWindow};
    use objc2_foundation::MainThreadMarker;
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
    fn listed(number: isize) -> bool {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        app.orderedWindows()
            .iter()
            .any(|w| w.windowNumber() == number)
    }
    Application::with_platform(Rc::new(gpui_macos::MacPlatform::new(false)))
        .with_activation_policy(MacActivationPolicy::Prohibited)
        .run(move |cx| {
            let window = cx
                .open_window(
                    WindowOptions {
                        focus: false,
                        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                            point(px(-20000.), px(-20000.)),
                            size(px(200.), px(200.)),
                        ))),
                        ..Default::default()
                    },
                    |_, cx| cx.new(|_| Empty),
                )
                .unwrap();
            let (weak, number) = window
                .update(cx, |_, w, _| {
                    let ns = native(w);
                    (Weak::from_retained(&ns), ns.windowNumber())
                })
                .unwrap();
            assert!(
                listed(number),
                "the window opened in the launch callback is listed"
            );
            window.update(cx, |_, w, _| w.remove_window()).unwrap();
            cx.spawn(async move |cx| {
                for _ in 0..50 {
                    cx.background_executor()
                        .timer(Duration::from_millis(100))
                        .await;
                    if weak.load().is_none() {
                        break;
                    }
                }
                assert!(
                    weak.load().is_none(),
                    "a window the launch callback opened is freed after it closes"
                );
                assert!(!listed(number), "and no longer listed");
                println!("launch_release: ok");
                std::process::exit(0);
            })
            .detach();
        });
}
#[cfg(not(target_os = "macos"))]
fn main() {}
