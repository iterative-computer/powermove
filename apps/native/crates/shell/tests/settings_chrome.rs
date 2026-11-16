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
    use objc2_foundation::{NSObjectProtocol, NSPoint, NSRect, NSSize};
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
    // The system's own floating sidebar: its glass holds our sidebar view,
    // the detail runs under it, and the page starts beside it.
    window.setFrame_display(NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(820., 640.)), false);
    if let Some(frame) = window.contentView().and_then(|c| unsafe { c.superview() }) {
        frame.layoutSubtreeIfNeeded();
    }
    if std::env::var_os("PM_DUMP_SIDEBAR").is_some() {
        // Diagnostic: the views the system put around the sidebar.
        fn dump(v: &NSView, depth: usize) {
            let f = v.frame();
            eprintln!(
                "{:indent$}{} ({:.0},{:.0} {:.0}x{:.0})",
                "",
                v.class().name().to_string_lossy(),
                f.origin.x,
                f.origin.y,
                f.size.width,
                f.size.height,
                indent = depth * 2
            );
            if depth < 9 {
                for c in v.subviews().iter() {
                    dump(&c, depth + 1);
                }
            }
        }
        if let Some(frame) = window.contentView().and_then(|c| unsafe { c.superview() }) {
            dump(&frame, 0);
        }
    }
    let sidebar_view = items.objectAtIndex(0).viewController(mtm).view();
    let mut glass = false;
    if let Some(class) = objc2::runtime::AnyClass::get(c"NSGlassEffectView") {
        let mut up = unsafe { sidebar_view.superview() };
        while let Some(v) = up {
            glass |= v.isKindOfClass(class);
            up = unsafe { v.superview() };
        }
        assert!(items.objectAtIndex(1).automaticallyAdjustsSafeAreaInsets(), "the detail runs under the sidebar");
        assert!(glass, "the sidebar is the system's glass");
        let page = gpui.convertRect_toView(gpui.bounds(), None);
        assert!(
            page.origin.x >= pm_shell::settings_window::SIDEBAR_WIDTH - 1.,
            "the page starts beside the sidebar, at {}",
            page.origin.x
        );
    }

    // The page's ground is beside the sidebar, never under it: nothing under
    // the sidebar's glass is painted (the window, the detail), so it shows
    // what is behind the window. (Acceptance found it a flat fill when the
    // ground was on the detail, which runs under the sidebar.)
    chrome.set_scheme(true, 0x1e1e20);
    assert!(alpha(&window.backgroundColor()) < 0.01, "a scheme leaves the window clear");
    let detail = items.objectAtIndex(1).viewController(mtm).view();
    assert!(std::ptr::eq(&*detail, &*host), "GPUI's host view is the detail");
    if let Some(frame) = window.contentView().and_then(|c| unsafe { c.superview() }) {
        frame.layoutSubtreeIfNeeded();
    }
    let painted = |v: &NSView| {
        v.layer()
            .and_then(|l| l.backgroundColor())
            .and_then(|c| objc2_app_kit::NSColor::colorWithCGColor(&c))
            .and_then(|c| c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace()))
    };
    assert!(
        painted(&detail).is_none_or(|c| c.alphaComponent() < 0.01),
        "the detail, under the sidebar, is unpainted"
    );
    let grounds: Vec<_> = detail
        .subviews()
        .iter()
        .filter(|v| painted(v).is_some_and(|c| c.alphaComponent() > 0.99))
        .collect();
    assert_eq!(grounds.len(), 1, "one view carries the page's ground");
    let ground = &grounds[0];
    let at = ground.convertRect_toView(ground.bounds(), None);
    let top = window.contentView().map_or(0., |c| c.bounds().size.height);
    assert!(
        at.origin.x >= pm_shell::settings_window::SIDEBAR_WIDTH - 1.,
        "the ground starts beside the sidebar, at {}",
        at.origin.x
    );
    assert!(at.origin.y + at.size.height >= top - 1., "the ground runs up under the toolbar");
    let c = painted(ground).unwrap();
    let rgb = [c.redComponent(), c.greenComponent(), c.blueComponent()];
    let want = [0x1e as f64 / 255., 0x1e as f64 / 255., 0x20 as f64 / 255.];
    for (got, want) in rgb.iter().zip(want) {
        assert!((got - want).abs() < 0.01, "ground {rgb:?}");
    }
    // The report a release bundle writes (PM_NATIVE_SETTINGS_PROBE) says the same.
    let report = chrome.report();
    assert_eq!(report["opaque"], false, "{report}");
    assert!(report["backgroundAlpha"].as_f64().unwrap() < 0.01, "{report}");
    assert_eq!(report["detailPainted"], false, "{report}");
    assert!(report["groundX"].as_f64().unwrap() >= pm_shell::settings_window::SIDEBAR_WIDTH - 1., "{report}");
    if report["glassAvailable"] == true {
        assert_eq!(report["sidebarInGlass"], true, "{report}");
    }
    println!("settings_chrome report: {report}");
    drop(chrome);
    window.close();
    println!("settings_chrome: ok");
}
#[cfg(not(target_os = "macos"))]
fn main() {}
