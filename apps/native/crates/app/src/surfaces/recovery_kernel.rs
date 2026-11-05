//! Typed kernel boundary. No document mutation or persistence in the surface.
//! The app session controller must call this adapter on its worker completions;
//! existing session.rs is intentionally unchanged under this lane's ownership.
use super::recovery::{Action, RecoveryState, RecoverySurface, recovery_surface};
use pm_kernel::session::{RecoveryDecision, RecoveryResult, ShellCall, ShellReply};

pub fn state(result: &RecoveryResult) -> RecoveryState {
    match result {
        RecoveryResult::None => RecoveryState::None,
        RecoveryResult::Recovered { truncated } => RecoveryState::Recovered {
            truncated: *truncated,
        },
        RecoveryResult::OfferCopy { truncated, .. } => RecoveryState::OfferCopy {
            truncated: *truncated,
        },
        // Developer reason intentionally never enters surface copy.
        RecoveryResult::Unrecoverable { .. } => RecoveryState::Unrecoverable,
    }
}
#[derive(Clone, Debug)]
pub struct PreparedRecovery {
    ticket: String,
    pub surface: RecoverySurface,
    archive_recently_deleted: bool,
}
impl PreparedRecovery {
    /// Archive is false until the kernel/library exposes a confirmed Recently
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
        Some(Self {
            ticket: ticket.clone(),
            surface: recovery_surface(
                name,
                state(recovery),
                true,
                archive_recently_deleted,
                saved_exists,
            )?,
            archive_recently_deleted,
        })
    }
    pub fn ticket(&self) -> &str {
        &self.ticket
    }
    /// Called only after AlertQueue has validated the owner/id/revision/action.
    /// The returned call is submitted with KernelHandle::shell (never .wait()).
    pub fn decision(&self, action: Action) -> Option<ShellCall> {
        let RecoverySurface::Alert(alert) = &self.surface else {
            return None;
        };
        if !alert.buttons.iter().any(|b| b.action == action) {
            return None;
        }
        let decision = match action {
            Action::Recover => RecoveryDecision::Recover,
            Action::OpenAsCopy => RecoveryDecision::OpenAsCopy,
            Action::Discard if self.archive_recently_deleted => RecoveryDecision::Discard,
            // Open Saved Version needs a kernel open-without-replay policy that
            // keeps the unreadable journal. It must never be mapped to Discard.
            _ => return None,
        };
        Some(ShellCall::DecideRecovery {
            ticket: self.ticket.clone(),
            decision,
        })
    }
}
pub fn opened_surface(reply: &ShellReply, name: &str) -> Option<RecoverySurface> {
    let ShellReply::Opened { recovery, .. } = reply else {
        return None;
    };
    recovery_surface(name, state(recovery), false, false, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_kernel_reply_uses_ticket_and_exact_decisions() {
        let reply = ShellReply::RecoveryRequired {
            ticket: "synthetic-ticket".into(),
            recovery: RecoveryResult::Recovered { truncated: true },
        };
        let p = PreparedRecovery::from_reply(&reply, "Synthetic", false, true).unwrap();
        for (action, expected) in [
            (Action::Recover, RecoveryDecision::Recover),
            (Action::OpenAsCopy, RecoveryDecision::OpenAsCopy),
        ] {
            let Some(ShellCall::DecideRecovery { ticket, decision }) = p.decision(action) else {
                panic!("missing decision");
            };
            assert_eq!(ticket, "synthetic-ticket");
            assert_eq!(decision, expected);
        }
        assert!(p.decision(Action::Discard).is_none());
        let p = PreparedRecovery::from_reply(&reply, "Synthetic", true, true).unwrap();
        assert!(matches!(
            p.decision(Action::Discard),
            Some(ShellCall::DecideRecovery {
                decision: RecoveryDecision::Discard,
                ..
            })
        ));
    }
    #[test]
    fn all_kernel_results_map_without_leaking_diagnostics() {
        for result in [
            RecoveryResult::None,
            RecoveryResult::Recovered { truncated: false },
            RecoveryResult::Recovered { truncated: true },
            RecoveryResult::OfferCopy {
                project: "synthetic".into(),
                truncated: true,
            },
            RecoveryResult::Unrecoverable {
                reason: "PRIVATE DIAGNOSTIC SENTINEL".into(),
            },
        ] {
            assert!(!format!("{:?}", state(&result)).contains("SENTINEL"));
        }
        let p = PreparedRecovery::from_reply(
            &ShellReply::RecoveryRequired {
                ticket: "ticket".into(),
                recovery: RecoveryResult::Unrecoverable {
                    reason: "sentinel".into(),
                },
            },
            "Synthetic",
            true,
            true,
        )
        .unwrap();
        assert!(p.decision(Action::OpenSavedVersion).is_none());
        assert!(p.decision(Action::Discard).is_none());
        assert!(p.decision(Action::Recover).is_none());
    }
}
