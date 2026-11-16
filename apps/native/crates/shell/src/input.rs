//! The person's input, for whoever waits for them to be idle (the app's
//! background render): the editor reports every press here.
use std::sync::OnceLock;

static HOOK: OnceLock<fn()> = OnceLock::new();

/// Called on every press in an editor window (set once by the app).
pub fn on_input(hook: fn()) {
    let _ = HOOK.set(hook);
}
pub(crate) fn noted() {
    if let Some(hook) = HOOK.get() {
        hook();
    }
}
