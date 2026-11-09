//! S001 recovery alert, design/surfaces/recovery.md (2026-11-01): the kernel's
//! RecoveryRequired reply as an NSAlert sheet, and the answer as its decision.
//! Discard requires a confirmed Recently Deleted archive capability (N4).
use objc2::{MainThreadMarker, rc::Retained};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSEventModifierFlags, NSModalResponse, NSWindow,
};
use objc2_foundation::NSString;
use pm_kernel::session::{RecoveryDecision, RecoveryResult, ShellCall, ShellReply};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Recover,
    OpenAsCopy,
    Discard,
    OpenSavedVersion,
    Acknowledge,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Button {
    pub label: &'static str,
    pub action: Action,
    pub default: bool,
    pub escape: bool,
    pub destructive: bool,
}
impl Button {
    fn new(label: &'static str, action: Action) -> Self {
        Self {
            label,
            action,
            default: false,
            escape: false,
            destructive: false,
        }
    }
    fn default(mut self) -> Self {
        self.default = true;
        self
    }
    fn escape(mut self) -> Self {
        self.escape = true;
        self
    }
    fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub message: String,
    pub informative: String,
    /// Listed order is NSAlert order; first button is rightmost.
    pub buttons: Vec<Button>,
}
pub fn ask_recovery(name: &str, truncated: bool, can_archive_recently_deleted: bool) -> Alert {
    let mut buttons = vec![
        Button::new("Recover", Action::Recover).default(),
        Button::new("Open as Copy", Action::OpenAsCopy).escape(),
    ];
    if can_archive_recently_deleted {
        buttons.push(Button::new("Discard Edits", Action::Discard).destructive());
    }
    Alert {
        message: format!("Powermove quit unexpectedly while “{name}” had unsaved changes."),
        informative: format!(
            "Recover opens your latest edits. Open as Copy keeps the saved project unchanged and opens the edits beside it.{}",
            if truncated {
                " The last few seconds of edits may be missing."
            } else {
                ""
            }
        ),
        buttons,
    }
}
pub fn unrecoverable(name: &str, saved_file_exists: bool) -> Alert {
    let mut buttons = Vec::new();
    if saved_file_exists {
        buttons.push(Button::new("Open Saved Version", Action::OpenSavedVersion).default());
    }
    let ok = Button::new("OK", Action::Acknowledge);
    buttons.push(if saved_file_exists { ok } else { ok.default() });
    Alert {
        message: format!("“{name}” couldn’t be recovered."),
        informative: "The saved project is unchanged. Details are in Window ▸ Error Log.".into(),
        buttons,
    }
}

/// A RecoveryRequired reply's alert, and the ticket its answer decides.
#[derive(Clone, Debug)]
pub struct PreparedRecovery {
    ticket: String,
    pub alert: Alert,
    archive_recently_deleted: bool,
}
impl PreparedRecovery {
    /// Archive stays false until the kernel/library exposes a confirmed Recently
    /// Deleted destination; archive_session alone is insufficient (N4).
    pub fn from_reply(
        reply: &ShellReply,
        name: &str,
        archive_recently_deleted: bool,
        saved_exists: bool,
    ) -> Option<Self> {
        let ShellReply::RecoveryRequired { ticket, recovery } = reply else {
            return None;
        };
        // The developer reason of an unrecoverable journal never enters the copy.
        let alert = match recovery {
            RecoveryResult::None => return None,
            RecoveryResult::Recovered { truncated }
            | RecoveryResult::OfferCopy { truncated, .. } => {
                ask_recovery(name, *truncated, archive_recently_deleted)
            }
            RecoveryResult::Unrecoverable { .. } => unrecoverable(name, saved_exists),
        };
        Some(Self {
            ticket: ticket.clone(),
            alert,
            archive_recently_deleted,
        })
    }
    /// The call to submit with KernelHandle::shell (never .wait()), for an
    /// action this alert offered.
    pub fn decision(&self, action: Action) -> Option<ShellCall> {
        if !self.alert.buttons.iter().any(|b| b.action == action) {
            return None;
        }
        let decision = match action {
            Action::Recover => RecoveryDecision::Recover,
            Action::OpenAsCopy => RecoveryDecision::OpenAsCopy,
            Action::Discard if self.archive_recently_deleted => RecoveryDecision::Discard,
            // Never Discard: this one keeps the unreadable journal, set aside.
            Action::OpenSavedVersion => RecoveryDecision::OpenSavedVersion,
            _ => return None,
        };
        Some(ShellCall::DecideRecovery {
            ticket: self.ticket.clone(),
            decision,
        })
    }
}

/// The alert as a sheet on `window`; the receiver gets the chosen action.
pub fn sheet(alert: &Alert, window: &NSWindow) -> async_channel::Receiver<Option<Action>> {
    let mtm = MainThreadMarker::new().expect("NSAlert must be presented on main");
    let native = NSAlert::new(mtm);
    native.setMessageText(&NSString::from_str(&alert.message));
    native.setInformativeText(&NSString::from_str(&alert.informative));
    for b in &alert.buttons {
        let button = native.addButtonWithTitle(&NSString::from_str(b.label));
        let key = if b.default {
            "\r"
        } else if b.escape {
            "\u{1b}"
        } else {
            ""
        };
        button.setKeyEquivalent(&NSString::from_str(key));
        button.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
        button.setHasDestructiveAction(b.destructive);
    }
    let (tx, rx) = async_channel::bounded(1);
    let buttons: Vec<Action> = alert.buttons.iter().map(|b| b.action).collect();
    let retained: Retained<NSAlert> = native.clone();
    let done = block2::RcBlock::new(move |r: NSModalResponse| {
        let _keep_alive = &retained;
        let action = usize::try_from(r - NSAlertFirstButtonReturn)
            .ok()
            .and_then(|index| buttons.get(index).copied());
        let _ = tx.try_send(action);
    });
    native.beginSheetModalForWindow_completionHandler(window, Some(&done));
    rx
}

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
