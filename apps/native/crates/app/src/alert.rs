//! Every NSAlert the app shows: its copy, its buttons and their keys, and the
//! sheet that answers with the chosen button's action. Closing or aborting a
//! sheet answers `None`, so a dismissed alert never chooses anything.
use objc2::{MainThreadMarker, Message, rc::Retained};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAlertStyle, NSButtonCell, NSEventModifierFlags,
    NSModalResponse, NSWindow,
};
use objc2_foundation::NSString;

/// The key that presses a button, besides Return on the default one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    None,
    Escape,
    CommandD,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button<A> {
    pub label: &'static str,
    pub action: A,
    /// Return presses it.
    pub default: bool,
    pub key: Key,
    pub destructive: bool,
}
impl<A> Button<A> {
    pub fn new(label: &'static str, action: A) -> Self {
        Self {
            label,
            action,
            default: false,
            key: Key::None,
            destructive: false,
        }
    }
    pub fn default(mut self) -> Self {
        self.default = true;
        self
    }
    pub fn cancel(mut self) -> Self {
        self.key = Key::Escape;
        self
    }
    pub fn key(mut self, key: Key) -> Self {
        self.key = key;
        self
    }
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert<A> {
    pub message: String,
    pub informative: String,
    /// NSAlert order: the first is rightmost.
    pub buttons: Vec<Button<A>>,
    pub critical: bool,
    /// Asks nothing (OK, Show in Finder): closing its window or quitting
    /// dismisses it instead of being refused (`dismiss_notice`).
    pub notice: bool,
}
impl<A> Alert<A> {
    pub fn new(
        message: impl Into<String>,
        informative: impl Into<String>,
        buttons: Vec<Button<A>>,
    ) -> Self {
        Self {
            message: message.into(),
            informative: informative.into(),
            buttons,
            critical: false,
            notice: false,
        }
    }
    pub fn notice(mut self) -> Self {
        self.notice = true;
        self
    }
    pub fn critical(mut self) -> Self {
        self.critical = true;
        self
    }
    #[cfg(test)]
    pub fn labels(&self) -> Vec<&'static str> {
        self.buttons.iter().map(|b| b.label).collect()
    }
}
impl<A: Clone + 'static> Alert<A> {
    /// Shows the alert as a sheet on `window`. Main thread only.
    pub fn sheet(&self, window: &NSWindow) -> async_channel::Receiver<Option<A>> {
        self.sheet_with(window, |_| {})
    }
    /// As `sheet`, after `configure` adds to the alert (an accessory view).
    pub fn sheet_with(
        &self,
        window: &NSWindow,
        configure: impl FnOnce(&NSAlert),
    ) -> async_channel::Receiver<Option<A>> {
        let mtm = MainThreadMarker::new().expect("NSAlert must be presented on main");
        let native = NSAlert::new(mtm);
        native.setMessageText(&NSString::from_str(&self.message));
        native.setInformativeText(&NSString::from_str(&self.informative));
        if self.critical {
            native.setAlertStyle(NSAlertStyle::Critical);
        }
        let mut default_cell = None;
        for b in &self.buttons {
            let button = native.addButtonWithTitle(&NSString::from_str(b.label));
            // Explicit for every button: NSAlert gives the first one Return.
            let (key, modifiers) = match b.key {
                Key::Escape => ("\u{1b}", NSEventModifierFlags::empty()),
                Key::CommandD => ("d", NSEventModifierFlags::Command),
                Key::None if b.default => ("\r", NSEventModifierFlags::empty()),
                Key::None => ("", NSEventModifierFlags::empty()),
            };
            button.setKeyEquivalent(&NSString::from_str(key));
            button.setKeyEquivalentModifierMask(modifiers);
            button.setHasDestructiveAction(b.destructive);
            // A default with its own key (a Cancel that is the default) takes
            // Return as the window's default cell.
            if b.default && b.key != Key::None {
                default_cell = button
                    .cell()
                    .and_then(|cell| cell.downcast::<NSButtonCell>().ok());
            }
        }
        if default_cell.is_some() {
            native
                .window()
                .setDefaultButtonCell(default_cell.as_deref());
        }
        configure(&native);
        let (tx, rx) = async_channel::bounded(1);
        let actions: Vec<A> = self.buttons.iter().map(|b| b.action.clone()).collect();
        let retained: Retained<NSAlert> = native.clone();
        let sheet = native.window();
        if self.notice {
            NOTICES.with(|n| n.borrow_mut().push((sheet.clone(), window.retain())));
        }
        let done = block2::RcBlock::new(move |response: NSModalResponse| {
            let _keep_alive = &retained;
            NOTICES.with(|n| n.borrow_mut().retain(|(w, _)| !std::ptr::eq(&**w, &*sheet)));
            let _ = tx.try_send(chosen(&actions, response));
        });
        // Only while the app is active (a sheet orders its window front).
        let (on, shown) = (window.retain(), native.clone());
        gpui_macos::child_window::sheet(window, &native.window(), move || {
            shown.beginSheetModalForWindow_completionHandler(&on, Some(&done));
        });
        rx
    }
}

thread_local! {
    /// Sheet windows of alerts that ask nothing (`Alert::notice`), and the
    /// windows they are for.
    static NOTICES: std::cell::RefCell<Vec<(Retained<NSWindow>, Retained<NSWindow>)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Ends `window`'s sheet when it is an alert that asks nothing, as if
/// dismissed; returns whether the window is now free of sheets.
pub fn dismiss_notice(window: &NSWindow) -> bool {
    // A notice on it still waiting for the app to become active never shows.
    let waiting: Vec<_> = NOTICES.with(|n| {
        let mut n = n.borrow_mut();
        let (on, other) = std::mem::take(&mut *n)
            .into_iter()
            .partition(|(sheet, parent)| {
                std::ptr::eq(&**parent, window) && sheet.sheetParent().is_none()
            });
        *n = other;
        on
    });
    for (notice, _) in &waiting {
        gpui_macos::child_window::forget_sheet(notice);
    }
    let Some(sheet) = window.attachedSheet() else {
        return !gpui_macos::child_window::sheet_pending(window);
    };
    let notice = NOTICES.with(|n| n.borrow().iter().any(|(w, _)| std::ptr::eq(&**w, &*sheet)));
    if notice {
        window.endSheet_returnCode(&sheet, objc2_app_kit::NSModalResponseAbort);
    }
    window.attachedSheet().is_none()
}

/// A one-button alert as a sheet on the key window.
pub fn note(message: &str, informative: &str) {
    let mtm = MainThreadMarker::new().unwrap();
    let Some(window) = objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow() else {
        return;
    };
    let _ = Alert::new(message, informative, vec![Button::new("OK", ()).default()])
        .notice()
        .sheet(&window);
}

fn chosen<A: Clone>(actions: &[A], response: NSModalResponse) -> Option<A> {
    let index = usize::try_from(response - NSAlertFirstButtonReturn).ok()?;
    actions.get(index).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2_app_kit::{
        NSAlertSecondButtonReturn, NSAlertThirdButtonReturn, NSModalResponseAbort,
        NSModalResponseCancel,
    };
    #[test]
    fn buttons_keep_their_order_and_dismissed_sheets_choose_nothing() {
        let actions = ["save", "cancel", "discard"];
        assert_eq!(chosen(&actions, NSAlertFirstButtonReturn), Some("save"));
        assert_eq!(chosen(&actions, NSAlertSecondButtonReturn), Some("cancel"));
        assert_eq!(chosen(&actions, NSAlertThirdButtonReturn), Some("discard"));
        assert_eq!(chosen(&actions, NSModalResponseAbort), None);
        assert_eq!(chosen(&actions, NSModalResponseCancel), None);
        assert_eq!(chosen(&actions[..2], NSAlertThirdButtonReturn), None);
    }
}
