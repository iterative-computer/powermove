//! Powermove P-CHILD: an app's transient and panel windows (popovers, the
//! command palette and field menu, the Agent window) belong to their app.
//! They sit at the normal level, above their own editor window as its child
//! window and nowhere else, never on every Space, and hide whenever the app
//! isn't active. A pop-up or floating level is global across applications: a
//! window at it floats over every other app, and an app that was never active
//! never deactivates, so it stayed there.
use objc2::{MainThreadMarker, rc::Weak};
use objc2_app_kit::{
    NSApplication, NSApplicationDidBecomeActiveNotification, NSNormalWindowLevel, NSWindow,
    NSWindowCollectionBehavior, NSWindowOrderingMode,
};
use objc2_foundation::{NSNotification, NSNotificationCenter};
use std::{cell::RefCell, ptr::NonNull};

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
/// A shared window (the colour panel) moves to whichever window asked last.
/// Attaching resets the child's hiding and Spaces behaviour, so the policy is
/// applied again after.
pub fn attach(window: &NSWindow, parent: &NSWindow) {
    if let Some(old) = window.parentWindow() {
        if std::ptr::eq(&*old, parent) {
            apply(window);
            return;
        }
        old.removeChildWindow(window);
    }
    unsafe { parent.addChildWindow_ordered(window, NSWindowOrderingMode::Above) };
    apply(window);
}

/// A child window waiting for its app to become active, and the window it
/// belongs to: if that one closed or hid meanwhile, the child stays hidden.
struct Pending {
    window: Weak<NSWindow>,
    parent: Option<Weak<NSWindow>>,
    key: bool,
}

thread_local! {
    /// Child windows asked to show while the app was inactive (`key`: make it
    /// key too), and the observer that shows them once the app is active.
    static PENDING: RefCell<Vec<Pending>> = const { RefCell::new(Vec::new()) };
    static OBSERVER: RefCell<Option<objc2::rc::Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>> =
        const { RefCell::new(None) };
}

/// Shows a child window only while its app is active. A window an inactive
/// app orders front lands above other apps' windows (hiding on deactivation
/// can't help: nothing deactivates), so it waits for the app to become active
/// and shows then. Once shown, AppKit hides it when the app deactivates and
/// brings it back when it returns.
pub fn show(window: &NSWindow, key: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if NSApplication::sharedApplication(mtm).isActive() {
        order_front(window, key);
        return;
    }
    forget(window);
    PENDING.with(|p| {
        p.borrow_mut().push(Pending {
            window: Weak::from(window),
            parent: window.parentWindow().map(|parent| Weak::from(&*parent)),
            key,
        })
    });
    OBSERVER.with(|o| {
        if o.borrow().is_some() {
            return;
        }
        let block = block2::RcBlock::new(|_: NonNull<NSNotification>| show_pending());
        let token = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(NSApplicationDidBecomeActiveNotification),
                None,
                None,
                &block,
            )
        };
        *o.borrow_mut() = Some(token);
    });
}

/// Whether a child window waits for its app to become active.
pub fn is_pending(window: &NSWindow) -> bool {
    PENDING.with(|p| {
        p.borrow()
            .iter()
            .any(|e| e.window.load().is_some_and(|w| std::ptr::eq(&*w, window)))
    })
}

/// A queued child window that closed or hid: it no longer shows on activation.
pub fn forget(window: &NSWindow) {
    PENDING.with(|p| {
        p.borrow_mut()
            .retain(|e| e.window.load().is_some_and(|w| !std::ptr::eq(&*w, window)))
    });
}

/// The app became active: the child windows that waited show now.
pub fn show_pending() {
    let waiting = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for entry in waiting {
        let Some(window) = entry.window.load() else {
            continue;
        };
        // Its window closed or hid while it waited: it isn't wanted now.
        let parent_gone = entry
            .parent
            .is_some_and(|p| p.load().is_none_or(|p| !p.isVisible()));
        if !parent_gone {
            order_front(&window, entry.key);
        }
    }
}

/// Shows any of the app's windows the way a Mac app may while another app is
/// in front: a child window waits for activation ([`show`]); any other window
/// comes forward only while the app is active, and otherwise appears behind
/// the other apps' windows, where activating the app finds it.
pub fn present(window: &NSWindow, key: bool) {
    if window.hidesOnDeactivate() {
        show(window, key);
    } else if MainThreadMarker::new()
        .is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
    {
        order_front(window, key);
    } else {
        window.orderBack(None);
    }
}

/// Brings one of the app's own windows forward (focus back to the editor
/// after a popover closes, an already-open project), only while the app is
/// active: an inactive app's window ordered front lands above other apps.
pub fn raise_if_active(window: &NSWindow) {
    if MainThreadMarker::new().is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive()) {
        window.makeKeyAndOrderFront(None);
    }
}

fn order_front(window: &NSWindow, key: bool) {
    if key {
        window.makeKeyAndOrderFront(None);
    } else {
        window.orderFront(None);
    }
}

/// Whether `window` keeps the policy: its level, hiding and Spaces.
pub fn follows_policy(window: &NSWindow) -> bool {
    window.level() == NSNormalWindowLevel
        && window.hidesOnDeactivate()
        && !window.collectionBehavior().intersects(
            NSWindowCollectionBehavior::CanJoinAllSpaces | NSWindowCollectionBehavior::Stationary,
        )
}
