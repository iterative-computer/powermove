//! Every NSAlert the app shows: its copy, its buttons and their keys, and the
//! sheet that answers with the chosen button's action. Closing or aborting a
//! sheet answers `None`, so a dismissed alert never chooses anything.
use objc2::{MainThreadMarker, rc::Retained};
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
    pub label: std::borrow::Cow<'static, str>,
    pub action: A,
    /// Return presses it.
    pub default: bool,
    pub key: Key,
    pub destructive: bool,
}
impl<A> Button<A> {
    pub fn new(label: impl Into<std::borrow::Cow<'static, str>>, action: A) -> Self {
        Self {
            label: label.into(),
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
    pub fn labels(&self) -> Vec<&str> {
        self.buttons.iter().map(|b| b.label.as_ref()).collect()
    }
}
impl<A: Clone + 'static> Alert<A> {
    /// Shows the alert as a sheet on `window`. Main thread only.
    pub fn sheet(&self, window: &NSWindow) -> async_channel::Receiver<Option<A>> {
        self.sheet_with(window, |_| {})
    }
    /// A background event can ask only after its editor is in front.
    pub fn background_sheet(
        self,
        window: &NSWindow,
        cx: &gpui::App,
    ) -> async_channel::Receiver<Option<A>> {
        if can_attach_background_sheet(window) || gpui_macos::child_window::would_hold() {
            return self.sheet(window);
        }
        let window = objc2::rc::Weak::from(window);
        let (tx, rx) = async_channel::bounded(1);
        cx.spawn(async move |cx| {
            while !tx.is_closed() {
                let Some(window) = window.load() else { break };
                if can_attach_background_sheet(&window) {
                    let answer = self.sheet(&window);
                    drop(window);
                    let answer = answer.recv().await.ok().flatten();
                    let _ = tx.try_send(answer);
                    break;
                }
                drop(window);
                cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
            }
        }).detach();
        rx
    }
    /// As `sheet`, after `configure` adds to the alert (an accessory view).
    pub fn sheet_with(
        &self,
        window: &NSWindow,
        configure: impl FnOnce(&NSAlert),
    ) -> async_channel::Receiver<Option<A>> {
        // A development lane launch in the background: a question nobody can
        // see takes its Escape choice at once (none: dismissed), so nothing,
        // a quit least of all, waits on it. A normal launch always asks.
        let buttons = keyed(&self.buttons);
        if gpui_macos::child_window::would_hold() {
            let (tx, rx) = async_channel::bounded(1);
            let escape = buttons.iter().find(|b| b.key == Key::Escape);
            let _ = tx.try_send(escape.map(|b| b.action.clone()));
            eprintln!("powermove: a question in the background answered itself (lane launch)");
            return rx;
        }
        let mtm = MainThreadMarker::new().expect("NSAlert must be presented on main");
        let native = NSAlert::new(mtm);
        native.setMessageText(&NSString::from_str(&self.message));
        native.setInformativeText(&NSString::from_str(&self.informative));
        if self.critical {
            native.setAlertStyle(NSAlertStyle::Critical);
        }
        let mut default_cell = None;
        for b in &buttons {
            let button = native.addButtonWithTitle(&NSString::from_str(&b.label));
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
        let actions: Vec<A> = buttons.iter().map(|b| b.action.clone()).collect();
        let retained: Retained<NSAlert> = native.clone();
        let sheet = native.window();
        if self.notice {
            NOTICES.with(|n| n.borrow_mut().push(sheet.clone()));
        }
        let done = block2::RcBlock::new(move |response: NSModalResponse| {
            let _keep_alive = &retained;
            NOTICES.with(|n| n.borrow_mut().retain(|w| !std::ptr::eq(&**w, &*sheet)));
            let _ = tx.try_send(chosen(&actions, response));
        });
        native.beginSheetModalForWindow_completionHandler(window, Some(&done));
        rx
    }
}

/// AppKit orders the sheet's parent forward. The editor must already be
/// key, visible and the selected project tab before a background sheet starts.
pub fn can_attach_background_sheet(window: &NSWindow) -> bool {
    let app_active = MainThreadMarker::new().is_some_and(|mtm| {
        objc2_app_kit::NSApplication::sharedApplication(mtm).isActive()
    });
    background_sheet_allowed(
        app_active,
        window.isKeyWindow(),
        pm_shell::window_shown(window),
        window.attachedSheet().is_some(),
    )
}

fn background_sheet_allowed(app_active: bool, key: bool, shown: bool, attached: bool) -> bool {
    app_active && key && shown && !attached
}

#[cfg(test)]
#[test]
fn window_jumps_only_the_front_window_and_selected_tab_can_show_a_sheet() {
    assert!(background_sheet_allowed(true, true, true, false));
    assert!(!background_sheet_allowed(false, true, true, false));
    assert!(!background_sheet_allowed(true, false, true, false));
    assert!(!background_sheet_allowed(true, true, false, false));
    assert!(!background_sheet_allowed(true, true, true, true));
}

thread_local! {
    /// Sheet windows of alerts that ask nothing (`Alert::notice`).
    static NOTICES: std::cell::RefCell<Vec<Retained<objc2_app_kit::NSWindow>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Ends `window`'s sheet when it is an alert that asks nothing, as if
/// dismissed; returns whether the window is now free of sheets.
pub fn dismiss_notice(window: &NSWindow) -> bool {
    let Some(sheet) = window.attachedSheet() else {
        return true;
    };
    let notice = NOTICES.with(|n| n.borrow().iter().any(|w| std::ptr::eq(&**w, &*sheet)));
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

/// The buttons with their keys as every alert answers them (HIG): Return
/// presses the default (the first when none is marked), and Escape the choice
/// that changes nothing: the button marked so, else a lone button, else one
/// titled Cancel (NSAlert's own rule, which setting every key would lose).
fn keyed<A: Clone>(buttons: &[Button<A>]) -> Vec<Button<A>> {
    let mut out = buttons.to_vec();
    if !out.iter().any(|b| b.default)
        && let Some(first) = out.first_mut()
    {
        first.default = true;
    }
    if !out.iter().any(|b| b.key == Key::Escape) {
        let lone = out.len() == 1;
        if let Some(b) = out
            .iter_mut()
            .find(|b| b.key == Key::None && (lone || b.label == "Cancel"))
        {
            b.key = Key::Escape;
        }
    }
    out
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
    /// E3: every alert answers Return and Escape.
    #[test]
    fn every_alert_answers_return_and_escape() {
        fn escape(b: &[Button<&'static str>]) -> Vec<&'static str> {
            keyed(b)
                .iter()
                .filter(|b| b.key == Key::Escape)
                .map(|b| b.action)
                .collect()
        }
        fn default(b: &[Button<&'static str>]) -> Vec<&'static str> {
            keyed(b)
                .iter()
                .filter(|b| b.default)
                .map(|b| b.action)
                .collect()
        }
        // A lone OK answers both.
        let ok = [Button::new("OK", "ok")];
        assert_eq!((default(&ok), escape(&ok)), (vec!["ok"], vec!["ok"]));
        // A Cancel nobody marked still answers Escape, as NSAlert's would.
        let pair = [Button::new("Quit Anyway", "quit"), Button::new("Cancel", "cancel")];
        assert_eq!((default(&pair), escape(&pair)), (vec!["quit"], vec!["cancel"]));
        // What an alert marks wins; a Cancel that is the default keeps both.
        let marked = [
            Button::new("Cancel", "cancel").default().cancel(),
            Button::new("Delete", "delete").destructive(),
        ];
        assert_eq!((default(&marked), escape(&marked)), (vec!["cancel"], vec!["cancel"]));
        // Never two Escapes or two defaults.
        let both = [Button::new("Later", "later").cancel(), Button::new("Cancel", "c")];
        assert_eq!(escape(&both), vec!["later"]);
    }

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

#[cfg(test)]
#[test]
fn window_jumps_background_watchdog_requests_cannot_attach_a_sheet_directly() {
    let source = include_str!("extensions.rs");
    let watchdog = source
        .split_once("fn extension_request(")
        .unwrap()
        .1
        .split_once("Some(\"unresponsive\") =>")
        .unwrap()
        .1
        .split_once("Some(\"import\") =>")
        .unwrap()
        .0;
    assert!(
        !watchdog.contains(".sheet("),
        "a background request must not attach an AppKit sheet directly"
    );
}
