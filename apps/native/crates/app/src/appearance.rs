//! Settings › General › Appearance (v1's `themeMode`): Match System, Light
//! or Dark for the whole app. The choice is the profile's (kept in the
//! kernel's state, so it lasts across launches) and is applied as the app's
//! own NSAppearance: every window, menu and sheet takes it, and each editor
//! rebuilds its scheme and tokens as it does when the Mac's appearance
//! changes (Settings' window and its icon sets with them). Match System
//! clears it, so the app follows the Mac live.
use pm_kernel::state::StateSnapshot;
use std::cell::RefCell;

/// What the profile asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    System,
    Light,
    Dark,
}

impl Mode {
    /// The profile's choice; anything else (unset, unknown) follows the Mac,
    /// as v1 did.
    pub fn of(state: &StateSnapshot) -> Mode {
        match state.settings.get("themeMode").and_then(pm_model::Value::as_str) {
            Some("light") => Mode::Light,
            Some("dark") => Mode::Dark,
            _ => Mode::System,
        }
    }
}

thread_local! {
    static APPLIED: RefCell<Option<Mode>> = const { RefCell::new(None) };
}

/// Applies the profile's appearance if it changed. Called at launch and on
/// every state change; a development override (`--appearance`) wins.
pub fn follow(state: &StateSnapshot) {
    if std::env::var_os("PM_NATIVE_APPEARANCE").is_some() {
        return;
    }
    let mode = Mode::of(state);
    if APPLIED.with(|a| a.replace(Some(mode))) == Some(mode) {
        return;
    }
    apply_later(mode);
}

/// Sets the app's appearance on the main run loop's next turn: windows tell
/// GPUI of their new appearance as they change, and GPUI can only take that
/// while no app code is running (as with the Mac's own switch).
pub fn apply_later(mode: Mode) {
    use objc2_foundation::NSRunLoop;
    let block = block2::RcBlock::new(move || apply_now(mode));
    let run_loop = NSRunLoop::mainRunLoop();
    unsafe {
        let _: () = objc2::msg_send![&run_loop, performBlock: &*block];
    }
}

fn apply_now(mode: Mode) {
    use objc2_app_kit::{
        NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
    };
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    let name = match mode {
        Mode::System => None,
        Mode::Light => Some(unsafe { NSAppearanceNameAqua }),
        Mode::Dark => Some(unsafe { NSAppearanceNameDarkAqua }),
    };
    let appearance = name.and_then(NSAppearance::appearanceNamed);
    NSApplication::sharedApplication(mtm).setAppearance(appearance.as_deref());
}

#[cfg(test)]
mod tests {
    use super::Mode;

    #[test]
    fn the_profiles_choice_and_match_system_otherwise() {
        let mut state = pm_kernel::state::StateSnapshot::default();
        assert_eq!(Mode::of(&state), Mode::System);
        for (value, mode) in [("light", Mode::Light), ("dark", Mode::Dark), ("system", Mode::System), ("sepia", Mode::System)] {
            std::sync::Arc::make_mut(&mut state.settings)
                .insert("themeMode".into(), pm_model::Value::from(value));
            assert_eq!(Mode::of(&state), mode, "{value}");
        }
    }
}
