//! The real shell assembly must preserve the app's save/close callback.
//! Separate process: AppKit owns one application and requires the main thread.
#[cfg(target_os = "macos")]
fn main() {
    use gpui::{AnyElement, App, Context, Window};
    use pm_shell::{Command, Host, integration::Integration, layout::Rect};
    use std::{cell::Cell, rc::Rc, time::Duration};
    struct Guard;
    impl Integration for Guard {
        fn observe(&self, _: &mut Context<pm_shell::view::Editor>) {}
        fn command(&self, _: &Command, _: &mut Window, _: &mut App) -> bool {
            false
        }
        fn panel(&self, _: &str, _: Rect, _: &mut Window, _: &mut App) -> Option<AnyElement> {
            None
        }
        fn project_id(&self, _: &App) -> Option<String> {
            None
        }
        fn status(&self, _: &App) -> String {
            String::new()
        }
        fn drop_paths(&self, _: Vec<std::path::PathBuf>, _: &mut App) {}
    }
    let scratch = std::env::temp_dir().join(format!(
        "pm-close-guard-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&scratch).unwrap();
    // No restored user windows and no writes to the person's shell session.
    unsafe {
        std::env::set_var("PM_NATIVE_SHELL_STATE", scratch.join("session.json"));
    }
    let called = Rc::new(Cell::new(0));
    let factory = Rc::new(
        move |host: Rc<std::cell::RefCell<Host>>,
              window: &mut Window,
              cx: &mut App|
              -> Rc<dyn Integration> {
            let count = called.clone();
            window.on_window_should_close(cx, move |_, _| {
                count.set(count.get() + 1);
                false // The app is saving or waiting for the person's decision.
            });
            let handle = window.window_handle();
            let native = host.borrow().window.clone();
            let count = called.clone();
            cx.spawn(async move |cx| {
                cx.background_executor()
                    .timer(Duration::from_millis(200))
                    .await;
                native.performClose(None);
                cx.update(|cx| {
                    assert_eq!(count.get(), 1, "the shell replaced the app's close guard");
                    assert!(
                        cx.windows().contains(&handle),
                        "a refused close removed the project window"
                    );
                    handle
                        .update(cx, |_, window, _| window.remove_window())
                        .unwrap();
                    cx.quit();
                });
            })
            .detach();
            Rc::new(Guard)
        },
    );
    pm_shell::run_integrated(metal::Device::system_default().expect("Metal"), factory).unwrap();
    pm_shell::session::flush().unwrap();
    std::fs::remove_dir_all(scratch).unwrap();
}
#[cfg(not(target_os = "macos"))]
fn main() {}
