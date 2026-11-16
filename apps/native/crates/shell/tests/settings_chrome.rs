//! Settings' sidebar is the system's own material, blended with what is
//! behind the window: nothing opaque may sit between it and the window. GPUI
//! makes its windows opaque with an opaque background; framed as Settings,
//! the window is clear and only the detail carries the page's ground (the
//! acceptance run found the sidebar opaque). Runs on the main thread with
//! activation prohibited; the window is never ordered in.
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColorSpace,
        NSSplitViewController, NSSplitViewItemBehavior, NSView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use pm_shell::settings_window::SettingsChrome;

    let mtm = MainThreadMarker::new().expect("runs on the main thread (harness = false)");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    // As GPUI leaves a window: opaque, an opaque background, its view in a
    // host view that is the window's content.
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(820., 640.)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            true,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.setOpaque(true);
    let host = NSView::new(mtm);
    let gpui = NSView::new(mtm);
    host.addSubview(&gpui);
    window.setContentView(Some(&host));
    let (tx, _rx) = async_channel::unbounded();
    let chrome = SettingsChrome::install(gpui.clone(), tx).expect("framed as Settings");

    assert!(!window.isOpaque(), "an opaque window shows nothing behind the sidebar");
    let alpha = |c: &objc2_app_kit::NSColor| {
        c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()).map_or(1., |c| c.alphaComponent())
    };
    assert!(alpha(&window.backgroundColor()) < 0.01, "the window's own background is clear");
    let split = window
        .contentViewController()
        .and_then(|c| c.downcast::<NSSplitViewController>().ok())
        .expect("a split view controller");
    let items = split.splitViewItems();
    assert_eq!(items.len(), 2);
    assert_eq!(items.objectAtIndex(0).behavior(), NSSplitViewItemBehavior::Sidebar);

    // The page's ground is the detail's, never the window's.
    chrome.set_scheme(true, 0x1e1e20);
    assert!(alpha(&window.backgroundColor()) < 0.01, "a scheme leaves the window clear");
    let detail = items.objectAtIndex(1).viewController(mtm).view();
    assert!(std::ptr::eq(&*detail, &*host), "GPUI's host view is the detail");
    let layer = detail.layer().expect("the detail is layer-backed");
    let ground = layer.backgroundColor().expect("the detail carries the ground");
    let ground = objc2_app_kit::NSColor::colorWithCGColor(&ground)
        .and_then(|c| c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()))
        .expect("an sRGB ground");
    let rgb = [ground.redComponent(), ground.greenComponent(), ground.blueComponent()];
    let want = [0x1e as f64 / 255., 0x1e as f64 / 255., 0x20 as f64 / 255.];
    for (got, want) in rgb.iter().zip(want) {
        assert!((got - want).abs() < 0.01, "ground {rgb:?}");
    }
    drop(chrome);
    window.close();
    println!("settings_chrome: ok");
}
#[cfg(not(target_os = "macos"))]
fn main() {}
