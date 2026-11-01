//! Powermove P-LAYER/P-HIT: main-thread slot hooks at the Metal presentation boundary.
//! See vendor POWERMOVE.md. The host owns and retains its AppKit slot views.
use objc2::{MainThreadMarker, class, msg_send, rc::Retained};
use objc2_app_kit::NSView;
use objc2_foundation::NSPoint;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

struct Hooks {
    present: Rc<dyn Fn()>,
    accepts: Rc<dyn Fn(NSPoint) -> bool>,
    accessibility: Vec<AccessibilityItem>,
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
pub fn register(
    view: &NSView,
    present: Rc<dyn Fn()>,
    accepts: Rc<dyn Fn(NSPoint) -> bool>,
) -> CompositionRegistration {
    let mtm = MainThreadMarker::new().expect("composition requires main thread");
    let key = view as *const NSView as usize;
    HOSTS.with(|h| {
        h.borrow_mut().insert(
            key,
            Hooks {
                present,
                accepts,
                accessibility: Vec::new(),
            },
        );
    });
    CompositionRegistration {
        key,
        _main_thread: mtm,
    }
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
    HOSTS.with(|hosts| hosts.borrow().get(&view).map(|h| h.accessibility.clone()))
}
/// Explicit CATransaction also covers GPUI's transactional drawable present.
pub(crate) fn present(view: usize) -> Option<Transaction> {
    let present = HOSTS.with(|h| h.borrow().get(&view).map(|h| h.present.clone()))?;
    unsafe {
        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions: true];
    }
    let transaction = Transaction;
    present();
    Some(transaction)
}
pub(crate) struct Transaction;
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
