//! Hosts to wake when work that ends off their thread changes what a panel
//! shows: the speech model store, the Export sheet's encoder checks.
use std::sync::{Arc, Mutex, Weak};

pub(crate) type Wake = dyn Fn() + Send + Sync;
static WAKERS: Mutex<Vec<Weak<Wake>>> = Mutex::new(Vec::new());

/// Wakes this host from now on, while it lives.
pub(crate) fn watch(wake: &Arc<Wake>) {
    WAKERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(Arc::downgrade(wake));
}
/// Every live host publishes again.
pub(crate) fn wake_all() {
    let mut wakers = WAKERS.lock().unwrap_or_else(|e| e.into_inner());
    wakers.retain(|w| w.strong_count() > 0);
    for wake in wakers.iter().filter_map(Weak::upgrade) {
        wake();
    }
}
