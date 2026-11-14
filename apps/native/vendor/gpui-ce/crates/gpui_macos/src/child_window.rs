//! Powermove P-CHILD: an app's transient and panel windows (popovers, the
//! command palette and field menu, the Agent window) belong to their app.
//! They sit at the normal level, above their own editor window as its child
//! window and nowhere else, never on every Space, and hide whenever the app
//! isn't active. A pop-up or floating level is global across applications: a
//! window at it floats over every other app, and an app that was never active
//! never deactivates, so it stayed there.
//!
//! Sheets go through [`sheet`]. A normal launch begins them at once, exactly
//! as AppKit does. A development lane launch ([`hold_sheets_while_inactive`])
//! holds them while the app is inactive: `beginSheet` orders the parent
//! window front, which put a lane window under the owner's front window and
//! let macOS activate the lane app when his front app left.
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
/// AppKit orders a hidden child in at once when its parent is on screen, so
/// an inactive app's child would show before [`show`] lets it, and hiding it
/// on deactivation took its parent (the editor window) off screen with it.
/// Such a child waits unattached and joins its parent when it shows.
pub fn attach(window: &NSWindow, parent: &NSWindow) {
    if let Some(old) = window.parentWindow() {
        if std::ptr::eq(&*old, parent) {
            apply(window);
            return;
        }
        old.removeChildWindow(window);
    }
    let inactive = MainThreadMarker::new()
        .is_some_and(|mtm| !NSApplication::sharedApplication(mtm).isActive());
    if inactive && parent.isVisible() && !window.isVisible() {
        JOINS.with(|j| {
            let mut joins = j.borrow_mut();
            joins.retain(|(w, _)| w.load().is_some_and(|w| !std::ptr::eq(&*w, window)));
            joins.push((Weak::from(window), Weak::from(parent)));
        });
        apply(window);
        return;
    }
    unsafe { parent.addChildWindow_ordered(window, NSWindowOrderingMode::Above) };
    apply(window);
}

/// The parent a child waiting unattached ([`attach`]) joins when it shows.
fn waiting_parent(window: &NSWindow) -> Option<objc2::rc::Retained<NSWindow>> {
    JOINS.with(|j| {
        j.borrow()
            .iter()
            .find(|(w, _)| w.load().is_some_and(|w| std::ptr::eq(&*w, window)))
            .and_then(|(_, p)| p.load())
    })
}

/// A child that waited unattached joins its parent, now that it shows.
fn join(window: &NSWindow) {
    let parent = JOINS.with(|j| {
        let mut joins = j.borrow_mut();
        let at = joins
            .iter()
            .position(|(w, _)| w.load().is_some_and(|w| std::ptr::eq(&*w, window)))?;
        joins.remove(at).1.load()
    });
    if let Some(parent) = parent
        && window.parentWindow().is_none()
    {
        unsafe { parent.addChildWindow_ordered(window, NSWindowOrderingMode::Above) };
        apply(window);
    }
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
    /// Child windows asked to attach while the app was inactive, and their parents.
    static JOINS: RefCell<Vec<(Weak<NSWindow>, Weak<NSWindow>)>> = const { RefCell::new(Vec::new()) };
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
    let parent = window.parentWindow().or_else(|| waiting_parent(window));
    PENDING.with(|p| {
        p.borrow_mut().push(Pending {
            window: Weak::from(window),
            parent: parent.as_ref().map(|parent| Weak::from(&**parent)),
            key,
        })
    });
    observe_activation();
    #[cfg(debug_assertions)]
    if let Some(parent) = parent {
        review_show(window, &parent);
    }
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
    #[cfg(debug_assertions)]
    held::begin_all();
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
    join(window);
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

/// Begins a sheet: `begin` makes the AppKit call (`beginSheet`, an alert's
/// or a file panel's `beginSheetModalForWindow`) that puts `sheet` on
/// `parent`. In a normal launch that happens at once, as AppKit does, and
/// `cancel` is dropped unused. In a development lane launch
/// ([`hold_sheets_while_inactive`]) a sheet asked for while the app is
/// inactive waits, attached to nothing and with no window moved, and begins
/// when the app becomes active (or its minimised window comes back); if it
/// never begins (forgotten by its owner, its window closed) `cancel` runs,
/// so whatever waits on its answer hears that it was cancelled. Returns
/// whether it began now.
pub fn sheet(
    parent: &NSWindow,
    sheet: &NSWindow,
    begin: impl FnOnce() + 'static,
    cancel: impl FnOnce() + 'static,
) -> bool {
    #[cfg(debug_assertions)]
    if would_hold() {
        eprintln!("powermove: a sheet waits for the app to become active (lane launch)");
        held::push(parent, sheet, Box::new(begin), Box::new(cancel));
        return false;
    }
    let _ = (parent, sheet, cancel);
    begin();
    true
}

/// Development lane launches only: from now on, a sheet asked for while the
/// app is inactive waits for it to become active (see [`sheet`]). Set once,
/// in process, by the development launch flag the lane launcher passes;
/// never by the environment. Release builds have no such mode.
#[cfg(debug_assertions)]
pub fn hold_sheets_while_inactive(stage_for_review: bool) {
    held::MODE.with(|m| m.set(if stage_for_review { 2 } else { 1 }));
}

/// Whether a sheet asked for now would wait (a lane launch, app inactive).
/// An alert answers itself instead in that case: a question nobody can see
/// takes its Escape choice, so nothing (a quit) waits on it.
pub fn would_hold() -> bool {
    #[cfg(debug_assertions)]
    {
        held::MODE.with(|m| m.get()) > 0
            && MainThreadMarker::new()
                .is_some_and(|mtm| !NSApplication::sharedApplication(mtm).isActive())
    }
    #[cfg(not(debug_assertions))]
    false
}

/// Whether a sheet waits to go on `parent` (a view that shows one sheet at a
/// time asks this as well as `attachedSheet`). Always false in a normal launch.
pub fn sheet_pending(parent: &NSWindow) -> bool {
    #[cfg(debug_assertions)]
    {
        held::pending_on(parent)
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = parent;
        false
    }
}

/// The owner closed or dismissed a sheet before it began: its waiting entry
/// goes and its `cancel` runs. Nothing in a normal launch.
pub fn forget_sheet(sheet: &NSWindow) {
    #[cfg(debug_assertions)]
    held::forget(|h| h.sheet_is(sheet));
    #[cfg(not(debug_assertions))]
    let _ = sheet;
}

/// A window is going away: every sheet waiting to go on it is cancelled.
pub fn forget_sheets_on(parent: &NSWindow) {
    #[cfg(debug_assertions)]
    held::forget(|h| h.parent_is(parent));
    #[cfg(not(debug_assertions))]
    let _ = parent;
}

/// Whether a window's return from the Dock wakes the sheets waiting on it
/// (development lane launches; registered with the first waiting sheet).
#[cfg(debug_assertions)]
pub fn restore_observed() -> bool {
    held::RESTORE.with(|o| o.borrow().is_some())
}

/// The development lane's waiting sheets.
#[cfg(debug_assertions)]
mod held {
    use super::*;
    use objc2::rc::Retained;
    use objc2_app_kit::NSWindowDidDeminiaturizeNotification;
    use std::cell::Cell;

    thread_local! {
        /// 0: a normal launch; 1: hold while inactive; 2: and stand a
        /// waiting sheet above its window for review captures.
        pub(super) static MODE: Cell<u8> = const { Cell::new(0) };
        static SHEETS: RefCell<Vec<Held>> = const { RefCell::new(Vec::new()) };
        pub(super) static RESTORE: RefCell<Option<Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>> =
            const { RefCell::new(None) };
    }

    pub(super) struct Held {
        parent: Weak<NSWindow>,
        sheet: Weak<NSWindow>,
        begin: Option<Box<dyn FnOnce()>>,
        cancel: Option<Box<dyn FnOnce()>>,
    }
    impl Held {
        pub(super) fn sheet_is(&self, w: &NSWindow) -> bool {
            self.sheet.load().is_none_or(|s| std::ptr::eq(&*s, w))
        }
        pub(super) fn parent_is(&self, w: &NSWindow) -> bool {
            self.parent.load().is_none_or(|p| std::ptr::eq(&*p, w))
        }
    }
    /// Dropped without beginning: its owner hears it was cancelled.
    impl Drop for Held {
        fn drop(&mut self) {
            if self.begin.is_some()
                && let Some(cancel) = self.cancel.take()
            {
                cancel();
            }
        }
    }

    pub(super) fn push(
        parent: &NSWindow,
        sheet: &NSWindow,
        begin: Box<dyn FnOnce()>,
        cancel: Box<dyn FnOnce()>,
    ) {
        forget(|h| h.sheet_is(sheet));
        SHEETS.with(|s| {
            s.borrow_mut().push(Held {
                parent: Weak::from(parent),
                sheet: Weak::from(sheet),
                begin: Some(begin),
                cancel: Some(cancel),
            })
        });
        observe_activation();
        observe_restore();
        if MODE.with(|m| m.get()) == 2 {
            stage(sheet, parent);
        }
    }

    pub(super) fn pending_on(parent: &NSWindow) -> bool {
        SHEETS.with(|s| {
            s.borrow()
                .iter()
                .any(|h| h.parent.load().is_some_and(|p| std::ptr::eq(&*p, parent)))
        })
    }

    /// Removes the entries `which` matches; each one's `cancel` runs after
    /// the queue is released (a cancelled answer may ask for another sheet).
    pub(super) fn forget(which: impl Fn(&Held) -> bool) {
        let gone: Vec<Held> = SHEETS.with(|s| {
            let mut s = s.borrow_mut();
            let (gone, kept) = std::mem::take(&mut *s).into_iter().partition(|h| which(h));
            *s = kept;
            gone
        });
        for h in &gone {
            if let Some(w) = h.sheet.load() {
                take_down(&w);
            }
        }
        drop(gone);
    }

    /// The app became active: every waiting sheet whose window is open
    /// begins, oldest first; one whose window is in the Dock waits for it
    /// to come back; one whose window closed is cancelled.
    pub(super) fn begin_all() {
        let held = SHEETS.with(|s| std::mem::take(&mut *s.borrow_mut()));
        let mut still = Vec::new();
        for mut entry in held {
            let (Some(parent), Some(sheet)) = (entry.parent.load(), entry.sheet.load()) else {
                continue;
            };
            if parent.isVisible() {
                take_down(&sheet);
                if let Some(begin) = entry.begin.take() {
                    begin();
                }
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

    /// A window comes back from the Dock while the app is active: the sheets
    /// waiting on it begin.
    fn observe_restore() {
        RESTORE.with(|o| {
            if o.borrow().is_some() {
                return;
            }
            let block = block2::RcBlock::new(|_: NonNull<NSNotification>| {
                if MainThreadMarker::new()
                    .is_some_and(|mtm| NSApplication::sharedApplication(mtm).isActive())
                {
                    begin_all();
                }
            });
            let token = unsafe {
                NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                    Some(NSWindowDidDeminiaturizeNotification),
                    None,
                    None,
                    &block,
                )
            };
            *o.borrow_mut() = Some(token);
        });
    }

    /// Review captures (the launch's `--open`): a waiting sheet stands where
    /// it will hang, at the top of its window's content, ordered just above
    /// that window and never front, so a background capture sees it.
    fn stage(sheet: &NSWindow, parent: &NSWindow) {
        if !parent.isVisible() {
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

    /// Takes down a staged sheet before it begins or when it is cancelled.
    fn take_down(sheet: &NSWindow) {
        if sheet.isVisible() && sheet.sheetParent().is_none() {
            sheet.orderOut(None);
        }
    }
}
