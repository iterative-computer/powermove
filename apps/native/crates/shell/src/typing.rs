//! PM-402: letters typed right after the person opens a text field (the
//! Project panel's search button) belong to that field, never to shortcuts.
//! The field takes the keyboard a frame or two later than the click: until it
//! does, the window's key monitor holds every printable key ([`hold`]) and
//! hands them to the field, in order, once it has the keyboard. Keys held
//! when the field never comes are dropped: they were typed for it.
use std::{
    cell::{Cell, RefCell},
    time::{Duration, Instant},
};

thread_local! {
    static EXPECTED: Cell<Option<Instant>> = const { Cell::new(None) };
    #[cfg(target_os = "macos")]
    static HELD: RefCell<Vec<objc2::rc::Retained<objc2_app_kit::NSEvent>>> = const { RefCell::new(Vec::new()) };
}

/// How long typing waits for a field the person just opened.
pub const WAIT: Duration = Duration::from_millis(1500);

/// The person just opened a text field: typing goes to it from now on.
pub fn expect() {
    EXPECTED.with(|e| e.set(Some(Instant::now() + WAIT)));
}

/// Whether a field the person opened is still on its way.
pub fn expecting() -> bool {
    EXPECTED.with(|e| e.get().is_some_and(|until| Instant::now() < until))
}

/// Whether `chars` (what a key types, modifiers aside) is text: a printable
/// key without Command, Control or Function.
pub fn is_text(chars: &str, command_or_control: bool) -> bool {
    !command_or_control && !chars.is_empty() && !chars.chars().any(char::is_control)
        && !chars.chars().any(|c| ('\u{F700}'..='\u{F8FF}').contains(&c))
}

/// What the key monitor does with a key while a field is on its way.
#[derive(Debug, PartialEq, Eq)]
pub enum Hold {
    /// Not ours: the usual path.
    Pass,
    /// Held for the field.
    Held,
    /// The field has the keyboard: what was held goes to it now, then this key.
    Release,
}

/// Decides for one key: `text` (it types), `field` (a text field has the
/// keyboard now), `held` (keys are waiting).
pub fn decide(text: bool, field: bool, held: bool) -> Hold {
    let waiting = expecting();
    if field && (held || waiting) {
        return Hold::Release;
    }
    if waiting && text {
        return Hold::Held;
    }
    Hold::Pass
}

#[cfg(target_os = "macos")]
pub(crate) fn hold(event: objc2::rc::Retained<objc2_app_kit::NSEvent>) {
    HELD.with(|h| h.borrow_mut().push(event));
}

#[cfg(target_os = "macos")]
pub(crate) fn held() -> bool {
    HELD.with(|h| !h.borrow().is_empty())
}

/// The field has the keyboard: the held keys (and `then`) are posted again,
/// in order, and reach it as typing does. The wait is over.
#[cfg(target_os = "macos")]
pub(crate) fn release(then: Option<objc2::rc::Retained<objc2_app_kit::NSEvent>>) {
    EXPECTED.with(|e| e.set(None));
    let keys: Vec<_> = HELD.with(|h| std::mem::take(&mut *h.borrow_mut()));
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
    for key in keys.into_iter().chain(then) {
        app.postEvent_atStart(&key, false);
    }
}

/// The field the person opened has the keyboard (its view saw it take it):
/// anything held for it goes to it now.
#[cfg(target_os = "macos")]
pub fn field_ready() {
    if held() {
        release(None);
    } else {
        EXPECTED.with(|e| e.set(None));
    }
}

/// The wait ran out with keys still held: they were typed for a field that
/// never came, so they go nowhere.
#[cfg(target_os = "macos")]
pub(crate) fn drop_expired() {
    if !expecting() {
        HELD.with(|h| h.borrow_mut().clear());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_wait_for_the_field_the_person_opened() {
        assert!(is_text("p", false));
        assert!(!is_text("p", true), "a shortcut with Command or Control");
        assert!(!is_text("\u{1b}", false), "Escape");
        assert!(!is_text("\u{F700}", false), "an arrow key");
        assert_eq!(decide(true, false, false), Hold::Pass, "nothing opened");
        expect();
        assert_eq!(decide(true, false, false), Hold::Held, "on its way: held");
        assert_eq!(decide(false, false, false), Hold::Pass, "Escape and arrows pass");
        assert_eq!(decide(true, true, true), Hold::Release, "the field has it");
        EXPECTED.with(|e| e.set(Some(Instant::now() - Duration::from_millis(1))));
        assert_eq!(decide(true, false, false), Hold::Pass, "the wait ran out");
    }
}
