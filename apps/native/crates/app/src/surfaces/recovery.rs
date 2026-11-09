//! S001 recovery alert, design/surfaces/recovery.md (2026-11-01): the kernel's
//! RecoveryRequired reply as an NSAlert sheet, and the answer as its decision.
//! Discard requires a confirmed Recently Deleted archive capability (N4).
use crate::alert::{Alert, Button};
use pm_kernel::session::{RecoveryDecision, RecoveryResult, ShellCall, ShellReply};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Recover,
    OpenAsCopy,
    Discard,
    OpenSavedVersion,
    Acknowledge,
}
pub fn ask_recovery(
    name: &str,
    truncated: bool,
    can_archive_recently_deleted: bool,
) -> Alert<Action> {
    let mut buttons = vec![
        Button::new("Recover", Action::Recover).default(),
        Button::new("Open as Copy", Action::OpenAsCopy).cancel(),
    ];
    if can_archive_recently_deleted {
        buttons.push(Button::new("Discard Edits", Action::Discard).destructive());
    }
    Alert::new(
        format!("Powermove quit unexpectedly while “{name}” had unsaved changes."),
        format!(
            "Recover opens your latest edits. Open as Copy keeps the saved project unchanged and opens the edits beside it.{}",
            if truncated {
                " The last few seconds of edits may be missing."
            } else {
                ""
            }
        ),
        buttons,
    )
}
pub fn unrecoverable(name: &str, saved_file_exists: bool) -> Alert<Action> {
    let mut buttons = Vec::new();
    if saved_file_exists {
        buttons.push(Button::new("Open Saved Version", Action::OpenSavedVersion).default());
    }
    let ok = Button::new("OK", Action::Acknowledge);
    buttons.push(if saved_file_exists { ok } else { ok.default() });
    Alert::new(
        format!("“{name}” couldn’t be recovered."),
        "The saved project is unchanged. Details are in Window ▸ Error Log.",
        buttons,
    )
}

/// A RecoveryRequired reply's alert, and the ticket its answer decides.
#[derive(Clone, Debug)]
pub struct PreparedRecovery {
    ticket: String,
    pub alert: Alert<Action>,
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

#[cfg(test)]
#[path = "recovery_tests.rs"]
mod tests;
