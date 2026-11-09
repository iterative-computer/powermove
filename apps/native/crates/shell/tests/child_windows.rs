//! P-CHILD: every child window (GPUI pop-ups and floating windows such as the
//! Agent popover, the command palette and field menu, the colour panel) sits at
//! the normal level, hides while the app is inactive and never joins every
//! Space, so none can float over other applications. Runs on the main thread
//! with activation prohibited and never orders a window on screen.
#[cfg(target_os = "macos")]
fn main() {
    use gpui_macos::child_window::{apply, follows_policy};
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColorPanel, NSPanel,
        NSPopUpMenuWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowStyleMask,
    };
    use objc2_foundation::{NSPoint, NSRect, NSSize};

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
    }
    println!(
        "child_windows: {checked} window kinds and {} creation sites keep the policy",
        sources.len()
    );
}
#[cfg(not(target_os = "macos"))]
fn main() {}
