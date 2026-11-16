//! Settings › General › Open Projects In, one check per mode. Tabs: an editor
//! window never joins a tab bar on the system's initiative (a window showing
//! Home is never a "Home" tab, H3), and the app's own join makes one tab
//! group (a project opened from a window in use). Separate Windows: two
//! editor windows stay two windows with the system set to tab every window.
//! Runs on the main thread with activation prohibited; the windows stand far
//! off screen.
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowOrderingMode, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    let mtm = MainThreadMarker::new().expect("runs on the main thread (harness = false)");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    NSWindow::setAllowsAutomaticWindowTabbing(true, mtm);
    let window = |title: &str| {
        let frame = NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(400., 300.));
        let w = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Titled | NSWindowStyleMask::Closable,
                NSBackingStoreType::Buffered,
                true,
            )
        };
        unsafe { w.setReleasedWhenClosed(false) };
        w.setTitle(&NSString::from_str(title));
        pm_shell::editor_window_tabbing(&w);
        w
    };
    let tabs = |w: &Retained<NSWindow>| w.tabbedWindows().map_or(1, |t| t.count());

    // Tabs: nothing joins on its own; the app's join makes a group of two.
    pm_shell::set_tabs_mode(true);
    let project = window("Paper");
    let home = window("Home");
    project.makeKeyAndOrderFront(None);
    home.makeKeyAndOrderFront(None);
    assert_eq!(tabs(&home), 1, "a Home window joined a tab bar on its own");
    let other = window("Launch day");
    project.addTabbedWindow_ordered(&other, NSWindowOrderingMode::Above);
    assert_eq!(tabs(&project), 2, "the app's join made no tab group");
    for w in [&project, &home, &other] {
        w.close();
    }
    println!("window_tabs: tabs mode ok");

    // Separate Windows: two windows stay two windows.
    pm_shell::set_tabs_mode(false);
    let first = window("Paper");
    let second = window("Launch day");
    first.makeKeyAndOrderFront(None);
    second.makeKeyAndOrderFront(None);
    for w in [&first, &second] {
        assert_eq!(tabs(w), 1, "{} stands in a tab bar", w.title());
    }
    for w in [&first, &second] {
        w.close();
    }
    println!("window_tabs: separate windows mode ok");
}
#[cfg(not(target_os = "macos"))]
fn main() {}
