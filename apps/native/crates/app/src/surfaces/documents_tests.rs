#![allow(unused_imports)]
use super::*;
use pm_kernel::{
    Published,
    locks::{CopyReason, OpenDecision},
    session::{SessionHandle, ShellCall, ShellReply},
};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    time::Duration,
};
fn handle(id: &str) -> SessionHandle {
    SessionHandle {
        project: id.into(),
        generation: format!("generation-{id}"),
    }
}
#[test]
fn service_error_table_is_exact_and_diagnostics_never_become_copy() {
    for (code, message, info) in [
        (
            "offline",
            "Powermove can’t connect.",
            "Check your internet connection and try again.",
        ),
        (
            "not_found",
            "“Film” can’t be found.",
            "It may have been moved or deleted.",
        ),
        ("too_large", "“Film” is too large to open.", ""),
        (
            "invalid_project",
            "“Film” can’t be opened because it isn’t a Powermove project.",
            "",
        ),
        (
            "permission_denied",
            "You don’t have permission to open “Film”.",
            "Check its sharing and permissions in Finder.",
        ),
        (
            "disk_full",
            "There isn’t enough disk space.",
            "Free some space and try again.",
        ),
        (
            "invalid_input",
            "“Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            "invalid_data",
            "“Film” couldn’t be read.",
            "The file may be damaged.",
        ),
        (
            "unavailable",
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            "unknown",
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
        (
            "diagnostic / private/path",
            "The operation couldn’t be completed.",
            "Try again. If it keeps happening, choose Window ▸ Error Log for details.",
        ),
    ] {
        assert_eq!(service_error(code, "Film"), (message.into(), info));
    }
}
fn edit(k: &pm_kernel::KernelHandle) {
    k.call_blocking(
        "a",
        pm_kernel::Request::new(
            pm_kernel::Caller::native(),
            pm_kernel::Call::Transact {
                label: Some("Rename".into()),
                body: pm_kernel::WriteBody::Commands(vec![
                    pm_kernel::CompCommand::Set {
                        patch: Box::new(pm_kernel::commands::comps::CompPatch {
                            name: Some("Edited Film".into()),
                            ..Default::default()
                        }),
                    }
                    .into(),
                ]),
                expected_revision: None,
            },
        ),
    )
    .unwrap();
}
