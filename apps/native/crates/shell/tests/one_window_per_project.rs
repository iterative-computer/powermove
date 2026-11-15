//! H3: one project per window. Two editor windows, with the system set to
//! tab every window, never form a tab group: a window showing Home is never
//! a "Home" tab beside another window's project. Runs on the main thread
//! with activation prohibited; the windows stand far off screen.
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSWindow,
        NSWindowStyleMask,
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
        pm_shell::one_project_per_window(&w);
        w
    };
    let project = window("Paper");
    let home = window("Home");
    // (Windows set to prefer tabs do group here: the check can fail.)
    project.makeKeyAndOrderFront(None);
    home.makeKeyAndOrderFront(None);
    for w in [&project, &home] {
        let tabs = w.tabbedWindows().map_or(0, |t| t.count());
        assert!(
            tabs <= 1,
            "{} stands in a tab bar with {tabs} tabs",
            w.title()
        );
    }
    project.close();
    home.close();
    println!("one_window_per_project: ok");
}
#[cfg(not(target_os = "macos"))]
fn main() {}
