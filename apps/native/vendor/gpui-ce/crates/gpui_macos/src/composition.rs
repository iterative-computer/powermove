//! Powermove P-LAYER/P-HIT: main-thread slot hooks at the Metal presentation boundary.
//! See vendor POWERMOVE.md. The host owns and retains its AppKit slot views.
//!
//! P-120: native geometry changes and AppKit synchronous paints remain atomic
//! with GPUI in a CATransaction. Compositor-paced frames can present without
//! waiting on main: CAMetalDisplayLink supplies a drawable for the current update.
//! The legacy CVDisplayLink fallback still couples continuous frames, because its
//! unpaced async nextDrawable path exhausted the three-drawable pool on screen.
use objc2::{MainThreadMarker, class, msg_send, rc::Retained};
use objc2_app_kit::NSView;
use objc2_foundation::NSPoint;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

/// What a host's present callback did to native slot geometry this frame.
///
/// `Moved` covers any frame, clip/mask, transform, visibility or z-order change of
/// a native slot that must land on glass together with the GPUI pixels around it.
/// Property changes that do not move a slot edge (layer contents, colors, text)
/// are `Unchanged`: they still commit in the same explicit `CATransaction`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotChange {
    Unchanged,
    Moved,
}
impl SlotChange {
    pub fn moved(self) -> bool {
        self == SlotChange::Moved
    }
    /// `Moved` if either side moved.
    pub fn or(self, other: SlotChange) -> SlotChange {
        if self.moved() || other.moved() {
            SlotChange::Moved
        } else {
            SlotChange::Unchanged
        }
    }
}
impl From<bool> for SlotChange {
    fn from(moved: bool) -> Self {
        if moved {
            SlotChange::Moved
        } else {
            SlotChange::Unchanged
        }
    }
}

struct Hooks {
    present: Rc<dyn Fn() -> SlotChange>,
    accepts: Rc<dyn Fn(NSPoint) -> bool>,
    accessibility: Vec<AccessibilityItem>,
    /// The first publication places every slot, so it is always transactional.
    fresh: bool,
}
thread_local! { static HOSTS: RefCell<HashMap<usize, Hooks>> = RefCell::new(HashMap::new()); }

/// RAII registration; drop before destroying the slot host. Main-thread only.
pub struct CompositionRegistration {
    key: usize,
    _main_thread: MainThreadMarker,
}
impl Drop for CompositionRegistration {
    fn drop(&mut self) {
        let old = HOSTS.with(|h| h.borrow_mut().remove(&self.key));
        if let Some(old) = old {
            clear_parents(&old.accessibility);
        }
        crate::composition_accessibility::changed(self.key);
    }
}
/// `present` applies the just-painted ordered slot list. `accepts` returns false
/// only for transparent GPUI regions (compat overlay holes included). AppKit
/// visits above siblings/child windows first and preserves pressed-view capture.
/// Callbacks must not re-enter GPUI window operations while its draw lock is held.
///
/// This untracked form cannot say whether geometry moved, so every frame keeps the
/// transaction-coupled (main-thread-blocking) present. Prefer [`register_tracked`].
pub fn register(
    view: &NSView,
    present: Rc<dyn Fn()>,
    accepts: Rc<dyn Fn(NSPoint) -> bool>,
) -> CompositionRegistration {
    register_tracked(view, untracked(present), accepts)
}
fn untracked(present: Rc<dyn Fn()>) -> Rc<dyn Fn() -> SlotChange> {
    Rc::new(move || {
        present();
        SlotChange::Moved
    })
}
/// Like [`register`], but `present` reports whether slot geometry moved. Frames
/// that return [`SlotChange::Unchanged`] can present without waiting when paced
/// by Core Animation. The legacy continuous-frame fallback remains transactional.
/// The first frame after registration is always transactional.
pub fn register_tracked(
    view: &NSView,
    present: Rc<dyn Fn() -> SlotChange>,
    accepts: Rc<dyn Fn(NSPoint) -> bool>,
) -> CompositionRegistration {
    let mtm = MainThreadMarker::new().expect("composition requires main thread");
    let key = view as *const NSView as usize;
    insert_hooks(key, present, accepts);
    CompositionRegistration {
        key,
        _main_thread: mtm,
    }
}
fn insert_hooks(
    key: usize,
    present: Rc<dyn Fn() -> SlotChange>,
    accepts: Rc<dyn Fn(NSPoint) -> bool>,
) {
    HOSTS.with(|h| {
        h.borrow_mut().insert(
            key,
            Hooks {
                present,
                accepts,
                accessibility: Vec::new(),
                fresh: true,
            },
        );
    });
}
pub(crate) fn accepts(view: usize, point: NSPoint) -> bool {
    let accepts = HOSTS.with(|h| h.borrow().get(&view).map(|h| h.accepts.clone()));
    accepts.is_none_or(|accepts| accepts(point))
}
/// One entry in the committed composition's reading order. Native views remain
/// AppKit objects: do not mirror their WebKit/text/control subtrees into AccessKit.
#[derive(Clone)]
pub enum AccessibilityItem {
    /// A direct, exposed child of the AccessKit root, matched by AXIdentifier.
    Gpui(String),
    /// The host retains this view until the next publication or unregister.
    Native(Retained<NSView>),
    /// All GPUI root children not explicitly named elsewhere, in tree order.
    RemainingGpui,
}
/// Publish with slot geometry in the present callback. No accessibility callback
/// enters the host (or GPUI's window lock); it reads this retained snapshot.
pub fn publish_accessibility(view: &NSView, order: Vec<AccessibilityItem>) {
    let key = view as *const NSView as usize;
    let old = HOSTS.with(|hosts| {
        let mut hosts = hosts.borrow_mut();
        let host = hosts.get_mut(&key)?;
        if same_order(&host.accessibility, &order) {
            return None;
        }
        Some(std::mem::replace(&mut host.accessibility, order.clone()))
    });
    if let Some(old) = old {
        let removed: Vec<_> = old.into_iter().filter(|entry| match entry {
            AccessibilityItem::Native(view) => !order.iter().any(|new| matches!(new, AccessibilityItem::Native(v) if std::ptr::eq(&**v, &**view))),
            _ => false,
        }).collect();
        clear_parents(&removed);
        crate::composition_accessibility::changed(key);
    }
}
pub(crate) fn accessibility(view: usize) -> Option<Vec<AccessibilityItem>> {
    let order = HOSTS.with(|hosts| hosts.borrow().get(&view).map(|h| h.accessibility.clone()));
    if order.is_none() && !crate::native_text::has_slots(view) {
        return None;
    }
    let mut order = order.unwrap_or_default();
    // Explicit native anchors win; otherwise append managed text slots once.
    for entry in crate::native_text::accessibility(view) {
        if !order.iter().any(|old|matches!((old,&entry),(AccessibilityItem::Native(a),AccessibilityItem::Native(b)) if std::ptr::eq(&**a,&**b))) {order.push(entry);}
    }
    Some(order)
}
/// Opens the explicit CATransaction for this frame's slot publication and runs the
/// host. When geometry moved, GPUI's drawable must be presented with the
/// transaction ([`Transaction::moved`]); otherwise it presents asynchronously.
pub(crate) fn present(view: usize) -> Option<Transaction> {
    let hook = HOSTS.with(|h| {
        h.borrow_mut()
            .get_mut(&view)
            .map(|h| (h.present.clone(), std::mem::take(&mut h.fresh)))
    });
    if hook.is_none() && !crate::native_text::has_slots(view) {
        return None;
    }
    unsafe {
        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions: true];
    }
    let mut transaction = Transaction {
        change: SlotChange::Unchanged,
    };
    if let Some((present, fresh)) = hook {
        let change = gpui_util::measure("native composition", || present());
        transaction.change = change.or(fresh.into());
    }
    let text = crate::native_text::present(view);
    transaction.change = transaction.change.or(text);
    Some(transaction)
}
/// A frame drawn within this long after the previous one is "continuous".
/// The legacy CVDisplayLink fallback couples frames within this interval.
pub const CONTINUOUS_FRAME: std::time::Duration = std::time::Duration::from_micros(33_334);
/// P-120 policy for one GPUI frame. `forced` is set by AppKit-driven synchronous
/// paints (live resize `displayLayer:`, tab activation) that must match the window;
/// `continuous` when the previous frame was drawn within [`CONTINUOUS_FRAME`].
/// `compositor_paced` permits continuous updates without a scheduling wait.
/// Windows without composition hosts keep upstream behaviour (async unless forced).
pub(crate) fn presents_with_transaction(
    forced: bool,
    frame: Option<&Transaction>,
    continuous: bool,
    compositor_paced: bool,
) -> bool {
    forced || frame.is_some_and(|t| t.moved() || (continuous && !compositor_paced))
}
/// Admit a display-linked update only while its CPU budget fits before the
/// presentation-submission deadline. Rejected updates leave GPUI demand queued.
pub(crate) fn has_frame_budget(now: f64, deadline: f64, budget: f64) -> bool {
    now.is_finite()
        && deadline.is_finite()
        && budget.is_finite()
        && budget > 0.
        && now + budget < deadline
}

pub(crate) struct Transaction {
    change: SlotChange,
}
impl Transaction {
    pub(crate) fn moved(&self) -> bool {
        self.change.moved()
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        unsafe {
            let _: () = msg_send![class!(CATransaction), commit];
        }
    }
}

fn same_order(a: &[AccessibilityItem], b: &[AccessibilityItem]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| match (a, b) {
            (AccessibilityItem::Gpui(a), AccessibilityItem::Gpui(b)) => a == b,
            (AccessibilityItem::Native(a), AccessibilityItem::Native(b)) => {
                std::ptr::eq(&**a, &**b)
            }
            (AccessibilityItem::RemainingGpui, AccessibilityItem::RemainingGpui) => true,
            _ => false,
        })
}
pub(crate) fn clear_parents(order: &[AccessibilityItem]) {
    for item in order {
        if let AccessibilityItem::Native(view) = item {
            let children =
                unsafe { objc2_app_kit::NSAccessibilityUnignoredChildrenForOnlyChild(view) };
            for child in children.to_vec() {
                unsafe {
                    let _: () = msg_send![&*child, setAccessibilityParent: std::ptr::null::<objc2::runtime::AnyObject>()];
                }
            }
        }
    }
}

/// Screen-free probe of the P-120 presentation policy for contract tests that
/// live outside this (non-member) crate. Uses fake keys, never an AppKit view.
#[doc(hidden)]
pub mod probe {
    use super::*;
    /// Removes the fake host on drop.
    pub struct Registration(usize);
    impl Drop for Registration {
        fn drop(&mut self) {
            HOSTS.with(|h| h.borrow_mut().remove(&self.0));
        }
    }
    pub fn register(key: usize, present: Rc<dyn Fn() -> SlotChange>) -> Registration {
        insert_hooks(key, present, Rc::new(|_| true));
        Registration(key)
    }
    /// Same wrapping as the public untracked [`super::register`].
    pub fn register_untracked(key: usize, present: Rc<dyn Fn()>) -> Registration {
        register(key, untracked(present))
    }
    /// Runs one composition frame exactly as `MacWindow::draw` does and returns
    /// whether GPUI would present with the transaction (`None`: no host).
    pub fn frame(key: usize, forced: bool) -> Option<bool> {
        frame_after(key, forced, false)
    }
    /// `continuous`: the previous frame was within [`super::CONTINUOUS_FRAME`].
    pub fn frame_after(key: usize, forced: bool, continuous: bool) -> Option<bool> {
        let transaction = present(key);
        transaction.as_ref()?;
        Some(presents_with_transaction(
            forced,
            transaction.as_ref(),
            continuous,
            false,
        ))
    }
    /// Compositor-paced frames can use async even during continuous animation.
    pub fn paced_frame(key: usize, forced: bool) -> Option<bool> {
        let transaction = present(key);
        transaction.as_ref()?;
        Some(presents_with_transaction(
            forced,
            transaction.as_ref(),
            true,
            true,
        ))
    }
    pub fn has_frame_budget(now: f64, deadline: f64, budget: f64) -> bool {
        super::has_frame_budget(now, deadline, budget)
    }
    pub fn unregistered_frame(forced: bool) -> bool {
        presents_with_transaction(forced, None, true, false)
    }
}
