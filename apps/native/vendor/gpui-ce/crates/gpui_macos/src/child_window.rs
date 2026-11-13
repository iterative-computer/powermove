//! Powermove P-CHILD: an app's transient and panel windows (popovers, the
//! command palette and field menu, the Agent window) belong to their app.
//! They sit at the normal level, above their own editor window as its child
//! window and nowhere else, never on every Space, and hide whenever the app
//! isn't active. A pop-up or floating level is global across applications: a
//! window at it floats over every other app, and an app that was never active
//! never deactivates, so it stayed there.
//!
//! Sheets follow the same rule ([`sheet`]): AppKit's `beginSheet` orders the
//! parent window front, so an inactive app's window rose above every other
//! app's but the front one, and macOS then activated the app as soon as the
//! front app left.
use objc2::{MainThreadMarker, rc::Weak};
use objc2_app_kit::{
    NSApplication, NSApplicationDidBecomeActiveNotification, NSNormalWindowLevel,
    NSRequestUserAttentionType, NSWindow, NSWindowCollectionBehavior, NSWindowOrderingMode,
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
    /// Sheets asked for while the app was inactive ([`sheet`]).
    static SHEETS: RefCell<Vec<HeldSheet>> = const { RefCell::new(Vec::new()) };
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
    observe_activation();
}

/// Calls [`show_pending`] whenever the app becomes active.
fn observe_activation() {
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

/// A sheet waiting for its app to become active: the window it goes on, the
/// sheet's own window, and the call that begins it.
struct HeldSheet {
    parent: Weak<NSWindow>,
    sheet: Weak<NSWindow>,
    begin: Box<dyn FnOnce()>,
}

/// Begins a sheet only while its app is active; `begin` makes the AppKit
/// call (`beginSheet`, an alert's or a save panel's
/// `beginSheetModalForWindow`) that puts `sheet` on `parent`.
///
/// `beginSheet` orders its parent window front. An inactive app's window
/// ordered front lands right under the other app's front window, and macOS
/// activates the app as soon as that app leaves the front. So a sheet asked
/// for while the app is inactive waits, attached to nothing and with no
/// window moved, and begins when the person brings the app forward; the
/// Dock icon bounces once to say a question waits. If its parent closed
/// meanwhile, or the sheet window went away, `begin` is dropped: whatever
/// waits on the sheet's answer sees it cancelled. Returns whether it began
/// now.
pub fn sheet(parent: &NSWindow, sheet: &NSWindow, begin: impl FnOnce() + 'static) -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let app = NSApplication::sharedApplication(mtm);
    if app.isActive() {
        begin();
        return true;
    }
    forget_sheet(sheet);
    SHEETS.with(|s| {
        s.borrow_mut().push(HeldSheet {
            parent: Weak::from(parent),
            sheet: Weak::from(sheet),
            begin: Box::new(begin),
        })
    });
    app.requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
    observe_activation();
    #[cfg(debug_assertions)]
    {
        eprintln!("powermove: a sheet waits for the app to become active");
        review_sheet(sheet, parent);
    }
    false
}

/// Whether a sheet waits for the app to become active before it goes on
/// `parent` (a view that shows one sheet at a time asks this as well as
/// `attachedSheet`).
pub fn sheet_pending(parent: &NSWindow) -> bool {
    SHEETS.with(|s| {
        s.borrow()
            .iter()
            .any(|h| h.parent.load().is_some_and(|p| std::ptr::eq(&*p, parent)))
    })
}

/// Drops a waiting sheet (its owner closed or dismissed it before it began);
/// dropping its `begin` cancels whatever waits on its answer.
pub fn forget_sheet(sheet: &NSWindow) {
    let dropped: Vec<HeldSheet> = SHEETS.with(|s| {
        let mut s = s.borrow_mut();
        let (gone, kept) = std::mem::take(&mut *s)
            .into_iter()
            .partition(|h| h.sheet.load().is_none_or(|w| std::ptr::eq(&*w, sheet)));
        *s = kept;
        gone
    });
    for held in &dropped {
        if let Some(window) = held.sheet.load() {
            take_down(&window);
        }
    }
    // Dropped outside the borrow: a dropped answer may run code that asks
    // for another sheet.
    drop(dropped);
}

/// Begins the sheets that waited for the app to become active, oldest first.
/// One whose parent closed or hid is dropped; one whose parent is in the
/// Dock waits on.
fn begin_held_sheets() {
    let held = SHEETS.with(|s| std::mem::take(&mut *s.borrow_mut()));
    let mut still = Vec::new();
    for entry in held {
        let (Some(parent), Some(sheet)) = (entry.parent.load(), entry.sheet.load()) else {
            continue;
        };
        take_down(&sheet);
        if parent.isVisible() {
            (entry.begin)();
        } else if parent.isMiniaturized() {
            still.push(entry);
        }
    }
    if !still.is_empty() {
        SHEETS.with(|s| {
            let mut s = s.borrow_mut();
            still.append(&mut s);
            *s = still;
        });
    }
}

/// A development build launched for a review (`PM_NATIVE_REVIEW_OPEN`)
/// stands a child window just above its own parent while the app is in the
/// background, so a capture can see it. It is ordered relative to its
/// parent, never front, so it cannot come above another app's window.
#[cfg(debug_assertions)]
pub fn review_show(window: &NSWindow, parent: &NSWindow) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    if NSApplication::sharedApplication(mtm).isActive()
        || std::env::var_os("PM_NATIVE_REVIEW_OPEN").is_none()
    {
        return;
    }
    window.setHidesOnDeactivate(false);
    window.orderWindow_relativeTo(NSWindowOrderingMode::Above, parent.windowNumber());
}

/// A development build launched for a review (`PM_NATIVE_REVIEW_OPEN`)
/// stands a waiting sheet where it will hang, at the top of its window's
/// content, ordered just above that window and never front, so a capture can
/// see it while the app stays in the background. It is taken down before it
/// begins or when it is dropped.
#[cfg(debug_assertions)]
fn review_sheet(sheet: &NSWindow, parent: &NSWindow) {
    if std::env::var_os("PM_NATIVE_REVIEW_OPEN").is_none() || !parent.isVisible() {
        return;
    }
    let (frame, own) = (parent.frame(), sheet.frame());
    let top = frame.origin.y + parent.contentLayoutRect().size.height;
    sheet.setFrameOrigin(objc2_foundation::NSPoint::new(
        frame.origin.x + (frame.size.width - own.size.width) / 2.,
        top - own.size.height,
    ));
    sheet.orderWindow_relativeTo(NSWindowOrderingMode::Above, parent.windowNumber());
}

/// Takes down a waiting sheet a review stood up ([`review_sheet`]).
fn take_down(sheet: &NSWindow) {
    if sheet.isVisible() && sheet.sheetParent().is_none() {
        sheet.orderOut(None);
    }
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

/// The app became active: the child windows and then the sheets that waited
/// show now.
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
    begin_held_sheets();
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
