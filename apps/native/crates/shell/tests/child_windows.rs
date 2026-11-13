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
        apply, attach, follows_policy, forget, forget_sheet, is_pending, sheet, sheet_pending,
        show, show_pending,
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

    // A sheet asked for while the app is inactive: nothing begins, no window
    // moves, it waits; forgotten or with its window closed, it is dropped
    // (what waits on its answer sees it cancelled); with its window open it
    // begins once the app is active.
    struct Dropped(Rc<Cell<bool>>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }
    let on = window(NSWindowStyleMask::Titled);
    let asked = |sheet_window: &NSWindow| {
        let (began, dropped) = (Rc::new(Cell::new(false)), Rc::new(Cell::new(false)));
        let (b, guard) = (began.clone(), Dropped(dropped.clone()));
        let (parent, shown) = (on.clone(), sheet_window.retain());
        let now = sheet(&on, sheet_window, move || {
            let _keep = &guard;
            parent.beginSheet_completionHandler(&shown, None);
            b.set(true);
        });
        assert!(!now, "an inactive app begins no sheet");
        (began, dropped)
    };
    let sheet_window = window(NSWindowStyleMask::Titled);
    let (began, dropped) = asked(&sheet_window);
    assert!(sheet_pending(&on) && !began.get() && !dropped.get());
    assert!(on.attachedSheet().is_none() && sheet_window.sheetParent().is_none());
    assert!(
        !on.isVisible() && !sheet_window.isVisible(),
        "no window moved"
    );
    forget_sheet(&sheet_window);
    assert!(
        !sheet_pending(&on) && dropped.get() && !began.get(),
        "forgotten: dropped"
    );
    let (began, dropped) = asked(&sheet_window);
    show_pending();
    assert!(
        !sheet_pending(&on) && dropped.get() && !began.get(),
        "its window is not open: dropped on activation"
    );
    on.orderBack(None);
    let (began, _) = asked(&sheet_window);
    assert!(on.attachedSheet().is_none(), "still waiting while inactive");
    show_pending();
    assert!(
        began.get() && !sheet_pending(&on),
        "begins once the app is active"
    );
    assert!(
        on.attachedSheet()
            .is_some_and(|s| std::ptr::eq(&*s, &*sheet_window)),
        "on the window that asked"
    );
    on.endSheet(&sheet_window);
    on.orderOut(None);

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
            for (at, _) in source
                .match_indices("beginSheet")
                .chain(source.match_indices("beginCriticalSheet"))
            {
                let mut end = (at + 300).min(source.len());
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
