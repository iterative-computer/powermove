//! Powermove P-CHILD: an app's transient and panel windows (popovers, the
//! command palette and field menu, the Agent window) belong to their app.
//! They sit at the normal level, above their own editor window as its child
//! window and nowhere else, never on every Space, and hide whenever the app
//! isn't active. A pop-up or floating level is global across applications: a
//! window at it floats over every other app, and an app that was never active
//! never deactivates, so it stayed there.
use objc2_app_kit::{
    NSNormalWindowLevel, NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode,
};

/// The window properties every child window gets when it's made.
pub fn apply(window: &NSWindow) {
    window.setLevel(NSNormalWindowLevel);
    window.setHidesOnDeactivate(true);
    // Over its own window in full screen; never joining every Space.
    window.setCollectionBehavior(NSWindowCollectionBehavior::FullScreenAuxiliary);
}

/// Orders `window` just above `parent`, moving with it. AppKit keeps a child
/// window directly over its parent, so other apps' windows above the parent
/// stay above the child.
pub fn attach(window: &NSWindow, parent: &NSWindow) {
    if window
        .parentWindow()
        .is_some_and(|p| std::ptr::eq(&*p, parent))
    {
        return;
    }
    unsafe { parent.addChildWindow_ordered(window, NSWindowOrderingMode::Above) };
}

/// Whether `window` keeps the policy: its level, hiding and Spaces.
pub fn follows_policy(window: &NSWindow) -> bool {
    window.level() == NSNormalWindowLevel
        && window.hidesOnDeactivate()
        && !window.collectionBehavior().intersects(
            NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary,
        )
}
