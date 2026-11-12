use super::*;

use crate::alert::Key;

fn labels(a: &Alert<Action>) -> Vec<&str> {
    a.labels()
}
fn on_return(a: &Alert<Action>) -> Option<Action> {
    a.buttons.iter().find(|b| b.default).map(|b| b.action)
}
fn on_escape(a: &Alert<Action>) -> Option<Action> {
    a.buttons
        .iter()
        .find(|b| b.key == Key::Escape)
        .map(|b| b.action)
}
#[test]
fn ask_exact_copy_and_safe_keyboard() {
    for truncated in [false, true] {
        let a = ask_recovery("Launch Film", truncated, true);
        assert_eq!(
            a.message,
            "Powermove quit unexpectedly while “Launch Film” had unsaved changes."
        );
        assert_eq!(
            a.informative,
            format!(
                "Recover opens your latest edits. Open as Copy keeps the saved project unchanged and opens the edits beside it.{}",
                if truncated {
                    " The last few seconds of edits may be missing."
                } else {
                    ""
                }
            )
        );
        assert_eq!(labels(&a), ["Recover", "Open as Copy", "Discard Edits"]);
        assert_eq!(on_escape(&a), Some(Action::OpenAsCopy));
        assert_eq!(on_return(&a), Some(Action::Recover));
        assert!(a.buttons[2].destructive);
        assert!(!a.buttons[2].default);
    }
}
#[test]
fn unarchivable_edits_never_offer_discard() {
    assert_eq!(
        labels(&ask_recovery("Synthetic", false, false)),
        ["Recover", "Open as Copy"]
    );
}
#[test]
fn unrecoverable_preserves_saved_file_and_omits_missing_file_action() {
    let a = unrecoverable("Launch Film", true);
    assert_eq!(a.message, "“Launch Film” couldn’t be recovered.");
    assert_eq!(
        a.informative,
        "The saved project is unchanged. Details are in Window ▸ Error Log."
    );
    assert_eq!(labels(&a), ["Open Saved Version", "OK"]);
    assert_eq!(on_return(&a), Some(Action::OpenSavedVersion));
    let a = unrecoverable("Launch Film", false);
    assert_eq!(labels(&a), ["OK"]);
    assert_eq!(on_return(&a), Some(Action::Acknowledge));
}
#[test]
fn kernel_reply_uses_ticket_and_exact_decisions() {
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
    let none = ShellReply::RecoveryRequired {
        ticket: "t".into(),
        recovery: RecoveryResult::None,
    };
    assert!(PreparedRecovery::from_reply(&none, "Synthetic", true, true).is_none());
}
#[test]
fn another_files_work_is_offered_by_name_and_left_alone_when_declined() {
    let reply = ShellReply::RecoveryRequired {
        ticket: "ticket".into(),
        recovery: RecoveryResult::FromOtherFile {
            name: "cut.pmv".into(),
            truncated: false,
        },
    };
    let p = PreparedRecovery::from_reply(&reply, "cut restored", true, true).unwrap();
    assert_eq!(
        p.alert.message,
        "Recover unsaved changes from “cut.pmv” into “cut restored”?"
    );
    assert!(p.alert.informative.contains("they stay with “cut.pmv”"));
    assert_eq!(labels(&p.alert), ["Recover", "Leave Them"]);
    assert_eq!(on_return(&p.alert), Some(Action::Recover));
    assert!(matches!(
        p.decision(Action::Recover),
        Some(ShellCall::DecideRecovery {
            decision: RecoveryDecision::Recover,
            ..
        })
    ));
    assert!(matches!(
        p.decision(Action::Leave),
        Some(ShellCall::DecideRecovery {
            decision: RecoveryDecision::Discard,
            ..
        })
    ));
    // Nothing destructive, and no copy, is offered for work that may not be
    // this file's.
    assert!(p.decision(Action::Discard).is_none());
    assert!(p.decision(Action::OpenAsCopy).is_none());
}
#[test]
fn unrecoverable_reply_never_leaks_diagnostics_or_discards() {
    let p = PreparedRecovery::from_reply(
        &ShellReply::RecoveryRequired {
            ticket: "ticket".into(),
            recovery: RecoveryResult::Unrecoverable {
                reason: "PRIVATE DIAGNOSTIC SENTINEL".into(),
            },
        },
        "Synthetic",
        true,
        true,
    )
    .unwrap();
    assert!(!format!("{:?}", p.alert).contains("SENTINEL"));
    assert!(matches!(
        p.decision(Action::OpenSavedVersion),
        Some(ShellCall::DecideRecovery {
            decision: RecoveryDecision::OpenSavedVersion,
            ..
        })
    ));
    assert!(p.decision(Action::Discard).is_none());
    assert!(p.decision(Action::Recover).is_none());
}
#[test]
fn reopening_after_a_stop_says_so_and_keeps_the_choices() {
    let a = after_stop(ask_recovery("Launch Film", false, false), "Launch Film");
    assert_eq!(
        a.message,
        "“Launch Film” stopped after an internal error with unsaved changes."
    );
    assert_eq!(labels(&a), ["Recover", "Open as Copy"]);
    assert_eq!(on_return(&a), Some(Action::Recover));
}
