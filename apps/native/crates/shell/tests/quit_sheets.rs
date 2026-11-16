//! PM-464 and its review: the Quit item works from the first menu on and
//! beside any sheet, and a quit finds a sheet on any window of the app (a
//! popover's file chooser included) and brings that window forward.
//!
//! Its own process on the main thread (AppKit), activation prohibited; its
//! windows stand far off screen. Exits when done.
#[cfg(target_os = "macos")]
fn main() {
    use objc2::{MainThreadMarker, MainThreadOnly, rc::Retained, sel};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSPanel, NSWindow,
        NSWindowOrderingMode, NSWindowStyleMask,
    };
    use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
    use std::{cell::Cell, rc::Rc};

    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let spin = || NSRunLoop::currentRunLoop().runUntilDate(&NSDate::dateWithTimeIntervalSinceNow(0.2));

    // 1. Before any coordinator exists (Welcome, onboarding, a failed library),
    // the Quit item has a responder and is enabled.
    pm_shell::install_menus();
    let quit = sel!(pmQuit:);
    let target = unsafe { app.targetForAction_to_from(quit, None, None) };
    assert!(target.is_some(), "no responder for the Quit item before a coordinator");
    let menu = app.mainMenu().expect("menus installed");
    let item = (0..menu.numberOfItems())
        .filter_map(|i| menu.itemAtIndex(i)?.submenu())
        .flat_map(|sub| (0..sub.numberOfItems()).filter_map(move |i| sub.itemAtIndex(i)))
        .find(|item| item.action() == Some(quit))
        .expect("a Quit item with the quit action");
    unsafe { item.menu() }.unwrap().update();
    assert!(item.isEnabled(), "the Quit item is disabled before a coordinator");
    // Once the coordinator sets its hook, the item reaches it.
    let quits = Rc::new(Cell::new(0));
    let seen = quits.clone();
    pm_shell::set_quit_hook(move || seen.set(seen.get() + 1));
    assert!(unsafe { app.sendAction_to_from(quit, None, None) });
    assert_eq!(quits.get(), 1, "the Quit item did not reach the quit hook");

    // 2. Windows: an editor with a second tab, and a popover (a child
    // window) on that tab.
    let window = |panel: bool| -> Retained<NSWindow> {
        let frame = NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(400., 300.));
        let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        let w: Retained<NSWindow> = unsafe {
            if panel {
                Retained::into_super(NSPanel::initWithContentRect_styleMask_backing_defer(
                    NSPanel::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false,
                ))
            } else {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    NSWindow::alloc(mtm), frame, style, NSBackingStoreType::Buffered, false,
                )
            }
        };
        unsafe { w.setReleasedWhenClosed(false) };
        w
    };
    let first = window(false);
    let second = window(false);
    first.orderFront(None);
    first.addTabbedWindow_ordered(&second, NSWindowOrderingMode::Above);
    let group = first.tabGroup().expect("a tab group");
    group.setSelectedWindow(Some(&first));
    let popover = window(true);
    unsafe { second.addChildWindow_ordered(&popover, NSWindowOrderingMode::Above) };
    spin();
    assert!(pm_shell::quit_blocker_window().is_none(), "no sheet, nothing holds a quit");

    // 3. A file chooser stand-in on the popover, then a sheet on that sheet:
    // the popover is what holds the quit, with the deepest sheet.
    let chooser = window(true);
    popover.beginSheet_completionHandler(&chooser, None);
    spin();
    let (holder, sheet) = pm_shell::quit_blocker_window().expect("the popover's sheet holds a quit");
    assert!(*holder == *popover, "the blocker is the popover, not an editor");
    assert!(*sheet == *chooser);
    let nested = window(true);
    chooser.beginSheet_completionHandler(&nested, None);
    spin();
    let (holder, sheet) = pm_shell::quit_blocker_window().expect("still held");
    assert!(*holder == *popover && *sheet == *nested, "the deepest sheet of the chain");

    // 4. With the other tab chosen, bringing it forward chooses the popover's.
    group.setSelectedWindow(Some(&first));
    spin();
    assert!(group.selectedWindow().is_some_and(|w| *w == *first));
    pm_shell::bring_forward(&holder);
    spin();
    assert!(
        group.selectedWindow().is_some_and(|w| *w == *second),
        "the popover's tab was not chosen"
    );

    chooser.endSheet(&nested);
    popover.endSheet(&chooser);
    spin();
    assert!(pm_shell::quit_blocker_window().is_none(), "sheets ended, nothing holds a quit");
    println!("quit_sheets: ok");
}
#[cfg(not(target_os = "macos"))]
fn main() {}
