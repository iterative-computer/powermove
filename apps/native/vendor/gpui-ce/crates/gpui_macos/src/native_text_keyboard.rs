//! Composer focus rules, shared by the AppKit host and its headless test.
/// What AppKit is handling when it offers a composer the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Asking {
    Press,
    Key(u16),
    Other,
}
/// A composer takes the keyboard for the person only: the host began to
/// edit it (an explicit open, a press on its well), a press on it, or Tab
/// (key code 48) moving to it. Never a window becoming key, a remount, a
/// run ending or a pick elsewhere.
pub(crate) fn composer_takes_keyboard(armed: bool, asking: Option<Asking>) -> bool {
    armed || matches!(asking, Some(Asking::Press) | Some(Asking::Key(48)))
}
#[cfg(test)]
mod composer_keyboard_tests {
    use super::{Asking, composer_takes_keyboard};
    #[test]
    fn a_composer_takes_the_keyboard_only_for_the_person() {
        assert!(composer_takes_keyboard(false, Some(Asking::Press)), "a click into it");
        assert!(composer_takes_keyboard(false, Some(Asking::Key(48))), "Tab to it");
        assert!(composer_takes_keyboard(true, None), "the host began editing it");
        // A window becoming key, a remount or a republish: no event of the
        // person's, or one that was not for it (Space, Return).
        assert!(!composer_takes_keyboard(false, None));
        assert!(!composer_takes_keyboard(false, Some(Asking::Other)));
        assert!(!composer_takes_keyboard(false, Some(Asking::Key(49))), "Space plays");
        assert!(!composer_takes_keyboard(false, Some(Asking::Key(36))));
    }
}
