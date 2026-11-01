//! Powermove P-LAYER/P-HIT: main-thread slot hooks at the Metal presentation boundary.
//! See vendor POWERMOVE.md. The host owns and retains its AppKit slot views.
use objc2::{MainThreadMarker, class, msg_send};
use objc2_app_kit::NSView;
use objc2_foundation::NSPoint;
use std::{cell::RefCell, collections::HashMap, rc::Rc};

#[derive(Clone)]
struct Hooks {
    present: Rc<dyn Fn()>,
    accepts: Rc<dyn Fn(NSPoint) -> bool>,
}
thread_local! { static HOSTS: RefCell<HashMap<usize, Hooks>> = RefCell::new(HashMap::new()); }

/// RAII registration; drop before destroying the slot host. Main-thread only.
pub struct CompositionRegistration {
    key: usize,
    _main_thread: MainThreadMarker,
}
impl Drop for CompositionRegistration {
    fn drop(&mut self) {
        HOSTS.with(|h| {
            h.borrow_mut().remove(&self.key);
        });
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
        h.borrow_mut().insert(key, Hooks { present, accepts });
    });
    CompositionRegistration {
        key,
        _main_thread: mtm,
    }
}
pub(crate) fn accepts(view: usize, point: NSPoint) -> bool {
    let hooks = HOSTS.with(|h| h.borrow().get(&view).cloned());
    hooks.is_none_or(|h| (h.accepts)(point))
}
/// Explicit CATransaction also covers GPUI's transactional drawable present.
pub(crate) fn present(view: usize) -> Option<Transaction> {
    let hooks = HOSTS.with(|h| h.borrow().get(&view).cloned())?;
    unsafe {
        let _: () = msg_send![class!(CATransaction), begin];
        let _: () = msg_send![class!(CATransaction), setDisableActions: true];
    }
    let transaction = Transaction;
    (hooks.present)();
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
