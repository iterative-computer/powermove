//! P-CHILD: every child window (GPUI pop-ups and floating windows such as the
//! Agent popover, the command palette and field menu, the colour panel) sits at
//! the normal level, hides while the app is inactive and never joins every
//! Space, so none can float over other applications. A sheet asked for while
//! the app is inactive waits for activation instead of ordering its window
//! front. Runs on the main thread with activation prohibited; the only
//! windows it orders in stand far off screen.
#[cfg(target_os = "macos")]
fn main() {
    use gpui_macos::child_window::{
        apply, attach, follows_policy, forget, forget_sheet, forget_sheets_on,
        hold_sheets_while_inactive, is_pending, restore_observed, sheet, sheet_pending, show,
        show_pending, would_hold,
    };
    use objc2::{MainThreadMarker, MainThreadOnly, Message};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColorPanel, NSPanel,
        NSPopUpMenuWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};
    use std::{cell::Cell, rc::Rc};

    let mtm = MainThreadMarker::new().expect("runs on the main thread (harness = false)");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Prohibited);
    let frame = NSRect::new(NSPoint::new(-20000., -20000.), NSSize::new(10., 10.));
    let window = |style| unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            true,
        )
    };
    let panel = |style| {
        NSPanel::initWithContentRect_styleMask_backing_defer(
            NSPanel::alloc(mtm),
            frame,
            style,
            NSBackingStoreType::Buffered,
            true,
        )
    };
    let mut checked = 0;
    let mut check = |name: &str, w: &NSWindow| {
        // How GPUI made pop-ups before: global pop-up level, every Space.
        w.setLevel(NSPopUpMenuWindowLevel);
        w.setHidesOnDeactivate(false);
        w.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        assert!(
            !follows_policy(w),
            "{name}: the old configuration must fail"
        );
        apply(w);
        assert!(follows_policy(w), "{name}");
        assert_eq!(w.level(), 0, "{name}: normal level");
        assert!(w.hidesOnDeactivate(), "{name}: hides while inactive");
        assert!(
            !w.collectionBehavior()
                .contains(NSWindowCollectionBehavior::CanJoinAllSpaces),
            "{name}: not on every Space"
        );
        assert!(!w.isVisible(), "{name}: never shown by the test");
        checked += 1;
    };
    // GPUI's Normal/Floating windows and its pop-ups (non-activating panels).
    check("titled window", &window(NSWindowStyleMask::Titled));
    check(
        "pop-up panel",
        &panel(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel),
    );
    // The palette and field menu are borderless panels.
    check("borderless panel", &panel(NSWindowStyleMask::Borderless));
    // The system colour panel floats by default; the app turns that off.
    let colours = NSColorPanel::sharedColorPanel(mtm);
    colours.setFloatingPanel(false);
    check("colour panel", &colours);

    // Attaching to the window that asked keeps the policy (AppKit resets a
    // child's hiding and Spaces behaviour), and a shared window moves.
    let parent = window(NSWindowStyleMask::Titled);
    let other = window(NSWindowStyleMask::Titled);
    let child = panel(NSWindowStyleMask::Borderless);
    apply(&child);
    attach(&child, &parent);
    assert!(follows_policy(&child), "an attached child keeps the policy");
    assert!(
        child
            .parentWindow()
            .is_some_and(|p| std::ptr::eq(&*p, &*parent))
    );
    attach(&child, &other);
    assert!(follows_policy(&child));
    assert!(
        child
            .parentWindow()
            .is_some_and(|p| std::ptr::eq(&*p, &*other))
    );
    assert!(
        parent.childWindows().is_none_or(|c| c.is_empty()),
        "moved, not copied"
    );
    assert!(!child.isVisible() && !parent.isVisible() && !other.isVisible());

    // Shown while the app is inactive (this process can't be active): the
    // window waits off screen; a hide forgets it; activation shows it.
    assert!(!app.isActive());
    let waiting = panel(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    apply(&waiting);
    show(&waiting, false);
    assert!(
        !waiting.isVisible(),
        "an inactive app shows no child window"
    );
    assert!(is_pending(&waiting));
    forget(&waiting);
    assert!(
        !is_pending(&waiting),
        "a hidden or closed window never comes back"
    );
    show(&waiting, false);
    show_pending();
    assert!(!is_pending(&waiting));
    assert!(waiting.isVisible(), "it shows once the app is active");
    waiting.orderOut(None);
    assert!(!waiting.isVisible());

    // A pop-up asked for over a window on screen while the app is inactive
    // (the Agent popover from a review link or a tool): AppKit orders a new
    // child in with its visible parent at once, and hiding it then took the
    // editor window off screen too. It waits unattached and joins its
    // parent only when it shows.
    let host = window(NSWindowStyleMask::Titled);
    host.orderBack(None);
    assert!(host.isVisible());
    let pop = panel(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    attach(&pop, &host);
    assert!(follows_policy(&pop), "a waiting child keeps the policy");
    assert!(
        !pop.isVisible() && pop.parentWindow().is_none(),
        "not ordered in with its parent"
    );
    show(&pop, false);
    assert!(!pop.isVisible() && is_pending(&pop) && host.isVisible());
    show_pending();
    assert!(pop.isVisible() && host.isVisible());
    assert!(
        pop.parentWindow()
            .is_some_and(|p| std::ptr::eq(&*p, &*host)),
        "joins its parent when it shows"
    );
    assert!(follows_policy(&pop));
    pop.orderOut(None);
    host.orderOut(None);

    // SHEETS. A normal launch: a sheet asked for while the app is inactive
    // begins at once, exactly as AppKit does (the behaviour before the lane
    // gate existed); nothing waits and nothing is cancelled.
    struct Flag(Rc<Cell<bool>>);
    impl Drop for Flag {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let on = window(NSWindowStyleMask::Titled);
    on.orderBack(None);
    let sheet_window = window(NSWindowStyleMask::Titled);
    // (began, cancelled, closure dropped)
    let ask = |parent: &NSWindow, sheet_window: &NSWindow| {
        let (began, cancelled, dropped) = (
            Rc::new(Cell::new(false)),
            Rc::new(Cell::new(false)),
            Rc::new(Cell::new(false)),
        );
        let (b, c, guard) = (began.clone(), cancelled.clone(), Flag(dropped.clone()));
        let (p, shown) = (parent.retain(), sheet_window.retain());
        let now = sheet(
            parent,
            sheet_window,
            move || {
                let _keep = &guard;
                p.beginSheet_completionHandler(&shown, None);
                b.set(true);
            },
            move || c.set(true),
        );
        (now, began, cancelled, dropped)
    };
    assert!(
        !app.isActive() && !would_hold(),
        "a normal launch never holds"
    );
    let (now, began, cancelled, _) = ask(&on, &sheet_window);
    assert!(now && began.get() && !cancelled.get(), "begins at once");
    assert!(!sheet_pending(&on));
    assert!(
        on.attachedSheet()
            .is_some_and(|s| std::ptr::eq(&*s, &*sheet_window)),
        "on the window that asked, at once"
    );
    on.endSheet(&sheet_window);
    sheet_window.orderOut(None);

    // A development lane launch (the launcher's flag, in process): a sheet
    // asked for while inactive waits, attached to nothing, no window moved.
    hold_sheets_while_inactive(false);
    assert!(
        would_hold(),
        "a lane holds while inactive; an alert answers itself"
    );
    let order = |w: &NSWindow| {
        app.orderedWindows()
            .iter()
            .position(|x| std::ptr::eq(&*x, w))
    };
    let before = order(&on);
    let (now, began, cancelled, _) = ask(&on, &sheet_window);
    assert!(!now && !began.get() && !cancelled.get() && sheet_pending(&on));
    assert!(on.attachedSheet().is_none() && !sheet_window.isVisible());
    assert_eq!(order(&on), before, "no window moved");
    // Its owner closed it before it began (OpenSheet::close, an extension's
    // page closing its modal): the entry goes and its answer is cancelled.
    forget_sheet(&sheet_window);
    assert!(!sheet_pending(&on) && cancelled.get() && !began.get());
    // The window it waits for is torn down: cancelled by parent identity.
    let (_, began, cancelled, _) = ask(&on, &sheet_window);
    forget_sheets_on(&on);
    assert!(!sheet_pending(&on) && cancelled.get() && !began.get());
    // Its window closed before activation: cancelled when the app comes
    // forward (a dropped form's waiting task hears Cancel, never hangs).
    let gone = window(NSWindowStyleMask::Titled);
    let (_, began, cancelled, dropped) = ask(&gone, &sheet_window);
    show_pending();
    assert!(
        cancelled.get() && dropped.get() && !began.get() && !sheet_pending(&gone),
        "its window is not open: cancelled on activation"
    );
    // A window in the Dock keeps its sheet waiting, and its return wakes it
    // (the restore observer runs the same drain while the app is active).
    assert!(
        restore_observed(),
        "a restored window wakes its waiting sheets"
    );
    // With its window open, it begins once the app is active.
    let (_, began, cancelled, _) = ask(&on, &sheet_window);
    show_pending();
    assert!(began.get() && !cancelled.get() && !sheet_pending(&on));
    assert!(
        on.attachedSheet()
            .is_some_and(|s| std::ptr::eq(&*s, &*sheet_window)),
        "on the window that asked"
    );
    on.endSheet(&sheet_window);
    on.orderOut(None);
    // Extension modals cancel what they own when their page closes them or
    // the host goes away.
    let compat = include_str!("../../compat-host/src/mac.rs");
    for owner in ["unsafe fn child_closed", "fn close_sheets"] {
        let at = compat.find(owner).expect(owner);
        assert!(
            compat[at..at + 600].contains("forget_sheet("),
            "{owner} cancels its waiting sheet"
        );
    }
    // An owner-dismissed form forgets its waiting entry.
    let sheets = include_str!("../../app/src/sheets.rs");
    let close = sheets
        .find("pub fn close(&self)")
        .expect("OpenSheet::close");
    assert!(sheets[close..close + 200].contains("forget_sheet("));

    // Every sheet in the app goes through that gate: no source begins one
    // directly. (The gate itself is child_window.rs.)
    let native = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut roots = vec![native.join("vendor/gpui-ce/crates/gpui_macos/src")];
    for crate_dir in std::fs::read_dir(native.join("crates")).unwrap().flatten() {
        roots.push(crate_dir.path().join("src"));
    }
    let mut sites = 0;
    while let Some(dir) = roots.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.flatten().map(|e| e.path()) {
            if path.is_dir() {
                roots.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") || path.ends_with("child_window.rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            // An alert asks at once in a normal launch; in a lane launch in
            // the background it answers itself first (`would_hold`).
            if path.ends_with("app/src/alert.rs") {
                assert!(source.contains("child_window::would_hold()"));
                sites += source.matches("beginSheetModalForWindow").count();
                continue;
            }
            for (at, _) in source
                .match_indices("beginSheet")
                .chain(source.match_indices("beginCriticalSheet"))
            {
                let mut end = (at + 700).min(source.len());
                while !source.is_char_boundary(end) {
                    end -= 1;
                }
                let mut start = at.saturating_sub(600);
                while !source.is_char_boundary(start) {
                    start += 1;
                }
                let around = &source[start..end];
                assert!(
                    around.contains("child_window::sheet(") || around.contains("SHEET_GATE.get()"),
                    "{} begins a sheet outside child_window::sheet",
                    path.display()
                );
                sites += 1;
            }
        }
    }
    assert!(sites >= 8, "the scan found the app's sheets ({sites})");

    // Nothing orders a window front directly: every show goes through
    // child_window (active-only for child windows, behind other apps while
    // the app is inactive for the rest).
    for (name, source) in [
        (
            "gpui_macos window",
            include_str!("../../../vendor/gpui-ce/crates/gpui_macos/src/window.rs"),
        ),
        ("shell native", include_str!("../src/native.rs")),
        ("app builtins", include_str!("../../app/src/builtins.rs")),
        ("app session", include_str!("../../app/src/session.rs")),
        (
            "app surface routes",
            include_str!("../../app/src/surface_routes.rs"),
        ),
        (
            "app viewer popout",
            include_str!("../../app/src/viewer2.rs"),
        ),
        (
            "app colour panel",
            include_str!("../../app/src/colour_panel.rs"),
        ),
        (
            "agent visual reply viewer",
            include_str!("../../compat-host/src/visual.rs"),
        ),
    ] {
        for banned in [
            "makeKeyAndOrderFront",
            "orderFront:",
            "orderFront(",
            "orderFrontRegardless",
        ] {
            assert!(
                !source.contains(banned),
                "{name} orders a window front with {banned}"
            );
        }
    }

    // The creation sites use the policy and nothing sets a global level.
    let sources = [
        (
            "gpui_macos window",
            include_str!("../../../vendor/gpui-ce/crates/gpui_macos/src/window.rs"),
        ),
        ("shell native", include_str!("../src/native.rs")),
        (
            "app colour panel",
            include_str!("../../app/src/colour_panel.rs"),
        ),
    ];
    for (name, source) in sources {
        for banned in [
            "NSPopUpWindowLevel",
            "NSFloatingWindowLevel",
            "NSStatusWindowLevel",
            "CanJoinAllSpaces",
            "setHidesOnDeactivate(false",
        ] {
            assert!(!source.contains(banned), "{name} uses {banned}");
        }
        assert!(
            source.contains("child_window::apply"),
            "{name} applies the policy"
        );
        assert!(
            source.contains("child_window::show(") || source.contains("child_window::present("),
            "{name} shows only while the app is active"
        );
    }
    println!(
        "child_windows: {checked} window kinds, the inactive-app queue, {sites} sheet sites and {} creation sites keep the policy",
        sources.len()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
